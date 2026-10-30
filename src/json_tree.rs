//! JSON tree view: a lenient JSON parser that records where every value
//! starts, so the tree can jump to the source, plus JSON-pointer paths.
//!
//! Nodes are stored flat in document (pre-order) order; a node's children
//! follow it, and `end` is the index just past its last descendant, so a
//! collapsed subtree is skipped in one step.

/// What kind of value a node holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Root,
    Name(String),
    Index(usize),
}

#[derive(Clone, Debug)]
pub struct Node {
    pub parent: Option<u32>,
    pub depth: u16,
    pub key: Key,
    pub kind: Kind,
    /// Short text for the tree: the scalar value, or "{3}" / "[12]".
    pub preview: String,
    /// Byte offset where the value starts in the source.
    pub offset: usize,
    /// Byte offset just past the value.
    pub value_end: usize,
    /// Index just past the last descendant.
    pub end: u32,
    pub children: u32,
}

const PREVIEW_MAX: usize = 80;

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
    nodes: Vec<Node>,
}

impl Parser<'_> {
    fn err<T>(&self, msg: &str) -> Result<T, String> {
        let line = self.s[..self.i.min(self.s.len())]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
            + 1;
        Err(format!("Line {line}: {msg}"))
    }

    /// Skips whitespace and `//` / `/* */` comments.
    fn skip_ws(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if self.s[self.i..].starts_with(b"/*") {
                match self.text[self.i + 2..].find("*/") {
                    Some(e) => self.i += e + 4,
                    None => self.i = self.s.len(),
                }
            } else {
                return;
            }
        }
    }

    /// Parses a string starting at the opening quote; returns its value.
    fn string(&mut self) -> Result<String, String> {
        let quote = self.s[self.i];
        self.i += 1;
        let mut out = String::new();
        let mut run = self.i;
        while self.i < self.s.len() {
            match self.s[self.i] {
                b if b == quote => {
                    out.push_str(&self.text[run..self.i]);
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    out.push_str(&self.text[run..self.i]);
                    let Some(&e) = self.s.get(self.i + 1) else {
                        break;
                    };
                    self.i += 2;
                    match e {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let hex = self.text.get(self.i..self.i + 4).unwrap_or("");
                            let c = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
                            out.push(c.unwrap_or('\u{fffd}'));
                            self.i += 4;
                        }
                        _ => {
                            // `\"`, `\\`, `\/` and anything else: the char itself.
                            let c = self.text[self.i - 1..].chars().next().unwrap_or('?');
                            out.push(c);
                            self.i += c.len_utf8() - 1;
                        }
                    }
                    run = self.i;
                }
                b'\n' if quote == b'\'' => break,
                _ => self.i += 1,
            }
        }
        self.err("unterminated string")
    }

    fn push(&mut self, parent: Option<u32>, depth: u16, key: Key, kind: Kind) -> u32 {
        let idx = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        self.nodes.push(Node {
            parent,
            depth,
            key,
            kind,
            preview: String::new(),
            offset: self.i,
            value_end: self.i,
            end: idx + 1,
            children: 0,
        });
        idx
    }

    fn value(&mut self, parent: Option<u32>, depth: u16, key: Key) -> Result<(), String> {
        self.skip_ws();
        let Some(&c) = self.s.get(self.i) else {
            return self.err("unexpected end of input");
        };
        if depth > 512 {
            return self.err("nested too deeply");
        }
        match c {
            b'{' | b'[' => {
                let (kind, close) = if c == b'{' {
                    (Kind::Object, b'}')
                } else {
                    (Kind::Array, b']')
                };
                let me = self.push(parent, depth, key, kind);
                self.i += 1;
                let mut n = 0usize;
                loop {
                    self.skip_ws();
                    match self.s.get(self.i) {
                        None => return self.err("unclosed bracket"),
                        Some(&b) if b == close => {
                            self.i += 1;
                            break;
                        }
                        _ => {}
                    }
                    let child_key = if kind == Kind::Object {
                        let name = match self.s[self.i] {
                            b'"' | b'\'' => self.string()?,
                            _ => {
                                // JSON5 bare identifier keys.
                                let start = self.i;
                                while self.i < self.s.len()
                                    && (self.s[self.i].is_ascii_alphanumeric()
                                        || matches!(self.s[self.i], b'_' | b'$'))
                                {
                                    self.i += 1;
                                }
                                if start == self.i {
                                    return self.err("expected a key");
                                }
                                self.text[start..self.i].to_string()
                            }
                        };
                        self.skip_ws();
                        if self.s.get(self.i) != Some(&b':') {
                            return self.err("expected ':' after key");
                        }
                        self.i += 1;
                        Key::Name(name)
                    } else {
                        Key::Index(n)
                    };
                    self.value(Some(me), depth + 1, child_key)?;
                    n += 1;
                    self.skip_ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(&b) if b == close => {}
                        _ => return self.err("expected ',' or a closing bracket"),
                    }
                }
                let end = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
                let node = &mut self.nodes[me as usize];
                node.end = end;
                node.children = u32::try_from(n).unwrap_or(u32::MAX);
                node.value_end = self.i;
                node.preview = if kind == Kind::Object {
                    format!("{{{n}}}")
                } else {
                    format!("[{n}]")
                };
            }
            b'"' | b'\'' => {
                let me = self.push(parent, depth, key, Kind::String);
                let s = self.string()?;
                let node = &mut self.nodes[me as usize];
                node.value_end = self.i;
                node.preview = preview(&format!("\"{s}\""));
            }
            _ => {
                let me = self.push(parent, depth, key, Kind::Null);
                let start = self.i;
                while self.i < self.s.len()
                    && !matches!(self.s[self.i], b',' | b'}' | b']' | b'/')
                    && !self.s[self.i].is_ascii_whitespace()
                {
                    self.i += 1;
                }
                let word = &self.text[start..self.i];
                let kind = match word {
                    "true" | "false" => Kind::Bool,
                    "null" => Kind::Null,
                    w if w.parse::<f64>().is_ok()
                        || w.starts_with("0x")
                        || matches!(w, "Infinity" | "-Infinity" | "NaN") =>
                    {
                        Kind::Number
                    }
                    _ => return self.err(&format!("unexpected '{}'", preview(word))),
                };
                let node = &mut self.nodes[me as usize];
                node.kind = kind;
                node.value_end = self.i;
                node.preview = word.to_string();
            }
        }
        Ok(())
    }
}

fn preview(s: &str) -> String {
    let one_line: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(PREVIEW_MAX + 1)
        .collect();
    if one_line.chars().count() > PREVIEW_MAX {
        let cut: String = one_line.chars().take(PREVIEW_MAX).collect();
        format!("{cut}…")
    } else {
        one_line
    }
}

/// Parses `text` (JSON, also accepting comments, single quotes, bare keys and
/// trailing commas) into a flat node list.
pub fn parse(text: &str) -> Result<Vec<Node>, String> {
    let text_no_bom = text.strip_prefix('\u{feff}').unwrap_or(text);
    let bom = text.len() - text_no_bom.len();
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: bom,
        nodes: Vec::new(),
    };
    p.value(None, 0, Key::Root)?;
    p.skip_ws();
    if p.i < p.s.len() {
        return p.err("unexpected text after the JSON value");
    }
    Ok(p.nodes)
}

/// RFC 6901 JSON pointer of node `idx` (e.g. `/items/0/name`).
pub fn pointer(nodes: &[Node], idx: usize) -> String {
    let mut parts = Vec::new();
    let mut cur = Some(idx);
    while let Some(i) = cur {
        match &nodes[i].key {
            Key::Root => {}
            Key::Name(n) => parts.push(n.replace('~', "~0").replace('/', "~1")),
            Key::Index(n) => parts.push(n.to_string()),
        }
        cur = nodes[i].parent.map(|p| p as usize);
    }
    parts.reverse();
    parts.iter().map(|p| format!("/{p}")).collect()
}

/// Indices of the rows to show, skipping the children of collapsed nodes.
pub fn visible_rows(nodes: &[Node], collapsed: &std::collections::HashSet<u32>) -> Vec<u32> {
    let mut out = Vec::new();
    let mut i = 0u32;
    while (i as usize) < nodes.len() {
        out.push(i);
        i = if collapsed.contains(&i) {
            nodes[i as usize].end
        } else {
            i + 1
        };
    }
    out
}

/// Containers deeper than `depth`, to start collapsed.
pub fn collapse_below(nodes: &[Node], depth: u16) -> std::collections::HashSet<u32> {
    (0u32..)
        .zip(nodes)
        .filter(|(_, n)| n.depth >= depth && n.children > 0)
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_values_with_offsets() {
        let text = r#"{"a": [1, true, null], "b/c": {"d": "x\"y"}, "e": -1.5e3}"#;
        let n = parse(text).unwrap();
        let summary: Vec<_> = n
            .iter()
            .map(|n| (n.depth, n.kind, n.preview.as_str()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (0, Kind::Object, "{3}"),
                (1, Kind::Array, "[3]"),
                (2, Kind::Number, "1"),
                (2, Kind::Bool, "true"),
                (2, Kind::Null, "null"),
                (1, Kind::Object, "{1}"),
                (2, Kind::String, "\"x\"y\""),
                (1, Kind::Number, "-1.5e3"),
            ]
        );
        // Offsets point at the value in the source.
        assert_eq!(&text[n[1].offset..n[1].value_end], "[1, true, null]");
        assert_eq!(&text[n[6].offset..n[6].value_end], r#""x\"y""#);
        // Subtree ends.
        assert_eq!(n[0].end, 8);
        assert_eq!(n[1].end, 5);
        assert_eq!(n[5].end, 7);
    }

    #[test]
    fn pointers_escape_names() {
        let n = parse(r#"{"a": [0, {"b/c": 1, "t~": 2}]}"#).unwrap();
        let ptrs: Vec<_> = (0..n.len()).map(|i| pointer(&n, i)).collect();
        assert_eq!(ptrs, ["", "/a", "/a/0", "/a/1", "/a/1/b~1c", "/a/1/t~0"]);
    }

    #[test]
    fn lenient_syntax() {
        let n = parse("// c\n{a: 'x', /* y */ \"b\": [1, 2,],}\n").unwrap();
        assert_eq!(n.len(), 5);
        assert_eq!(n[1].key, Key::Name("a".into()));
        assert_eq!(n[2].preview, "[2]");
    }

    #[test]
    fn errors_name_the_line() {
        assert_eq!(
            parse("{\n\"a\": 1\n\"b\": 2}"),
            Err("Line 3: expected ',' or a closing bracket".into())
        );
        assert!(parse("[1, 2").is_err());
        assert!(parse("{\"a\": wat}").is_err());
        assert!(parse("1 2").is_err());
    }

    #[test]
    fn collapsing_skips_subtrees() {
        let n = parse(r#"{"a": {"b": [1, 2]}, "c": 3}"#).unwrap();
        let mut collapsed = std::collections::HashSet::new();
        assert_eq!(visible_rows(&n, &collapsed).len(), n.len());
        collapsed.insert(1); // "a"
        assert_eq!(visible_rows(&n, &collapsed), vec![0, 1, 5]);
        assert_eq!(collapse_below(&n, 2), [2].into_iter().collect());
    }

    impl PartialEq for Node {
        fn eq(&self, o: &Self) -> bool {
            self.offset == o.offset && self.kind == o.kind
        }
    }
}
