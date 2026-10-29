//! Function List: finds functions, methods and their classes with the
//! per-language regex rules in `assets/function_lists.json`.
//!
//! Each language has:
//! - `skip`: comments and string literals, ignored when matching and when
//!   counting braces;
//! - `functions`: patterns with a `name` group (and an optional `class`
//!   group, e.g. `Foo::bar` or Go receivers);
//! - `containers`: class-like blocks with a `name` group. Their body is either
//!   the `{ ... }` block opened by the `{` the pattern ends with (`"braces"`),
//!   or the following lines indented deeper than the header (`"indent"`).
//!   Functions found inside a body get the container's name as their class.

use std::collections::HashMap;

use fancy_regex::Regex;
use serde::Deserialize;

const RULES_JSON: &str = include_str!("../assets/function_lists.json");

#[derive(Deserialize)]
struct RuleSource {
    #[serde(default)]
    skip: Option<String>,
    #[serde(default)]
    functions: Vec<String>,
    #[serde(default)]
    containers: Vec<ContainerSource>,
}

#[derive(Deserialize)]
struct ContainerSource {
    start: String,
    #[serde(default)]
    body: BodyKind,
    /// Patterns used inside the body; defaults to the top-level `functions`.
    #[serde(default)]
    functions: Option<Vec<String>>,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum BodyKind {
    #[default]
    Braces,
    Indent,
}

struct Container {
    start: Regex,
    body: BodyKind,
    functions: Option<Vec<Regex>>,
}

pub struct Parser {
    skip: Option<Regex>,
    functions: Vec<Regex>,
    containers: Vec<Container>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundItem {
    pub name: String,
    pub class: Option<String>,
    /// Byte offset in the (LF-normalised) text.
    pub pos: usize,
    /// 0-based line.
    pub line: usize,
}

fn compile(expr: &str) -> Option<Regex> {
    fancy_regex::RegexBuilder::new(&format!("(?m){expr}"))
        .backtrack_limit(1_000_000)
        .build()
        .ok()
}

fn compile_all(exprs: &[String]) -> Vec<Regex> {
    exprs.iter().filter_map(|e| compile(e)).collect()
}

/// Sorted, non-overlapping byte ranges to ignore.
struct Zones(Vec<(usize, usize)>);

impl Zones {
    fn contains(&self, pos: usize) -> bool {
        let i = self.0.partition_point(|&(_, e)| e <= pos);
        self.0.get(i).is_some_and(|&(s, _)| s <= pos)
    }
}

impl Parser {
    fn from_source(src: &RuleSource) -> Self {
        Self {
            skip: src.skip.as_deref().and_then(compile),
            functions: compile_all(&src.functions),
            containers: src
                .containers
                .iter()
                .filter_map(|c| {
                    Some(Container {
                        start: compile(&c.start)?,
                        body: c.body,
                        functions: c.functions.as_deref().map(compile_all),
                    })
                })
                .collect(),
        }
    }

    fn skip_zones(&self, text: &str) -> Zones {
        let mut out = Vec::new();
        if let Some(re) = &self.skip {
            for m in re.find_iter(text).flatten() {
                if m.end() > m.start() {
                    out.push((m.start(), m.end()));
                }
            }
        }
        Zones(out)
    }

    /// End of the `{ ... }` block whose `{` is at `open`.
    fn brace_end(text: &str, open: usize, zones: &Zones) -> usize {
        let mut depth = 0usize;
        let mut zi = zones.0.partition_point(|&(_, e)| e <= open);
        let bytes = text.as_bytes();
        let mut i = open;
        while i < bytes.len() {
            if let Some(&(s, e)) = zones.0.get(zi)
                && i >= s
            {
                i = e;
                zi += 1;
                continue;
            }
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        text.len()
    }

    /// End of the block of lines indented deeper than the line holding `header`.
    fn indent_end(text: &str, header: usize) -> usize {
        let line_start = text[..header].rfind('\n').map_or(0, |i| i + 1);
        let indent = |line: &str| line.len() - line.trim_start_matches([' ', '\t']).len();
        let base = indent(&text[line_start..]);
        let mut pos = text[header..]
            .find('\n')
            .map_or(text.len(), |i| header + i + 1);
        let mut end = pos;
        while pos < text.len() {
            let next = text[pos..].find('\n').map_or(text.len(), |i| pos + i + 1);
            let line = &text[pos..next];
            if !line.trim().is_empty() {
                if indent(line) <= base {
                    break;
                }
                end = next;
            }
            pos = next;
        }
        end
    }

    fn find_functions(
        patterns: &[Regex],
        text: &str,
        range: (usize, usize),
        class: Option<&str>,
        zones: &Zones,
        out: &mut Vec<(usize, String, Option<String>)>,
    ) {
        let slice = &text[..range.1];
        for re in patterns {
            let mut pos = range.0;
            while pos < range.1 {
                let Ok(Some(caps)) = re.captures_from_pos(slice, pos) else {
                    break;
                };
                let whole = caps.get(0).expect("group 0");
                pos = whole.end().max(whole.start() + 1);
                let Some(name) = caps.name("name") else {
                    continue;
                };
                if zones.contains(name.start()) || name.as_str().trim().is_empty() {
                    continue;
                }
                let cls = class
                    .map(str::to_string)
                    .or_else(|| caps.name("class").map(|c| c.as_str().to_string()));
                out.push((name.start(), name.as_str().to_string(), cls));
            }
        }
    }

    pub fn parse(&self, text: &str) -> Vec<(usize, String, Option<String>)> {
        let zones = self.skip_zones(text);
        let mut out = Vec::new();
        // Container bodies, in order and non-overlapping.
        let mut bodies: Vec<(usize, usize)> = Vec::new();
        for c in &self.containers {
            let mut pos = 0;
            while pos < text.len() {
                let Ok(Some(caps)) = c.start.captures_from_pos(text, pos) else {
                    break;
                };
                let whole = caps.get(0).expect("group 0");
                pos = whole.end().max(whole.start() + 1);
                let Some(name) = caps.name("name") else {
                    continue;
                };
                if zones.contains(name.start())
                    || bodies
                        .iter()
                        .any(|&(s, e)| whole.start() >= s && whole.start() < e)
                {
                    continue;
                }
                let end = match c.body {
                    BodyKind::Braces => match text[..whole.end()].rfind('{') {
                        Some(open) if open >= name.end() => Self::brace_end(text, open, &zones),
                        _ => continue,
                    },
                    BodyKind::Indent => Self::indent_end(text, whole.start()),
                };
                let patterns = c.functions.as_deref().unwrap_or(&self.functions);
                Self::find_functions(
                    patterns,
                    text,
                    (whole.end(), end),
                    Some(name.as_str()),
                    &zones,
                    &mut out,
                );
                bodies.push((whole.start(), end));
                pos = pos.max(end);
            }
        }
        bodies.sort_unstable();
        // Everything outside container bodies.
        let mut start = 0;
        for &(s, e) in bodies.iter().chain([(text.len(), text.len())].iter()) {
            if s > start {
                Self::find_functions(&self.functions, text, (start, s), None, &zones, &mut out);
            }
            start = start.max(e);
        }
        out.sort_by_key(|x| x.0);
        out.dedup_by_key(|x| x.0);
        out
    }
}

pub struct FunctionLists {
    sources: HashMap<String, RuleSource>,
    compiled: HashMap<String, Parser>,
}

impl FunctionLists {
    pub fn new() -> Self {
        let sources = serde_json::from_str(RULES_JSON).unwrap_or_default();
        Self {
            sources,
            compiled: HashMap::new(),
        }
    }

    pub fn has_parser(&self, lang: &str) -> bool {
        self.sources.contains_key(lang)
    }

    /// Parses `text` with the rules for `lang` and returns the found items.
    pub fn parse(&mut self, lang: &str, text: &str) -> Vec<FoundItem> {
        let Some(src) = self.sources.get(lang) else {
            return Vec::new();
        };
        let parser = self
            .compiled
            .entry(lang.to_string())
            .or_insert_with(|| Parser::from_source(src));
        let normalized = if text.contains('\r') {
            text.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            text.to_string()
        };
        let mut line_starts = vec![0usize];
        for (i, b) in normalized.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        parser
            .parse(&normalized)
            .into_iter()
            .map(|(pos, name, class)| {
                let line = line_starts.partition_point(|&s| s <= pos).saturating_sub(1);
                FoundItem {
                    name: name.split_whitespace().collect::<Vec<_>>().join(" "),
                    class,
                    pos,
                    line,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(lang: &str, src: &str) -> Vec<(String, Option<String>, usize)> {
        FunctionLists::new()
            .parse(lang, src)
            .into_iter()
            .map(|i| (i.name, i.class, i.line))
            .collect()
    }

    fn item(name: &str, class: Option<&str>, line: usize) -> (String, Option<String>, usize) {
        (name.to_string(), class.map(str::to_string), line)
    }

    #[test]
    fn every_rule_compiles() {
        let sources: HashMap<String, RuleSource> = serde_json::from_str(RULES_JSON).unwrap();
        assert!(sources.len() >= 25, "only {} languages", sources.len());
        for (lang, src) in &sources {
            let all = src.skip.iter().chain(&src.functions).chain(
                src.containers
                    .iter()
                    .flat_map(|c| std::iter::once(&c.start).chain(c.functions.iter().flatten())),
            );
            for e in all {
                assert!(compile(e).is_some(), "{lang}: bad regex {e}");
            }
        }
    }

    #[test]
    fn python_functions_and_classes() {
        let src = "import os\n\ndef top(a, b):\n    pass\n\nclass Foo(Base):\n    def method(self):\n        pass\n\n# def commented():\nasync def later():\n    pass\n";
        let items = found("python", src);
        assert_eq!(
            items,
            vec![
                item("top", None, 2),
                item("method", Some("Foo"), 6),
                item("later", None, 10),
            ]
        );
    }

    #[test]
    fn cpp_functions() {
        let src = "int add(int a, int b) {\n  return a + b;\n}\n/* void hidden() {} */\nclass W {\n  void run() {\n    if (x) {\n    }\n  }\n};\nvoid W::go() const {\n}\n";
        let items = found("cpp", src);
        assert_eq!(
            items,
            vec![
                item("add", None, 0),
                item("run", Some("W"), 5),
                item("go", Some("W"), 10),
            ]
        );
    }

    #[test]
    fn rust_functions() {
        let src = "fn main() {\n    let s = \"fn fake() {\";\n}\nimpl<T> Display for Foo<T> {\n    pub fn bar(&self) -> u8 {\n        1\n    }\n}\npub(crate) async fn later() {}\n";
        let items = found("rust", src);
        assert_eq!(
            items,
            vec![
                item("main", None, 0),
                item("bar", Some("Foo"), 4),
                item("later", None, 8),
            ]
        );
    }

    #[test]
    fn javascript_functions() {
        let src = "function a() {}\nconst b = async (x) => x;\nexport class C extends D {\n  constructor() {}\n  async run(x) {\n    for (const y of x) {}\n  }\n}\n";
        let items = found("javascript", src);
        assert_eq!(
            items,
            vec![
                item("a", None, 0),
                item("b", None, 1),
                item("constructor", Some("C"), 3),
                item("run", Some("C"), 4),
            ]
        );
    }

    #[test]
    fn go_and_markdown() {
        let src = "func (s *Server) Start() error {\n}\nfunc main() {\n}\n";
        assert_eq!(
            found("go", src),
            vec![item("Start", Some("Server"), 0), item("main", None, 2)]
        );
        let md = "# Title\n\n```\n# not a heading\n```\n## Part\n";
        assert_eq!(
            found("markdown", md),
            vec![item("# Title", None, 0), item("## Part", None, 5)]
        );
    }
}

#[cfg(test)]
mod bench {
    #[test]
    #[ignore = "benchmark: FL_FILE=path FL_LANG=cpp cargo test --release fl_bench -- --ignored --nocapture"]
    fn fl_bench() {
        let path = std::env::var("FL_FILE").expect("FL_FILE");
        let lang = std::env::var("FL_LANG").expect("FL_LANG");
        let text = std::fs::read_to_string(path).unwrap();
        let mut fl = super::FunctionLists::new();
        let t = std::time::Instant::now();
        let items = fl.parse(&lang, &text);
        println!("{} items in {:?}", items.len(), t.elapsed());
        for i in items.iter().take(12) {
            println!("  {:>5} {:?} {}", i.line + 1, i.class, i.name);
        }
    }
}
