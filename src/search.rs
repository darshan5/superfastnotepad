//! Find / Replace / Mark / Find in Files engine.
//!
//! Search modes: Normal, Extended (`\n \r \t \0 \xNN ...`)
//! and Regular expression (PCRE-like via `fancy-regex`, with back-references
//! and look-around). Replacement strings support `$1`, `\1`, `${name}`, `$&`,
//! and the case operators `\U \L \E \u \l`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use fancy_regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SearchMode {
    #[default]
    Normal,
    Extended,
    Regex,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SearchOptions {
    pub mode: SearchMode,
    pub match_case: bool,
    pub whole_word: bool,
    pub wrap: bool,
    pub dot_matches_newline: bool,
}

/// Expands Extended-mode escapes.
pub fn unescape_extended(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(n) = it.next() else {
            out.push('\\');
            break;
        };
        let take = |count: usize, radix: u32, it: &mut std::iter::Peekable<std::str::Chars>| {
            let mut digits = String::new();
            for _ in 0..count {
                match it.peek() {
                    Some(d) if d.is_digit(radix) => digits.push(it.next().unwrap()),
                    _ => break,
                }
            }
            u32::from_str_radix(&digits, radix)
                .ok()
                .and_then(char::from_u32)
        };
        match n {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            'x' => out.extend(take(2, 16, &mut it)),
            'u' => out.extend(take(4, 16, &mut it)),
            'd' => out.extend(take(3, 10, &mut it)),
            'o' => out.extend(take(3, 8, &mut it)),
            'b' => out.extend(take(8, 2, &mut it)),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

/// Rewrites the editor's regex dialect (`\K`, `\h`, line-aware `$` and `.`) into `fancy-regex` syntax.
pub fn translate_regex(pat: &str, dot_matches_newline: bool) -> String {
    let mut out = String::with_capacity(pat.len() + 16);
    let chars: Vec<char> = pat.chars().collect();
    let mut i = 0;
    let mut in_class = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let n = chars[i + 1];
            match n {
                'h' if in_class => out.push_str(" \\t"),
                'h' => out.push_str("[ \\t]"),
                'H' if !in_class => out.push_str("[^ \\t]"),
                '<' | '>' if !in_class => out.push_str("\\b"),
                'Z' if !in_class => out.push_str("(?=\\r?\\n?\\z)"),
                'R' if !in_class => out.push_str("(?:\\r\\n|\\n|\\r)"),
                _ => {
                    out.push(c);
                    out.push(n);
                }
            }
            i += 2;
            continue;
        }
        if in_class {
            if c == ']' {
                in_class = false;
            }
            out.push(c);
        } else {
            match c {
                '[' => {
                    in_class = true;
                    out.push(c);
                    // A leading ']' or '^]' is literal.
                    if chars.get(i + 1) == Some(&'^') {
                        out.push('^');
                        i += 1;
                    }
                    if chars.get(i + 1) == Some(&']') {
                        out.push_str("\\]");
                        i += 1;
                    }
                }
                '$' => out.push_str("(?=\\r\\n|\\n|\\r|\\z)"),
                '.' if !dot_matches_newline => out.push_str("[^\\r\\n]"),
                _ => out.push(c),
            }
        }
        i += 1;
    }
    out
}

pub fn build_regex(pattern: &str, opts: &SearchOptions) -> Result<Regex, String> {
    if pattern.is_empty() {
        return Err("empty pattern".into());
    }
    let core = match opts.mode {
        SearchMode::Normal => fancy_regex::escape(pattern).into_owned(),
        SearchMode::Extended => fancy_regex::escape(&unescape_extended(pattern)).into_owned(),
        SearchMode::Regex => translate_regex(pattern, opts.dot_matches_newline),
    };
    let mut full = String::new();
    full.push_str("(?m");
    if !opts.match_case {
        full.push('i');
    }
    if opts.mode == SearchMode::Regex && opts.dot_matches_newline {
        full.push('s');
    }
    full.push(')');
    if opts.whole_word && opts.mode != SearchMode::Regex {
        full.push_str(&format!("(?<![\\w]){core}(?![\\w])"));
    } else {
        full.push_str(&core);
    }
    fancy_regex::RegexBuilder::new(&full)
        .backtrack_limit(5_000_000)
        .build()
        .map_err(|e| e.to_string())
}

/// All matches as byte ranges.
pub fn find_all(re: &Regex, text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    while start <= text.len() {
        match re.find_from_pos(text, start) {
            Ok(Some(m)) => {
                out.push((m.start(), m.end()));
                start = if m.end() == m.start() {
                    next_char_boundary(text, m.end())
                } else {
                    m.end()
                };
            }
            _ => break,
        }
    }
    out
}

fn next_char_boundary(text: &str, mut i: usize) -> usize {
    i += 1;
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// First match at/after `from` (byte offset); wraps when `wrap` is set.
pub fn find_next(re: &Regex, text: &str, from: usize, wrap: bool) -> Option<(usize, usize, bool)> {
    let from = from.min(text.len());
    if let Ok(Some(m)) = re.find_from_pos(text, from) {
        return Some((m.start(), m.end(), false));
    }
    if wrap && let Ok(Some(m)) = re.find_from_pos(text, 0) {
        return Some((m.start(), m.end(), true));
    }
    None
}

/// Last match ending at/before `before` (byte offset); wraps when `wrap` is set.
pub fn find_prev(
    re: &Regex,
    text: &str,
    before: usize,
    wrap: bool,
) -> Option<(usize, usize, bool)> {
    let all = find_all(re, text);
    if let Some(&(s, e)) = all.iter().rev().find(|(s, e)| *e <= before && *s < before) {
        return Some((s, e, false));
    }
    if wrap && let Some(&(s, e)) = all.last() {
        return Some((s, e, true));
    }
    None
}

/// Expands a replacement template for one match.
pub fn expand_replacement(
    re: &Regex,
    text: &str,
    m: (usize, usize),
    template: &str,
    mode: SearchMode,
) -> String {
    match mode {
        SearchMode::Normal => template.to_string(),
        SearchMode::Extended => unescape_extended(template),
        SearchMode::Regex => {
            let Ok(Some(caps)) = re.captures_from_pos(text, m.0) else {
                return template.to_string();
            };
            let group = |name: &str| -> String {
                let g = if let Ok(n) = name.parse::<usize>() {
                    caps.get(n)
                } else {
                    caps.name(name)
                };
                g.map(|g| g.as_str().to_string()).unwrap_or_default()
            };
            #[derive(Clone, Copy, PartialEq)]
            enum Case {
                None,
                Upper,
                Lower,
            }
            let mut case = Case::None;
            let mut one_shot: Option<bool> = None; // Some(true)=upper next
            let mut out = String::new();
            let push = |s: &str, case: Case, one_shot: &mut Option<bool>, out: &mut String| {
                for ch in s.chars() {
                    if let Some(up) = one_shot.take() {
                        if up {
                            out.extend(ch.to_uppercase());
                        } else {
                            out.extend(ch.to_lowercase());
                        }
                        continue;
                    }
                    match case {
                        Case::None => out.push(ch),
                        Case::Upper => out.extend(ch.to_uppercase()),
                        Case::Lower => out.extend(ch.to_lowercase()),
                    }
                }
            };
            let t: Vec<char> = template.chars().collect();
            let mut i = 0;
            while i < t.len() {
                let c = t[i];
                if c == '\\' && i + 1 < t.len() {
                    let n = t[i + 1];
                    i += 2;
                    match n {
                        'n' => push("\n", case, &mut one_shot, &mut out),
                        'r' => push("\r", case, &mut one_shot, &mut out),
                        't' => push("\t", case, &mut one_shot, &mut out),
                        '0' => push(&group("0"), case, &mut one_shot, &mut out),
                        'U' => case = Case::Upper,
                        'L' => case = Case::Lower,
                        'E' => case = Case::None,
                        'u' => one_shot = Some(true),
                        'l' => one_shot = Some(false),
                        d if d.is_ascii_digit() => {
                            push(&group(&d.to_string()), case, &mut one_shot, &mut out)
                        }
                        other => push(&other.to_string(), case, &mut one_shot, &mut out),
                    }
                    continue;
                }
                if c == '$' && i + 1 < t.len() {
                    let n = t[i + 1];
                    if n == '&' {
                        push(&group("0"), case, &mut one_shot, &mut out);
                        i += 2;
                        continue;
                    }
                    if n == '{'
                        && let Some(close) = t[i + 2..].iter().position(|&x| x == '}')
                    {
                        let name: String = t[i + 2..i + 2 + close].iter().collect();
                        push(&group(&name), case, &mut one_shot, &mut out);
                        i += 3 + close;
                        continue;
                    }
                    if n.is_ascii_digit() {
                        let mut j = i + 1;
                        let mut num = String::new();
                        while j < t.len() && t[j].is_ascii_digit() && num.len() < 2 {
                            num.push(t[j]);
                            j += 1;
                        }
                        push(&group(&num), case, &mut one_shot, &mut out);
                        i = j;
                        continue;
                    }
                }
                push(&c.to_string(), case, &mut one_shot, &mut out);
                i += 1;
            }
            out
        }
    }
}

/// Replaces every match in `text`; returns the new text and the count.
pub fn replace_all_in(re: &Regex, text: &str, template: &str, mode: SearchMode) -> (String, usize) {
    let matches = find_all(re, text);
    if matches.is_empty() {
        return (text.to_string(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for &m in &matches {
        out.push_str(&text[last..m.0]);
        out.push_str(&expand_replacement(re, text, m, template, mode));
        last = m.1;
    }
    out.push_str(&text[last..]);
    (out, matches.len())
}

// ------------------------------------------------------------------ results

#[derive(Clone, Debug)]
pub struct LineHit {
    /// 0-based line.
    pub line: usize,
    pub text: String,
    /// Byte ranges into `text` that matched.
    pub ranges: Vec<(usize, usize)>,
}

#[derive(Clone, Debug)]
pub struct FileHits {
    pub path: Option<PathBuf>,
    pub doc_id: Option<u64>,
    pub title: String,
    pub hits: Vec<LineHit>,
}

#[derive(Clone, Debug)]
pub struct SearchResults {
    pub query: String,
    pub files: Vec<FileHits>,
    pub searched_files: usize,
}

impl SearchResults {
    pub fn total_hits(&self) -> usize {
        self.files
            .iter()
            .map(|f| f.hits.iter().map(|h| h.ranges.len()).sum::<usize>())
            .sum()
    }
}

/// Groups byte-range matches into per-line hits.
pub fn hits_by_line(text: &str, matches: &[(usize, usize)]) -> Vec<LineHit> {
    let mut out: Vec<LineHit> = Vec::new();
    if matches.is_empty() {
        return out;
    }
    let mut line_no = 0usize;
    let mut line_start = 0usize;
    let mut scan = 0usize;
    let bytes = text.as_bytes();
    for &(ms, me) in matches {
        while scan < ms {
            match bytes[scan] {
                b'\n' => {
                    line_no += 1;
                    line_start = scan + 1;
                }
                b'\r' => {
                    if scan + 1 < bytes.len() && bytes[scan + 1] == b'\n' {
                        scan += 1;
                    }
                    line_no += 1;
                    line_start = scan + 1;
                }
                _ => {}
            }
            scan += 1;
        }
        let line_end = text[line_start..]
            .find(['\r', '\n'])
            .map_or(text.len(), |p| line_start + p);
        let r = (ms - line_start, me.min(line_end).max(ms) - line_start);
        if let Some(last) = out.last_mut()
            && last.line == line_no
        {
            last.ranges.push(r);
            continue;
        }
        out.push(LineHit {
            line: line_no,
            text: text[line_start..line_end].to_string(),
            ranges: vec![r],
        });
    }
    out
}

// ------------------------------------------------------------------ find in files

#[derive(Clone, Debug, Default)]
pub struct FifParams {
    pub pattern: String,
    pub replace: Option<String>,
    pub dir: PathBuf,
    pub filters: String,
    pub recursive: bool,
    pub hidden: bool,
    pub opts: SearchOptions,
}

pub enum FifMessage {
    Progress(usize),
    File(FileHits),
    Replaced(usize),
    Done {
        searched: usize,
        error: Option<String>,
    },
}

/// Simple glob (`*`, `?`) matcher, case-insensitive.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    fn rec(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => rec(&p[1..], n) || (!n.is_empty() && rec(p, &n[1..])),
            (Some('?'), Some(_)) => rec(&p[1..], &n[1..]),
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) && rec(&p[1..], &n[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    rec(&p, &n)
}

/// Parses file filter syntax: `*.cpp *.h !*.bak !\build`.
pub fn parse_filters(filters: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut incl = Vec::new();
    let mut excl = Vec::new();
    let mut excl_dirs = Vec::new();
    for f in filters.split([' ', ';', ',']).filter(|s| !s.is_empty()) {
        if let Some(d) = f.strip_prefix("!\\").or_else(|| f.strip_prefix("!/")) {
            excl_dirs.push(d.trim_start_matches(['+']).to_string());
        } else if let Some(e) = f.strip_prefix('!') {
            excl.push(e.to_string());
        } else {
            incl.push(f.to_string());
        }
    }
    if incl.is_empty() {
        incl.push("*".into());
    }
    (incl, excl, excl_dirs)
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

/// Runs Find/Replace in Files on a background thread.
pub fn spawn_find_in_files(
    params: FifParams,
    tx: Sender<FifMessage>,
    cancel: Arc<AtomicBool>,
    wake: impl Fn() + Send + 'static,
) {
    std::thread::spawn(move || {
        let re = match build_regex(&params.pattern, &params.opts) {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(FifMessage::Done {
                    searched: 0,
                    error: Some(e),
                });
                wake();
                return;
            }
        };
        let (incl, excl, excl_dirs) = parse_filters(&params.filters);
        let mut searched = 0;
        let walker = walkdir::WalkDir::new(&params.dir)
            .max_depth(if params.recursive { usize::MAX } else { 1 })
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                if !params.hidden && is_hidden(e.path()) {
                    return false;
                }
                if e.file_type().is_dir() {
                    let name = e.file_name().to_string_lossy();
                    return !excl_dirs.iter().any(|d| glob_match(d, &name));
                }
                true
            });
        for entry in walker.filter_map(Result::ok) {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !incl.iter().any(|p| glob_match(p, &name))
                || excl.iter().any(|p| glob_match(p, &name))
            {
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            // Skip binary files.
            if bytes.iter().take(8000).any(|&b| b == 0) {
                continue;
            }
            searched += 1;
            if searched % 50 == 0 {
                let _ = tx.send(FifMessage::Progress(searched));
                wake();
            }
            let enc = crate::encoding::TextEncoding::detect(&bytes);
            let text = enc.decode(&bytes);
            let matches = find_all(&re, &text);
            if matches.is_empty() {
                continue;
            }
            if let Some(rep) = &params.replace {
                let (new_text, n) = replace_all_in(&re, &text, rep, params.opts.mode);
                if std::fs::write(entry.path(), enc.encode(&new_text)).is_ok() {
                    let _ = tx.send(FifMessage::Replaced(n));
                }
            } else {
                let hits = hits_by_line(&text, &matches);
                let _ = tx.send(FifMessage::File(FileHits {
                    path: Some(entry.path().to_path_buf()),
                    doc_id: None,
                    title: entry.path().display().to_string(),
                    hits,
                }));
            }
            wake();
        }
        let _ = tx.send(FifMessage::Done {
            searched,
            error: None,
        });
        wake();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(mode: SearchMode) -> SearchOptions {
        SearchOptions {
            mode,
            ..Default::default()
        }
    }

    #[test]
    fn normal_mode_escapes_metachars() {
        let re = build_regex("a.b", &opts(SearchMode::Normal)).unwrap();
        assert_eq!(find_all(&re, "a.b axb A.B"), vec![(0, 3), (8, 11)]);
    }

    #[test]
    fn match_case_and_whole_word() {
        let mut o = opts(SearchMode::Normal);
        o.match_case = true;
        o.whole_word = true;
        let re = build_regex("cat", &o).unwrap();
        assert_eq!(
            find_all(&re, "cat Cat concat cat_ cat"),
            vec![(0, 3), (20, 23)]
        );
    }

    #[test]
    fn extended_mode() {
        let re = build_regex("a\\tb\\x41", &opts(SearchMode::Extended)).unwrap();
        assert_eq!(find_all(&re, "a\tbA"), vec![(0, 4)]);
    }

    #[test]
    fn regex_dollar_handles_crlf() {
        let re = build_regex("o$", &opts(SearchMode::Regex)).unwrap();
        assert_eq!(find_all(&re, "foo\r\nbar\r\nzoo"), vec![(2, 3), (12, 13)]);
        let re = build_regex("^.+$", &opts(SearchMode::Regex)).unwrap();
        assert_eq!(find_all(&re, "ab\r\ncd"), vec![(0, 2), (4, 6)]);
    }

    #[test]
    fn regex_replace_groups_and_case() {
        let re = build_regex("(\\w+)@(\\w+)", &opts(SearchMode::Regex)).unwrap();
        let (out, n) = replace_all_in(&re, "joe@example x", "\\U$2\\E:\\1", SearchMode::Regex);
        assert_eq!(n, 1);
        assert_eq!(out, "EXAMPLE:joe x");
        let (out2, _) = replace_all_in(&re, "joe@ex", "\\u$1-${2}", SearchMode::Regex);
        assert_eq!(out2, "Joe-ex");
    }

    #[test]
    fn backreferences_work() {
        let re = build_regex("(\\w)\\1", &opts(SearchMode::Regex)).unwrap();
        assert_eq!(find_all(&re, "abccd ee"), vec![(2, 4), (6, 8)]);
    }

    #[test]
    fn next_prev_with_wrap() {
        let re = build_regex("x", &opts(SearchMode::Normal)).unwrap();
        let t = "x..x..";
        assert_eq!(find_next(&re, t, 1, false), Some((3, 4, false)));
        assert_eq!(find_next(&re, t, 4, true), Some((0, 1, true)));
        assert_eq!(find_prev(&re, t, 3, false), Some((0, 1, false)));
        assert_eq!(find_prev(&re, t, 0, true), Some((3, 4, true)));
    }

    #[test]
    fn line_grouping() {
        let t = "one x\r\ntwo\nx x three";
        let re = build_regex("x", &opts(SearchMode::Normal)).unwrap();
        let hits = hits_by_line(t, &find_all(&re, t));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line, 0);
        assert_eq!(hits[1].line, 2);
        assert_eq!(hits[1].ranges, vec![(0, 1), (2, 3)]);
        assert_eq!(hits[1].text, "x x three");
    }

    #[test]
    fn filters_and_globs() {
        let (i, e, d) = parse_filters("*.rs *.toml !*.lock !\\target");
        assert_eq!(i, vec!["*.rs", "*.toml"]);
        assert_eq!(e, vec!["*.lock"]);
        assert_eq!(d, vec!["target"]);
        assert!(glob_match("*.RS", "main.rs"));
        assert!(!glob_match("*.rs", "main.rsx"));
    }
}
