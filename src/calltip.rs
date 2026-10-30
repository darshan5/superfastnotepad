//! Call tips: after typing `name(`, find how `name` is defined in the same
//! file and show its parameter list.

use regex::Regex;

/// The longest tip shown.
const MAX_TIP: usize = 200;

/// Finds the definition of function `name` in `text` and returns it as
/// `name(params)` (whitespace collapsed), or `None` if there isn't one.
pub fn find_signature(text: &str, name: &str) -> Option<String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    {
        return None;
    }
    let n = regex::escape(name);
    // Parameter list: allows one level of nested parentheses.
    let params = r"(\((?:[^()]|\([^()]*\))*\))";
    let patterns = [
        // Keyword definitions: Rust, Python, JS/TS, Go, Kotlin, Swift, PHP,
        // Perl, PowerShell, shell `function`.
        format!(
            r"\b(?:fn|def|function|func|fun|sub|proc|procedure|method)\s+(?:\([^)]*\)\s*)?{n}\s*(?:<[^>()]*>)?\s*{params}"
        ),
        // JS/TS arrow functions and function expressions.
        format!(
            r"\b(?:const|let|var)\s+{n}\s*(?::[^=]+)?=\s*(?:async\s+)?(?:function\s*)?{params}"
        ),
        // C-like typed declarations followed by a body or ending in `;`
        // inside a class/header: `int add(int a, int b) {`, also qualified
        // as `void Widget::add(...)`.
        format!(
            r"(?m)^[ \t]*(?:[\w:<>,\*&\[\]]+[ \t]+)+[\*&]*(?:\w+::)*{n}[ \t]*{params}[^;{{\n]*(?:\{{|\n[ \t]*\{{|;)"
        ),
    ];
    // Only look at the few lines around each `name(` instead of running the
    // patterns over the whole file.
    let mut windows = String::new();
    let mut last_end = 0;
    for (at, _) in text.match_indices(name) {
        let after = text[at + name.len()..].trim_start_matches([' ', '\t']);
        if !after.starts_with('(')
            && !after.starts_with('<')
            && !after.starts_with('=')
            && !after.starts_with(':')
        {
            continue;
        }
        let start = text[..at].rfind('\n').map_or(0, |i| i + 1).max(last_end);
        let mut end = at;
        for _ in 0..6 {
            match text[end..].find('\n') {
                Some(i) => end += i + 1,
                None => {
                    end = text.len();
                    break;
                }
            }
        }
        if start < end {
            windows.push_str(&text[start..end]);
            if !windows.ends_with('\n') {
                windows.push('\n');
            }
            last_end = end;
        }
    }
    if windows.is_empty() {
        return None;
    }
    for p in &patterns {
        let Ok(re) = Regex::new(p) else { continue };
        if let Some(c) = re.captures(&windows) {
            let params = c.get(1)?.as_str();
            let flat = params.split_whitespace().collect::<Vec<_>>().join(" ");
            let tip = format!("{name}{flat}");
            return Some(if tip.chars().count() > MAX_TIP {
                format!("{}…", tip.chars().take(MAX_TIP).collect::<String>())
            } else {
                tip
            });
        }
    }
    None
}

/// Index of the parameter the caret is in, counting commas at the top
/// level between the `(` at `open` and the caret; `None` once the call's
/// closing `)` comes before the caret.
pub fn active_param(text_between: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut index = 0;
    let mut quote: Option<char> = None;
    for c in text_between.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            ',' if depth == 0 => index += 1,
            _ => {}
        }
    }
    Some(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_definitions_across_languages() {
        let cases = [
            (
                "fn add(a: i32, b: i32) -> i32 {\n}",
                "add",
                "add(a: i32, b: i32)",
            ),
            (
                "pub fn new<T: Clone>(items: Vec<T>) -> Self {",
                "new",
                "new(items: Vec<T>)",
            ),
            (
                "def greet(name,\n          greeting='hi'):\n",
                "greet",
                "greet(name, greeting='hi')",
            ),
            (
                "function load(url, opts = {}) {}",
                "load",
                "load(url, opts = {})",
            ),
            (
                "const sum = (xs: number[]) => xs.length;",
                "sum",
                "sum(xs: number[])",
            ),
            (
                "func (s *Server) Start(port int) error {",
                "Start",
                "Start(port int)",
            ),
            ("int max(int a, int b)\n{\n", "max", "max(int a, int b)"),
            (
                "void Notepad_plus::init(HINSTANCE h, HWND w) {",
                "init",
                "init(HINSTANCE h, HWND w)",
            ),
            (
                "    public static String join(List<String> parts, char sep) {",
                "join",
                "join(List<String> parts, char sep)",
            ),
            ("function Get-Item($Path) {", "Get", "Get"),
        ];
        for (src, name, want) in cases {
            let got = find_signature(src, name);
            if want == name {
                assert_eq!(got, None, "{src}");
            } else {
                assert_eq!(got.as_deref(), Some(want), "{src}");
            }
        }
    }

    #[test]
    fn ignores_plain_calls() {
        assert_eq!(find_signature("x = add(1, 2)\ny = add(3)", "add"), None);
        assert_eq!(find_signature("foo(1);", "foo"), None);
        assert_eq!(find_signature("fn a() {}", ""), None);
    }

    #[test]
    fn active_parameter() {
        assert_eq!(active_param(""), Some(0));
        assert_eq!(active_param("a, b"), Some(1));
        assert_eq!(active_param("f(x, y), \"a,b\", "), Some(2));
        assert_eq!(active_param("a) + 1"), None);
    }
}

#[cfg(test)]
mod bench {
    #[test]
    #[ignore = "benchmark: BENCH_FILE=path cargo test --release calltip_bench -- --ignored --nocapture"]
    fn calltip_bench() {
        let text =
            std::fs::read_to_string(std::env::var("BENCH_FILE").expect("BENCH_FILE")).unwrap();
        for name in ["command", "doesNotExistAnywhere", "init"] {
            let t = std::time::Instant::now();
            let r = super::find_signature(&text, name);
            println!(
                "{name:<22} {:?} in {:?}",
                r.map(|s| s.chars().take(50).collect::<String>()),
                t.elapsed()
            );
        }
    }
}
