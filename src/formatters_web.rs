//! Pretty-printers for web languages: CSS (plus a minifier), HTML,
//! JavaScript / TypeScript and Markdown.
//!
//! These are token-based re-indenters, not full parsers. They aim to be
//! safe first: every printer keeps all non-whitespace content in order
//! (apart from a few deliberate normalisations such as an added `;` in CSS
//! or `*` bullets becoming `-` in Markdown) and returns `Err` when the input
//! is too ambiguous to re-indent without risk of corrupting it.
//!
//! All printers share `FormatOptions` (indent width, tabs, print width).

use crate::formatters::FormatOptions;
use unicode_width::UnicodeWidthStr;

// ------------------------------------------------------------------ helpers

fn indent_str(o: &FormatOptions, level: usize) -> String {
    if o.use_tabs {
        "\t".repeat(level)
    } else {
        " ".repeat(o.tab_width * level)
    }
}

fn indent_cols(o: &FormatOptions, level: usize) -> usize {
    level * if o.use_tabs { 4 } else { o.tab_width }
}

/// Byte offset → "line L, column C" (1-based).
fn location(text: &str, pos: usize) -> String {
    let pos = pos.min(text.len());
    let before = &text[..pos];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    format!("line {line}, column {col}")
}

/// Joins output lines, drops trailing blank lines and ends with one newline.
fn finish_lines(lines: &[String]) -> String {
    let mut s = lines.join("\n");
    let trimmed = s.trim_end_matches(['\n', ' ', '\t']).len();
    s.truncate(trimmed);
    if s.is_empty() {
        return String::new();
    }
    s.push('\n');
    s
}

// ------------------------------------------------------------------ CSS

#[derive(Debug, Clone, PartialEq)]
enum Ct {
    Ws(String),
    Comment(String),
    /// SCSS / LESS `// ...` comment (only recognised at a statement start).
    LineComment(String),
    Str(String),
    /// A whole `url(...)` call.
    Url(String),
    /// SCSS interpolation `#{...}`.
    Interp(String),
    Open,
    Close,
    Semi,
    Text(String),
}

fn css_at_stmt_start(toks: &[Ct]) -> bool {
    match toks.iter().rev().find(|t| !matches!(t, Ct::Ws(_))) {
        None => true,
        Some(t) => matches!(
            t,
            Ct::Open | Ct::Close | Ct::Semi | Ct::Comment(_) | Ct::LineComment(_)
        ),
    }
}

fn css_push_text(out: &mut Vec<Ct>, s: &str) {
    if let Some(Ct::Text(t)) = out.last_mut() {
        t.push_str(s);
    } else {
        out.push(Ct::Text(s.to_string()));
    }
}

/// Scans a quoted string starting at `i` (the quote); returns the end offset.
fn css_scan_string(s: &str, i: usize) -> Result<usize, String> {
    let b = s.as_bytes();
    let q = b[i];
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'\n' => break,
            c if c == q => return Ok(j + 1),
            _ => j += 1,
        }
    }
    Err(format!("Unterminated string at {}", location(s, i)))
}

fn css_tokenize(s: &str) -> Result<Vec<Ct>, String> {
    let b = s.as_bytes();
    let mut out: Vec<Ct> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            let st = i;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push(Ct::Ws(s[st..i].to_string()));
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = s[i + 2..]
                .find("*/")
                .ok_or_else(|| format!("Unterminated comment at {}", location(s, i)))?;
            let j = i + 2 + end + 2;
            out.push(Ct::Comment(s[i..j].to_string()));
            i = j;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') && css_at_stmt_start(&out) {
            let j = s[i..].find('\n').map_or(b.len(), |e| i + e);
            out.push(Ct::LineComment(s[i..j].trim_end().to_string()));
            i = j;
        } else if c == b'"' || c == b'\'' {
            let j = css_scan_string(s, i)?;
            out.push(Ct::Str(s[i..j].to_string()));
            i = j;
        } else if c == b'{' {
            out.push(Ct::Open);
            i += 1;
        } else if c == b'}' {
            out.push(Ct::Close);
            i += 1;
        } else if c == b';' {
            out.push(Ct::Semi);
            i += 1;
        } else if c == b'#' && b.get(i + 1) == Some(&b'{') {
            let mut depth = 0;
            let mut j = i + 1;
            loop {
                match b.get(j) {
                    None => {
                        return Err(format!("Unterminated #{{...}} at {}", location(s, i)));
                    }
                    Some(b'{') => depth += 1,
                    Some(b'}') => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    Some(b'"' | b'\'') => {
                        j = css_scan_string(s, j)?;
                        continue;
                    }
                    _ => {}
                }
                j += 1;
            }
            out.push(Ct::Interp(s[i..=j].to_string()));
            i = j + 1;
        } else if (c == b'u' || c == b'U')
            && s.get(i..i + 4)
                .is_some_and(|w| w.eq_ignore_ascii_case("url("))
            && (i == 0
                || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'-' || b[i - 1] == b'_'))
        {
            let mut j = i + 4;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                j = css_scan_string(s, j)?;
            }
            while j < b.len() && b[j] != b')' {
                if b[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            if j >= b.len() {
                return Err(format!("Unterminated url( at {}", location(s, i)));
            }
            out.push(Ct::Url(s[i..=j].to_string()));
            i = j + 1;
        } else {
            // A run of ordinary characters; the special starters above end it.
            let st = i;
            i += 1;
            while i < b.len()
                && !b[i].is_ascii_whitespace()
                && !matches!(
                    b[i],
                    b'"' | b'\'' | b'{' | b'}' | b';' | b'/' | b'#' | b'u' | b'U'
                )
            {
                i += 1;
            }
            css_push_text(&mut out, &s[st..i]);
        }
    }
    Ok(out)
}

/// Renders a prelude (selector, declaration or at-rule) with whitespace runs
/// collapsed to single spaces. With `comma_space`, top-level commas get
/// exactly one space after them and none before.
fn css_render(toks: &[Ct], comma_space: bool) -> String {
    let mut s = String::new();
    let mut pending_ws = false;
    for t in toks {
        match t {
            Ct::Ws(_) => pending_ws = true,
            Ct::Text(x) => {
                for ch in x.chars() {
                    if comma_space && ch == ',' {
                        while s.ends_with(' ') {
                            s.pop();
                        }
                        s.push(',');
                        pending_ws = true;
                        continue;
                    }
                    if pending_ws && !s.is_empty() {
                        s.push(' ');
                    }
                    pending_ws = false;
                    s.push(ch);
                }
            }
            Ct::Comment(x) | Ct::Str(x) | Ct::Url(x) | Ct::Interp(x) | Ct::LineComment(x) => {
                if pending_ws && !s.is_empty() {
                    s.push(' ');
                }
                pending_ws = false;
                s.push_str(x);
            }
            Ct::Open => s.push('{'),
            Ct::Close => s.push('}'),
            Ct::Semi => s.push(';'),
        }
    }
    s
}

fn css_trim(toks: &[Ct]) -> &[Ct] {
    let st = toks
        .iter()
        .position(|t| !matches!(t, Ct::Ws(_)))
        .unwrap_or(toks.len());
    let en = toks
        .iter()
        .rposition(|t| !matches!(t, Ct::Ws(_)))
        .map_or(st, |p| p + 1);
    &toks[st..en]
}

/// Splits a prelude at top-level occurrences of `sep` inside `Text` tokens
/// (outside parentheses and brackets). Returns `None` when `sep` is absent.
fn css_split_top(toks: &[Ct], sep: char, first_only: bool) -> Vec<Vec<Ct>> {
    let mut parts: Vec<Vec<Ct>> = vec![Vec::new()];
    let mut depth = 0i32;
    let mut done = false;
    for t in toks {
        if let Ct::Text(x) = t
            && !done
        {
            let mut cur = String::new();
            for ch in x.chars() {
                match ch {
                    '(' | '[' => depth += 1,
                    ')' | ']' => depth -= 1,
                    _ => {}
                }
                if ch == sep && depth == 0 && !done {
                    if !cur.is_empty() {
                        parts
                            .last_mut()
                            .unwrap()
                            .push(Ct::Text(std::mem::take(&mut cur)));
                    }
                    parts.push(Vec::new());
                    if first_only {
                        done = true;
                    }
                } else {
                    cur.push(ch);
                }
            }
            if !cur.is_empty() {
                parts.last_mut().unwrap().push(Ct::Text(cur));
            }
        } else {
            parts.last_mut().unwrap().push(t.clone());
        }
    }
    parts
}

fn css_has_text(toks: &[Ct], pat: &str) -> bool {
    toks.iter()
        .any(|t| matches!(t, Ct::Text(x) if x.contains(pat)))
}

/// A declaration `name: value` (or a SCSS `$var: value`) → normalised text.
fn css_declaration(toks: &[Ct], depth: usize) -> String {
    let toks = css_trim(toks);
    let starts = css_render(&toks[..toks.len().min(1)], false);
    let custom = starts.starts_with("--");
    let is_decl = !starts.starts_with('@')
        && (depth > 0 || starts.starts_with('$') || custom)
        && toks.first().is_some_and(|t| matches!(t, Ct::Text(_)));
    if is_decl {
        let parts = css_split_top(toks, ':', true);
        if parts.len() == 2 {
            let name = css_render(css_trim(&parts[0]), false);
            let value = css_render(css_trim(&parts[1]), !custom);
            // `a:hover` style pseudo selectors never end with `;`, so a
            // single top-level colon here is a declaration.
            return if value.is_empty() {
                format!("{name}:")
            } else {
                format!("{name}: {value}")
            };
        }
    }
    css_render(toks, false)
}

/// Selector list → one selector per line (at-rule headers stay on one line).
fn css_selector(toks: &[Ct], ind: &str) -> String {
    let toks = css_trim(toks);
    let head = css_render(toks, false);
    if head.starts_with('@') {
        return head;
    }
    css_split_top(toks, ',', false)
        .iter()
        .map(|p| css_render(css_trim(p), false))
        .collect::<Vec<_>>()
        .join(&format!(",\n{ind}"))
}

pub fn css_format(text: &str, o: &FormatOptions) -> Result<String, String> {
    let toks = css_tokenize(text)?;
    let mut out: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut prelude: Vec<Ct> = Vec::new();
    // A blank line was seen in the input since the last emitted item.
    let mut blank = false;
    // A newline was seen since the last emitted item (for trailing comments).
    let mut newline = true;
    // The previous top-level item was a block.
    let mut after_block = false;
    // The last emitted line opened a block.
    let mut after_open = true;
    // The last emitted item was a comment (which then sticks to the next rule).
    let mut after_comment = false;

    fn sep(out: &mut Vec<String>, depth: usize, blank: bool, after_block: bool, after_open: bool) {
        if out.is_empty() || after_open {
            return;
        }
        if blank || (depth == 0 && after_block) {
            out.push(String::new());
        }
    }

    for t in toks {
        let empty = css_trim(&prelude).is_empty();
        match t {
            Ct::Ws(w) => {
                if empty {
                    let n = w.matches('\n').count();
                    if n >= 2 {
                        blank = true;
                    }
                    if n >= 1 {
                        newline = true;
                    }
                    prelude.clear();
                } else {
                    prelude.push(Ct::Ws(w));
                }
            }
            Ct::Comment(c) | Ct::LineComment(c) if empty => {
                if !newline && !after_open && !out.is_empty() {
                    let last = out.last_mut().unwrap();
                    last.push(' ');
                    last.push_str(&c);
                } else {
                    sep(&mut out, depth, blank, after_block, after_open);
                    out.push(format!("{}{c}", indent_str(o, depth)));
                    after_open = false;
                    after_block = false;
                    after_comment = true;
                }
                blank = false;
                newline = false;
                prelude.clear();
            }
            Ct::Open => {
                if css_has_text(&prelude, "//") {
                    return Err("Unsupported `//` comment inside a selector".into());
                }
                let top_rule = depth == 0 && !after_comment;
                sep(&mut out, depth, blank, after_block || top_rule, after_open);
                after_comment = false;
                let ind = indent_str(o, depth);
                let sel = css_selector(&prelude, &ind);
                out.push(if sel.is_empty() {
                    format!("{ind}{{")
                } else {
                    format!("{ind}{sel} {{")
                });
                depth += 1;
                prelude.clear();
                blank = false;
                newline = false;
                after_open = true;
            }
            Ct::Semi | Ct::Close => {
                if !empty {
                    if css_has_text(&prelude, "//") {
                        return Err("Unsupported `//` comment inside a declaration".into());
                    }
                    sep(&mut out, depth, blank, after_block, after_open);
                    let d = css_declaration(&prelude, depth);
                    out.push(format!("{}{d};", indent_str(o, depth)));
                    after_open = false;
                    after_block = false;
                    after_comment = false;
                } else if t == Ct::Semi {
                    // A stray `;`: keep it attached to the previous line.
                    match out.last_mut() {
                        Some(l) if !after_open || depth > 0 => l.push(';'),
                        _ => out.push(";".into()),
                    }
                }
                if t == Ct::Close {
                    if depth == 0 {
                        return Err("Unbalanced '}'".into());
                    }
                    depth -= 1;
                    out.push(format!("{}}}", indent_str(o, depth)));
                    after_open = false;
                    after_block = true;
                }
                prelude.clear();
                blank = false;
                newline = false;
            }
            other => prelude.push(other),
        }
    }
    if depth > 0 {
        return Err("Unclosed '{' at end of input".into());
    }
    if !css_trim(&prelude).is_empty() {
        sep(&mut out, depth, blank, after_block, after_open);
        out.push(css_declaration(&prelude, 0));
    }
    Ok(finish_lines(&out))
}

/// Minifies CSS: drops comments (except `/*! ... */`), collapses whitespace
/// and removes the last `;` in each block. Strings are never altered.
pub fn css_minify(text: &str) -> Result<String, String> {
    let toks = css_tokenize(text)?;
    // For each token: does the statement it belongs to end with `{`?
    let mut opens_block = vec![false; toks.len()];
    let mut next_open = false;
    for (k, t) in toks.iter().enumerate().rev() {
        match t {
            Ct::Open => next_open = true,
            Ct::Semi | Ct::Close => next_open = false,
            _ => {}
        }
        opens_block[k] = next_open;
    }
    let mut s = String::with_capacity(text.len());
    let mut space = false;
    let mut semi = false;
    let mut depth = 0usize;
    for (k, t) in toks.into_iter().enumerate() {
        let x = match t {
            Ct::Ws(_) => {
                space = true;
                continue;
            }
            Ct::Comment(c) if c.starts_with("/*!") => c,
            Ct::Comment(_) | Ct::LineComment(_) => {
                space = true;
                continue;
            }
            Ct::Semi => {
                if semi {
                    s.push(';');
                }
                semi = true;
                space = false;
                continue;
            }
            Ct::Close => {
                semi = false;
                space = false;
                depth = depth.saturating_sub(1);
                s.push('}');
                continue;
            }
            Ct::Open => {
                depth += 1;
                "{".to_string()
            }
            Ct::Text(x) | Ct::Str(x) | Ct::Url(x) | Ct::Interp(x) => x,
        };
        if semi {
            s.push(';');
            semi = false;
        } else if space
            && !s.is_empty()
            && !s.ends_with(['{', '}', ';', ',', ':', '('])
            && !s.ends_with("*/")
            && !x.starts_with(['{', '}', ';', ',', ')'])
            // In a declaration (not a selector) `color : red` → `color:red`.
            && !(x.starts_with(':') && depth > 0 && !opens_block[k])
        {
            s.push(' ');
        }
        space = false;
        s.push_str(&x);
    }
    if semi {
        s.push(';');
    }
    Ok(s)
}

// ------------------------------------------------------------------ JavaScript / TypeScript lexer

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Jk {
    Ident,
    Num,
    Str,
    Template,
    Regex,
    Punct,
    LineComment,
    BlockComment,
}

#[derive(Clone, Copy, Debug)]
struct Jt<'a> {
    kind: Jk,
    text: &'a str,
    /// Newlines in the whitespace before this token.
    nl: usize,
    /// Any whitespace before this token.
    ws: bool,
}

impl Jt<'_> {
    fn is_comment(&self) -> bool {
        matches!(self.kind, Jk::LineComment | Jk::BlockComment)
    }
    fn is(&self, p: &str) -> bool {
        self.kind == Jk::Punct && self.text == p
    }
    fn word(&self, w: &str) -> bool {
        self.kind == Jk::Ident && self.text == w
    }
}

const JS_PUNCT: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "**", "<<", ">>",
];
const JS_PUNCT1: &str = "{}()[];,<>+-*/%&|^!~?:=.@";

/// Keywords after which an expression (so possibly a regex) starts.
const JS_KW_BEFORE_EXPR: &[&str] = &[
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
    "extends",
];

fn js_ident_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$' || ch == '\u{200c}' || ch == '\u{200d}'
}

fn js_ident_start(ch: char) -> bool {
    js_ident_char(ch) && !ch.is_ascii_digit()
}

struct JsLex<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
    toks: Vec<Jt<'a>>,
    parens: Vec<bool>,
    last_paren_ctrl: bool,
    nl: usize,
    ws: bool,
}

impl<'a> JsLex<'a> {
    fn new(s: &'a str, i: usize) -> Self {
        Self {
            s,
            b: s.as_bytes(),
            i,
            toks: Vec::new(),
            parens: Vec::new(),
            last_paren_ctrl: false,
            nl: 0,
            ws: false,
        }
    }

    fn err(&self, msg: &str, at: usize) -> String {
        format!("{msg} at {}", location(self.s, at))
    }

    fn push(&mut self, kind: Jk, st: usize, en: usize) {
        self.toks.push(Jt {
            kind,
            text: &self.s[st..en],
            nl: self.nl,
            ws: self.ws,
        });
        self.nl = 0;
        self.ws = false;
        self.i = en;
    }

    /// Index of the last non-comment token.
    fn prev_sig(&self) -> Option<usize> {
        self.toks.iter().rposition(|t| !t.is_comment())
    }

    fn is_property(&self, idx: usize) -> bool {
        self.toks[..idx]
            .iter()
            .rev()
            .find(|t| !t.is_comment())
            .is_some_and(|t| t.is(".") || t.is("?."))
    }

    /// Decides regex literal vs division from the previous significant token.
    fn regex_ok(&self) -> bool {
        let Some(p) = self.prev_sig() else {
            return true;
        };
        let t = self.toks[p];
        match t.kind {
            Jk::Num | Jk::Str | Jk::Template | Jk::Regex => false,
            Jk::Ident => JS_KW_BEFORE_EXPR.contains(&t.text) && !self.is_property(p),
            Jk::Punct => match t.text {
                ")" => self.last_paren_ctrl,
                "]" | "++" | "--" => false,
                _ => true,
            },
            _ => true,
        }
    }

    fn scan_string(&self, st: usize) -> Result<usize, String> {
        let q = self.b[st];
        let mut j = st + 1;
        while j < self.b.len() {
            match self.b[j] {
                b'\\' => {
                    j += 1;
                    if self.b.get(j) == Some(&b'\r') && self.b.get(j + 1) == Some(&b'\n') {
                        j += 1;
                    }
                    j += 1;
                }
                b'\n' => break,
                c if c == q => return Ok(j + 1),
                _ => j += 1,
            }
        }
        Err(self.err("Unterminated string", st))
    }

    fn scan_template(&self, st: usize) -> Result<usize, String> {
        let mut j = st + 1;
        while j < self.b.len() {
            match self.b[j] {
                b'\\' => j += 2,
                b'`' => return Ok(j + 1),
                b'$' if self.b.get(j + 1) == Some(&b'{') => {
                    let mut sub = JsLex::new(self.s, j + 2);
                    j = sub.run(true)? + 1;
                }
                _ => j += 1,
            }
        }
        Err(self.err("Unterminated template literal", st))
    }

    fn scan_regex(&self, st: usize) -> Option<usize> {
        let mut j = st + 1;
        let mut class = false;
        while j < self.b.len() {
            match self.b[j] {
                b'\\' => j += 1,
                b'\n' | b'\r' => return None,
                b'[' => class = true,
                b']' => class = false,
                b'/' if !class => {
                    j += 1;
                    let rest = &self.s[j..];
                    let flags = rest
                        .char_indices()
                        .find(|&(_, c)| !js_ident_char(c))
                        .map_or(rest.len(), |(k, _)| k);
                    return Some(j + flags);
                }
                _ => {}
            }
            j += 1;
        }
        None
    }

    fn scan_number(&self, st: usize) -> usize {
        let b = self.b;
        let mut j = st;
        let alnum = |j: usize| j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_');
        if b[j] == b'0' && matches!(b.get(j + 1), Some(b'x' | b'X' | b'o' | b'O' | b'b' | b'B')) {
            j += 2;
        } else {
            let (mut dot, mut exp) = (false, false);
            while j < b.len() {
                match b[j] {
                    b'0'..=b'9' | b'_' => j += 1,
                    b'.' if !dot && !exp => {
                        dot = true;
                        j += 1;
                    }
                    b'e' | b'E' if !exp => {
                        exp = true;
                        j += 1;
                        if matches!(b.get(j), Some(b'+' | b'-')) {
                            j += 1;
                        }
                    }
                    _ => break,
                }
            }
        }
        while alnum(j) {
            j += 1;
        }
        j
    }

    fn scan_ident(&self, st: usize) -> usize {
        let mut j = st;
        if self.b[j] == b'#' {
            j += 1;
        }
        while j < self.b.len() {
            if self.b[j] == b'\\' {
                j += 2;
                continue;
            }
            match self.s[j..].chars().next() {
                Some(c) if js_ident_char(c) => j += c.len_utf8(),
                _ => break,
            }
        }
        j.min(self.b.len())
    }

    /// `<` in expression position: allow `<T, U>(` generic arrows, reject JSX.
    fn check_angle(&self, st: usize) -> Result<(), String> {
        let rest = &self.s[st + 1..];
        let Some(first) = rest.chars().next() else {
            return Ok(());
        };
        if !(js_ident_start(first) || first == '>') {
            return Ok(());
        }
        let close = rest.find(|c: char| !(js_ident_char(c) || c == ',' || c == ' ' || c == '\t'));
        if let Some(k) = close
            && rest[k..].starts_with('>')
            && rest[k + 1..].trim_start().starts_with('(')
        {
            return Ok(());
        }
        Err(self.err("JSX and `<Type>` casts are not supported", st))
    }

    /// Tokenizes until the end of input, or (inside `${...}`) until the
    /// matching `}`. Returns the offset where it stopped.
    fn run(&mut self, stop_at_brace: bool) -> Result<usize, String> {
        let mut depth = 0usize;
        while self.i < self.b.len() {
            let st = self.i;
            let c = self.b[st];
            let next = self.b.get(st + 1).copied();
            if c == b'\n' {
                self.nl += 1;
                self.ws = true;
                self.i += 1;
                continue;
            }
            if c.is_ascii_whitespace() {
                self.ws = true;
                self.i += 1;
                continue;
            }
            if c >= 0x80 {
                let ch = self.s[st..].chars().next().unwrap_or(' ');
                if ch.is_whitespace() || ch == '\u{feff}' {
                    if ch == '\u{2028}' || ch == '\u{2029}' {
                        self.nl += 1;
                    }
                    self.ws = true;
                    self.i += ch.len_utf8();
                    continue;
                }
            }
            if (c == b'/' && next == Some(b'/')) || (c == b'#' && st == 0 && next == Some(b'!')) {
                let en = self.s[st..].find('\n').map_or(self.b.len(), |e| st + e);
                let text_end = st + self.s[st..en].trim_end().len();
                self.push(Jk::LineComment, st, text_end);
                self.i = en;
            } else if c == b'/' && next == Some(b'*') {
                let e = self.s[st + 2..]
                    .find("*/")
                    .ok_or_else(|| self.err("Unterminated comment", st))?;
                self.push(Jk::BlockComment, st, st + 2 + e + 2);
            } else if c == b'\'' || c == b'"' {
                let en = self.scan_string(st)?;
                self.push(Jk::Str, st, en);
            } else if c == b'`' {
                let en = self.scan_template(st)?;
                self.push(Jk::Template, st, en);
            } else if c.is_ascii_digit() || (c == b'.' && next.is_some_and(|n| n.is_ascii_digit()))
            {
                let en = self.scan_number(st);
                self.push(Jk::Num, st, en);
            } else if c == b'/'
                && self.regex_ok()
                && let Some(en) = self.scan_regex(st)
            {
                self.push(Jk::Regex, st, en);
            } else if c == b'\\'
                || (c == b'#' && self.s[st + 1..].chars().next().is_some_and(js_ident_start))
                || self.s[st..].chars().next().is_some_and(js_ident_start)
            {
                let en = self.scan_ident(st);
                self.push(Jk::Ident, st, en);
            } else {
                if c == b'<' && self.regex_ok() {
                    self.check_angle(st)?;
                }
                let rest = &self.s[st..];
                let mut len = JS_PUNCT
                    .iter()
                    .find(|p| rest.starts_with(**p))
                    .map_or(0, |p| p.len());
                if len == 2
                    && rest.starts_with("?.")
                    && rest.as_bytes().get(2).is_some_and(|d| d.is_ascii_digit())
                {
                    len = 1;
                }
                if len == 0 {
                    if JS_PUNCT1.as_bytes().contains(&c) {
                        len = 1;
                    } else {
                        return Err(self.err("Unexpected character", st));
                    }
                }
                match c {
                    b'{' => depth += 1,
                    b'}' if len == 1 => {
                        if stop_at_brace && depth == 0 {
                            return Ok(st);
                        }
                        depth = depth.saturating_sub(1);
                    }
                    b'(' => {
                        let ctrl = self.prev_sig().is_some_and(|p| {
                            let t = self.toks[p];
                            t.kind == Jk::Ident
                                && matches!(t.text, "if" | "while" | "for" | "with")
                                && !self.is_property(p)
                        });
                        self.parens.push(ctrl);
                    }
                    b')' => self.last_paren_ctrl = self.parens.pop().unwrap_or(false),
                    _ => {}
                }
                self.push(Jk::Punct, st, st + len);
            }
        }
        if stop_at_brace {
            return Err("Unterminated ${...} in template literal".into());
        }
        Ok(self.b.len())
    }
}

fn js_tokenize(s: &str) -> Result<Vec<Jt<'_>>, String> {
    let mut lex = JsLex::new(s, 0);
    lex.run(false)?;
    Ok(lex.toks)
}

// ------------------------------------------------------------------ JavaScript / TypeScript printer

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Operand,
    /// Keyword that starts an expression (`return`, `typeof`, ...).
    KwOp,
    Kw,
    /// Contextual keyword that may also be an identifier (`async`, `get`, `type`, ...).
    Ctx,
    Prefix,
    Postfix,
    Binary,
    /// `<`, `>`, generator `*`: spacing kept as written (generics, JSX-free TS).
    Angle,
    Open,
    Close,
    Sep,
    Dot,
    Colon,
    /// TypeScript optional marker `x?: T`.
    Optional,
    Comment,
}

const JS_KW: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "return",
    "switch",
    "throw",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
];

const JS_CTX: &[&str] = &[
    "async",
    "await",
    "yield",
    "let",
    "static",
    "get",
    "set",
    "of",
    "from",
    "as",
    "satisfies",
    "type",
    "declare",
    "readonly",
    "abstract",
    "namespace",
    "module",
    "is",
    "keyof",
    "infer",
    "public",
    "private",
    "protected",
    "override",
    "accessor",
    "unique",
    "implements",
    "interface",
    "package",
];

/// Words after which `{` starts an object literal / pattern / type, not a block.
const JS_OBJ_AFTER_WORD: &[&str] = &[
    "return",
    "const",
    "let",
    "var",
    "import",
    "export",
    "default",
    "typeof",
    "case",
    "in",
    "of",
    "yield",
    "await",
    "throw",
    "extends",
    "as",
    "satisfies",
    "new",
    "void",
    "delete",
    "instanceof",
    "keyof",
    "type",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bk {
    Block,
    Object,
    Paren,
    Index,
    Array,
}

fn js_roles(toks: &[Jt]) -> Vec<Role> {
    let mut roles = vec![Role::Comment; toks.len()];
    let mut prev: Option<usize> = None;
    for k in 0..toks.len() {
        let t = toks[k];
        if t.is_comment() {
            continue;
        }
        let pr = prev.map(|p| roles[p]);
        let operand_before = matches!(pr, Some(Role::Operand | Role::Close | Role::Postfix));
        let r = match t.kind {
            Jk::Ident => {
                if pr == Some(Role::Dot) {
                    Role::Operand
                } else if JS_CTX.contains(&t.text) {
                    Role::Ctx
                } else if JS_KW_BEFORE_EXPR.contains(&t.text) {
                    Role::KwOp
                } else if JS_KW.contains(&t.text) {
                    Role::Kw
                } else {
                    Role::Operand
                }
            }
            Jk::Num | Jk::Str | Jk::Template | Jk::Regex => Role::Operand,
            Jk::Punct => match t.text {
                "(" | "[" | "{" => Role::Open,
                ")" | "]" | "}" => Role::Close,
                "," | ";" => Role::Sep,
                "." | "?." => Role::Dot,
                "..." | "~" | "@" => Role::Prefix,
                "!" if operand_before && !t.ws => Role::Postfix,
                "!" => Role::Prefix,
                "+" | "-" if operand_before => Role::Binary,
                "+" | "-" => Role::Prefix,
                "++" | "--" if operand_before && t.nl == 0 => Role::Postfix,
                "++" | "--" => Role::Prefix,
                // Could close a generic (`Array<T>=x` lexes as `>=`): keep spacing.
                "<" | ">" | ">>" | ">>>" | ">=" | ">>=" | ">>>=" => Role::Angle,
                // `}\n*gen() {}`: a generator method, not multiplication.
                "*" if t.nl > 0 && prev.is_some_and(|p| toks[p].is("}") || toks[p].is(";")) => {
                    Role::Prefix
                }
                "*" if operand_before => Role::Binary,
                "*" => Role::Angle,
                ":" => Role::Colon,
                "?" => {
                    let next = toks[k + 1..].iter().find(|t| !t.is_comment());
                    if next.is_some_and(|n| n.is(":") || n.is(")") || n.is(",") || n.is("=")) {
                        Role::Optional
                    } else {
                        Role::Binary
                    }
                }
                _ => Role::Binary,
            },
            _ => Role::Comment,
        };
        roles[k] = r;
        prev = Some(k);
    }
    roles
}

fn js_space(p: &Jt, pr: Role, t: &Jt, tr: Role) -> bool {
    use Role::*;
    if tr == Sep {
        return false;
    }
    if tr == Comment || pr == Comment {
        return true;
    }
    if pr == Angle && p.text.ends_with('>') && t.is("{") {
        return true;
    }
    // `= <T>(x) => x`: a generic arrow after an operator.
    if t.is("<") && matches!(pr, Binary | Colon | KwOp | Sep) {
        return true;
    }
    if pr == Angle || tr == Angle {
        return t.ws;
    }
    if pr == Sep {
        return !(t.is(")") || t.is("]"));
    }
    if pr == Open {
        return p.is("{") && !t.is("}");
    }
    if tr == Close {
        return t.is("}");
    }
    if matches!(pr, Dot | Prefix) || matches!(tr, Dot | Postfix | Optional | Colon) {
        return false;
    }
    if t.is("(") {
        return match pr {
            Operand | Close | Postfix => false,
            Ctx => t.ws,
            Kw | KwOp => p.text != "import",
            _ => true,
        };
    }
    if t.is("[") {
        return match pr {
            Operand | Close | Postfix => false,
            Ctx => t.ws,
            _ => true,
        };
    }
    if t.kind == Jk::Template && matches!(pr, Operand | Close | Ctx) {
        return t.ws;
    }
    true
}

/// Would printing `a` directly followed by `b` tokenize differently?
fn js_must_separate(a: &str, b: &str) -> bool {
    let (Some(x), Some(y)) = (a.chars().last(), b.chars().next()) else {
        return false;
    };
    let word = |c: char| js_ident_char(c) || c == '\\';
    if word(x) && (word(y) || y == '#') {
        return true;
    }
    if word(x) && y == '.' && a.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return true;
    }
    if x == '.' && y.is_ascii_digit() {
        return true;
    }
    if x == '/' && (y == '/' || y == '*' || (word(y) && a.len() > 1)) {
        return true;
    }
    if !word(x) && !word(y) {
        let mut combo = String::from(a);
        combo.push(y);
        if JS_PUNCT.iter().any(|p| p.starts_with(combo.as_str())) {
            return true;
        }
    }
    false
}

/// True if the source has a string or template literal spanning lines, so
/// re-indenting its lines (e.g. inside HTML) would change the program.
fn js_has_multiline_literal(text: &str) -> bool {
    js_tokenize(text).map_or(true, |toks| {
        toks.iter()
            .any(|t| matches!(t.kind, Jk::Str | Jk::Template | Jk::Regex) && t.text.contains('\n'))
    })
}

struct Frame {
    kind: Bk,
    broken: bool,
    inner: usize,
    outer: usize,
    switch: bool,
    ternary: usize,
}

fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Pretty-prints JavaScript or TypeScript (a token-based re-indenter).
pub fn js_format(text: &str, o: &FormatOptions) -> Result<String, String> {
    const NONE: usize = usize::MAX;
    let toks = js_tokenize(text)?;
    let n = toks.len();
    if n == 0 {
        return Ok(String::new());
    }
    let roles = js_roles(&toks);

    // Bracket partners.
    let mut partner = vec![NONE; n];
    let mut stack: Vec<usize> = Vec::new();
    for (k, t) in toks.iter().enumerate() {
        if t.kind != Jk::Punct {
            continue;
        }
        match t.text {
            "(" | "[" | "{" => stack.push(k),
            ")" | "]" | "}" => {
                let Some(op) = stack.pop() else {
                    return Err(format!("Unmatched '{}'", t.text));
                };
                let want = match toks[op].text {
                    "(" => ")",
                    "[" => "]",
                    _ => "}",
                };
                if want != t.text {
                    return Err(format!("'{}' closed by '{}'", toks[op].text, t.text));
                }
                partner[op] = k;
                partner[k] = op;
            }
            _ => {}
        }
    }
    if let Some(&op) = stack.last() {
        return Err(format!("Unclosed '{}'", toks[op].text));
    }
    let mut prev_sig = vec![NONE; n];
    let mut last = NONE;
    for k in 0..n {
        prev_sig[k] = last;
        if !toks[k].is_comment() {
            last = k;
        }
    }
    let word_at = |k: usize, w: &str| k != NONE && toks[k].word(w) && roles[k] != Role::Operand;

    // Classify every opening bracket.
    let mut kind = vec![Bk::Paren; n];
    let mut ctrl = vec![false; n];
    let mut switch = vec![false; n];
    let mut do_block = vec![false; n];
    let mut force = vec![false; n];
    for k in 0..n {
        if partner[k] == NONE || partner[k] < k {
            continue;
        }
        let p = prev_sig[k];
        kind[k] = match toks[k].text {
            "(" => {
                ctrl[k] = ["if", "while", "for", "with"].iter().any(|w| word_at(p, w));
                Bk::Paren
            }
            "[" => {
                let index = p != NONE
                    && (matches!(roles[p], Role::Operand | Role::Close | Role::Postfix)
                        || (roles[p] == Role::Ctx && !toks[k].ws));
                if index { Bk::Index } else { Bk::Array }
            }
            _ => {
                if p == NONE {
                    Bk::Block
                } else {
                    let t = toks[p];
                    match t.kind {
                        Jk::Ident => {
                            if roles[p] != Role::Operand && JS_OBJ_AFTER_WORD.contains(&t.text) {
                                Bk::Object
                            } else if word_at(prev_sig[p], "enum") {
                                force[k] = true;
                                Bk::Object
                            } else {
                                do_block[k] = word_at(p, "do");
                                Bk::Block
                            }
                        }
                        Jk::Str => Bk::Block,
                        Jk::Punct => match t.text {
                            ")" => {
                                switch[k] = word_at(prev_sig[partner[p]], "switch");
                                Bk::Block
                            }
                            "=>" | ";" | "{" | "}" | "]" | ">" => Bk::Block,
                            ":" => {
                                // `case x: {` opens a block.
                                let mut j = prev_sig[p];
                                let mut case = false;
                                while j != NONE {
                                    let u = toks[j];
                                    if u.is(";") || u.is("{") || u.is("}") || u.is("?") {
                                        break;
                                    }
                                    if word_at(j, "case") || word_at(j, "default") {
                                        case = true;
                                        break;
                                    }
                                    j = prev_sig[j];
                                }
                                if case { Bk::Block } else { Bk::Object }
                            }
                            _ => Bk::Object,
                        },
                        _ => Bk::Object,
                    }
                }
            }
        };
    }

    // Prefix sums: tokens that force a list to break, and flat widths.
    let mut brk = vec![0usize; n + 1];
    let mut wsum = vec![0usize; n + 1];
    for k in 0..n {
        let t = toks[k];
        let forces = (k > 0 && t.nl > 0)
            || t.kind == Jk::LineComment
            || (t.is("{") && kind[k] == Bk::Block && partner[k] > k + 1);
        brk[k + 1] = brk[k] + usize::from(forces);
        wsum[k + 1] = wsum[k] + width(t.text) + 1;
    }

    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut col = 0usize;
    let mut frames = vec![Frame {
        kind: Bk::Block,
        broken: true,
        inner: 0,
        outer: 0,
        switch: false,
        ternary: 0,
    }];
    let mut line_level = 0usize;
    let mut force_nl = false;
    let mut after_block: Option<usize> = None;
    let mut last_ctrl_close = NONE;
    let mut prev_role = Role::Comment;
    let mut line_first = 0usize;

    for k in 0..n {
        let t = toks[k];
        let is_closer = partner[k] != NONE && partner[k] < k;
        let is_opener = partner[k] != NONE && partner[k] > k;
        let mut role = roles[k];
        {
            let top = frames.last_mut().unwrap();
            if role == Role::Colon && top.ternary > 0 {
                top.ternary -= 1;
                role = Role::Binary;
            }
        }
        let ps = prev_sig[k];
        let mut nl = 0usize;
        if k > 0 {
            let prev = toks[k - 1];
            let trailing = t.is_comment() && t.nl == 0 && prev.kind != Jk::LineComment;
            if t.nl > 0 {
                nl = if t.nl >= 2 { 2 } else { 1 };
            }
            if trailing {
                nl = 0;
            } else {
                if force_nl {
                    nl = nl.max(1);
                }
                if let Some(c) = after_block {
                    let joiner = t.word("else")
                        || t.word("catch")
                        || t.word("finally")
                        || (t.word("while") && do_block[partner[c]]);
                    let stays = joiner
                        || [")", "]", ",", ";", ".", "?.", "(", "++", "--", "=>"]
                            .iter()
                            .any(|p| t.is(p));
                    if !stays {
                        nl = nl.max(1);
                    } else if joiner && c == k - 1 {
                        nl = 0;
                    }
                }
                if is_closer && frames.last().unwrap().broken {
                    nl = nl.max(1);
                }
                if t.is("{")
                    && kind[k] == Bk::Block
                    && nl > 0
                    && ps == k - 1
                    && (ps == last_ctrl_close
                        || ["else", "try", "finally", "do"]
                            .iter()
                            .any(|w| word_at(ps, w))
                        || toks[ps].is("=>"))
                {
                    nl = 0;
                }
                if prev.kind == Jk::LineComment {
                    nl = nl.max(1);
                }
                if nl == 2 && (is_closer || (partner[k - 1] != NONE && partner[k - 1] > k - 1)) {
                    nl = 1;
                }
                force_nl = false;
                after_block = None;
            }
        }

        if nl > 0 {
            for _ in 0..nl {
                out.push('\n');
            }
            let top = frames.last().unwrap();
            let mut level = if is_closer { top.outer } else { top.inner };
            if !is_closer {
                if top.switch && !(t.word("case") || t.word("default")) {
                    level += 1;
                }
                let cont = ps != NONE
                    && (matches!(roles[ps], Role::Binary | Role::Dot)
                        || (roles[ps] == Role::Prefix && !toks[ps].is("@"))
                        || ps == last_ctrl_close
                        || word_at(ps, "else")
                        || t.is(".")
                        || t.is("?.")
                        || (role == Role::Binary && !t.is_comment()));
                if cont {
                    level += 1;
                }
            }
            line_level = level;
            line_first = k;
            out.push_str(&indent_str(o, level));
            col = indent_cols(o, level);
        } else if k > 0 {
            let prev = &toks[k - 1];
            if js_space(prev, prev_role, &t, role) || js_must_separate(prev.text, t.text) {
                out.push(' ');
                col += 1;
            }
        }

        // The token itself.
        let lines: Vec<&str> = t.text.split('\n').collect();
        if t.kind == Jk::BlockComment
            && lines.len() > 1
            && lines[1..].iter().all(|l| l.trim_start().starts_with('*'))
        {
            out.push_str(lines[0].trim_end());
            for l in &lines[1..] {
                out.push('\n');
                out.push_str(&indent_str(o, line_level));
                out.push(' ');
                out.push_str(l.trim());
            }
            col = indent_cols(o, line_level) + 1 + width(lines[lines.len() - 1].trim());
        } else {
            out.push_str(t.text);
            col = if lines.len() > 1 {
                width(lines[lines.len() - 1])
            } else {
                col + width(t.text)
            };
        }

        // State updates.
        if role == Role::Binary && t.is("?") {
            frames.last_mut().unwrap().ternary += 1;
        }
        if is_opener {
            let kd = kind[k];
            let p = partner[k];
            let broken = match kd {
                Bk::Block => p > k + 1,
                Bk::Object | Bk::Array => {
                    p > k + 1
                        && (force[k]
                            || brk[p + 1] - brk[k + 1] > 0
                            || col + (wsum[p + 1] - wsum[k + 1]) > o.print_width)
                }
                _ => false,
            };
            frames.push(Frame {
                kind: kd,
                broken,
                inner: line_level + 1,
                outer: line_level,
                switch: switch[k],
                ternary: 0,
            });
            if broken {
                force_nl = true;
            }
        } else if is_closer {
            let f = frames.pop().unwrap();
            if f.kind == Bk::Block && t.is("}") {
                after_block = Some(k);
            }
            if t.is(")") && ctrl[partner[k]] {
                last_ctrl_close = k;
            }
        }
        let top = frames.last().unwrap();
        if t.is(";") && top.kind != Bk::Paren && (top.kind == Bk::Block || top.broken) {
            force_nl = true;
        }
        if t.is(",") && top.broken && matches!(top.kind, Bk::Object | Bk::Array) {
            force_nl = true;
        }
        if t.kind == Jk::LineComment {
            force_nl = true;
        }
        // `case x:` / `default:` in a switch: the body starts on a new line.
        if role == Role::Colon
            && top.switch
            && (word_at(line_first, "case") || word_at(line_first, "default"))
            && !toks[k + 1..]
                .iter()
                .find(|u| !u.is_comment())
                .is_some_and(|u| u.is("{"))
        {
            force_nl = true;
        }
        prev_role = role;
    }
    out.push('\n');
    Ok(out)
}

// ------------------------------------------------------------------ HTML

const HTML_VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr", "keygen", "command",
];

const HTML_INLINE: &[&str] = &[
    "a", "abbr", "acronym", "audio", "b", "bdi", "bdo", "big", "br", "button", "canvas", "cite",
    "code", "data", "del", "dfn", "em", "font", "i", "iframe", "img", "input", "ins", "kbd",
    "label", "mark", "meter", "noscript", "object", "output", "picture", "progress", "q", "s",
    "samp", "select", "small", "span", "strike", "strong", "sub", "sup", "textarea", "time", "tt",
    "u", "var", "video", "wbr", "math",
];

/// Elements whose start tag implicitly closes an open `<p>`.
const HTML_CLOSES_P: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "ul",
];

#[derive(Debug, Clone)]
enum Hn {
    Text(String),
    /// Comment, doctype, CDATA, processing instruction or a stray end tag.
    Raw(String),
    Elem(Box<HElem>),
}

#[derive(Debug, Clone)]
struct HElem {
    name: String,
    lname: String,
    attrs: Vec<String>,
    self_closing: bool,
    children: Vec<Hn>,
    closed: bool,
    /// Verbatim contents of `<pre>`, `<textarea>`, `<script>` and `<style>`.
    raw: Option<String>,
}

impl HElem {
    fn attr(&self, name: &str) -> Option<String> {
        self.attrs.iter().find_map(|a| {
            let (k, v) = a.split_once('=')?;
            k.eq_ignore_ascii_case(name)
                .then(|| v.trim_matches(['"', '\'']).to_ascii_lowercase())
        })
    }
}

struct HtmlParser<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

impl HtmlParser<'_> {
    fn err(&self, msg: &str, at: usize) -> String {
        format!("{msg} at {}", location(self.s, at))
    }

    fn find_from(&self, from: usize, pat: &str) -> Option<usize> {
        self.s[from..].find(pat).map(|p| from + p)
    }

    /// Case-insensitive search for `</name`.
    fn find_close(&self, from: usize, lname: &str) -> Option<usize> {
        let pat = format!("</{lname}");
        let lower = self.s[from..].to_ascii_lowercase();
        let mut off = 0;
        while let Some(p) = lower[off..].find(&pat) {
            let at = off + p;
            let after = lower.as_bytes().get(at + pat.len()).copied();
            if after.is_none_or(|c| c == b'>' || c.is_ascii_whitespace() || c == b'/') {
                return Some(from + at);
            }
            off = at + 1;
        }
        None
    }

    fn name_end(&self, from: usize) -> usize {
        let mut j = from;
        while j < self.b.len()
            && !self.b[j].is_ascii_whitespace()
            && !matches!(self.b[j], b'>' | b'/' | b'<')
        {
            j += 1;
        }
        j
    }

    /// Parses a start tag at `self.i` (the `<`).
    fn start_tag(&mut self) -> Result<HElem, String> {
        let st = self.i;
        let ne = self.name_end(st + 1);
        let name = self.s[st + 1..ne].to_string();
        let mut j = ne;
        let mut attrs = Vec::new();
        let mut self_closing = false;
        loop {
            while j < self.b.len() && self.b[j].is_ascii_whitespace() {
                j += 1;
            }
            match self.b.get(j) {
                None => return Err(self.err("Unterminated tag", st)),
                Some(b'>') => {
                    j += 1;
                    break;
                }
                Some(b'/') if self.b.get(j + 1) == Some(&b'>') => {
                    self_closing = true;
                    j += 2;
                    break;
                }
                _ => {}
            }
            let an = j;
            j += 1;
            while j < self.b.len()
                && !self.b[j].is_ascii_whitespace()
                && !matches!(self.b[j], b'>' | b'=')
                && !(self.b[j] == b'/' && self.b.get(j + 1) == Some(&b'>'))
            {
                j += 1;
            }
            let mut attr = self.s[an..j].to_string();
            let mut k = j;
            while k < self.b.len() && self.b[k].is_ascii_whitespace() {
                k += 1;
            }
            if self.b.get(k) == Some(&b'=') {
                k += 1;
                while k < self.b.len() && self.b[k].is_ascii_whitespace() {
                    k += 1;
                }
                let vs = k;
                match self.b.get(k) {
                    Some(&q @ (b'"' | b'\'')) => {
                        let e = self.s[k + 1..]
                            .find(q as char)
                            .ok_or_else(|| self.err("Unterminated attribute value", k))?;
                        k = k + 1 + e + 1;
                    }
                    _ => {
                        while k < self.b.len()
                            && !self.b[k].is_ascii_whitespace()
                            && self.b[k] != b'>'
                        {
                            k += 1;
                        }
                    }
                }
                attr.push('=');
                attr.push_str(&self.s[vs..k]);
                j = k;
            }
            attrs.push(attr);
        }
        self.i = j;
        let lname = name.to_ascii_lowercase();
        Ok(HElem {
            name,
            lname,
            attrs,
            self_closing,
            children: Vec::new(),
            closed: false,
            raw: None,
        })
    }

    fn parse(&mut self) -> Result<Vec<Hn>, String> {
        let mut root: Vec<Hn> = Vec::new();
        let mut stack: Vec<HElem> = Vec::new();

        fn add(stack: &mut [HElem], root: &mut Vec<Hn>, n: Hn) {
            match stack.last_mut() {
                Some(e) => e.children.push(n),
                None => root.push(n),
            }
        }
        // Pops the top `count` elements; the last one popped is `closed`.
        fn pop(stack: &mut Vec<HElem>, root: &mut Vec<Hn>, count: usize, closed_last: bool) {
            for c in 0..count {
                let mut e = stack.pop().unwrap();
                e.closed = closed_last && c + 1 == count;
                add(stack, root, Hn::Elem(Box::new(e)));
            }
        }
        // Implicitly closes `targets` unless a `boundary` element comes first.
        fn implied(
            stack: &mut Vec<HElem>,
            root: &mut Vec<Hn>,
            targets: &[&str],
            boundary: &[&str],
        ) {
            let mut count = None;
            for (depth, e) in stack.iter().rev().enumerate() {
                if targets.contains(&e.lname.as_str()) {
                    count = Some(depth + 1);
                    break;
                }
                if boundary.contains(&e.lname.as_str()) {
                    break;
                }
            }
            if let Some(c) = count {
                pop(stack, root, c, false);
            }
        }

        let mut text_start = 0;
        while self.i < self.b.len() {
            let st = self.i;
            let rest = &self.s[st..];
            let next = self.b.get(st + 1).copied().unwrap_or(0);
            let markup = self.b[st] == b'<'
                && (next.is_ascii_alphabetic()
                    || next == b'!'
                    || next == b'?'
                    || (next == b'/' && self.b.get(st + 2).is_some_and(u8::is_ascii_alphabetic)));
            if !markup {
                self.i += 1;
                continue;
            }
            if text_start < st {
                add(
                    &mut stack,
                    &mut root,
                    Hn::Text(self.s[text_start..st].to_string()),
                );
            }
            if rest.starts_with("<!--") {
                let e = self
                    .find_from(st + 4, "-->")
                    .ok_or_else(|| self.err("Unterminated comment", st))?;
                self.i = e + 3;
                add(
                    &mut stack,
                    &mut root,
                    Hn::Raw(self.s[st..self.i].to_string()),
                );
            } else if rest.starts_with("<![CDATA[") {
                let e = self
                    .find_from(st, "]]>")
                    .ok_or_else(|| self.err("Unterminated CDATA", st))?;
                self.i = e + 3;
                add(
                    &mut stack,
                    &mut root,
                    Hn::Raw(self.s[st..self.i].to_string()),
                );
            } else if next == b'!' || next == b'?' {
                let e = self
                    .find_from(st, ">")
                    .ok_or_else(|| self.err("Unterminated declaration", st))?;
                self.i = e + 1;
                add(
                    &mut stack,
                    &mut root,
                    Hn::Raw(self.s[st..self.i].to_string()),
                );
            } else if next == b'/' {
                let e = self
                    .find_from(st, ">")
                    .ok_or_else(|| self.err("Unterminated end tag", st))?;
                let lname = self.s[st + 2..self.name_end(st + 2)].to_ascii_lowercase();
                self.i = e + 1;
                match stack.iter().rposition(|x| x.lname == lname) {
                    Some(pos) => {
                        let count = stack.len() - pos;
                        pop(&mut stack, &mut root, count, true);
                    }
                    None => add(
                        &mut stack,
                        &mut root,
                        Hn::Raw(self.s[st..self.i].to_string()),
                    ),
                }
            } else {
                let mut e = self.start_tag()?;
                match e.lname.as_str() {
                    "li" => implied(&mut stack, &mut root, &["li"], &["ul", "ol", "menu"]),
                    "dt" | "dd" => implied(&mut stack, &mut root, &["dt", "dd"], &["dl"]),
                    "tr" => implied(
                        &mut stack,
                        &mut root,
                        &["tr"],
                        &["table", "thead", "tbody", "tfoot"],
                    ),
                    "td" | "th" => implied(&mut stack, &mut root, &["td", "th"], &["tr", "table"]),
                    "thead" | "tbody" | "tfoot" => {
                        implied(
                            &mut stack,
                            &mut root,
                            &["thead", "tbody", "tfoot"],
                            &["table"],
                        );
                    }
                    "option" => implied(
                        &mut stack,
                        &mut root,
                        &["option"],
                        &["select", "datalist", "optgroup"],
                    ),
                    "optgroup" => implied(&mut stack, &mut root, &["optgroup"], &["select"]),
                    _ => {}
                }
                if HTML_CLOSES_P.contains(&e.lname.as_str()) {
                    // Close an open <p>, looking only through inline elements.
                    let pos = stack
                        .iter()
                        .rposition(|x| x.lname == "p" || !HTML_INLINE.contains(&x.lname.as_str()));
                    if let Some(pos) = pos
                        && stack[pos].lname == "p"
                    {
                        let count = stack.len() - pos;
                        pop(&mut stack, &mut root, count, false);
                    }
                }
                if e.self_closing || HTML_VOID.contains(&e.lname.as_str()) {
                    e.closed = true;
                    add(&mut stack, &mut root, Hn::Elem(Box::new(e)));
                } else if matches!(e.lname.as_str(), "script" | "style" | "pre" | "textarea") {
                    let close = self
                        .find_close(self.i, &e.lname)
                        .ok_or_else(|| self.err(&format!("Unclosed <{}>", e.lname), st))?;
                    e.raw = Some(self.s[self.i..close].to_string());
                    let gt = self
                        .find_from(close, ">")
                        .ok_or_else(|| self.err("Unterminated end tag", close))?;
                    self.i = gt + 1;
                    e.closed = true;
                    add(&mut stack, &mut root, Hn::Elem(Box::new(e)));
                } else {
                    stack.push(e);
                }
            }
            text_start = self.i;
        }
        if text_start < self.b.len() {
            add(
                &mut stack,
                &mut root,
                Hn::Text(self.s[text_start..].to_string()),
            );
        }
        let n = stack.len();
        pop(&mut stack, &mut root, n, false);
        Ok(root)
    }
}

/// One piece of inline flow: text/markup, or a whitespace run.
#[derive(Debug, Clone)]
enum Atom {
    Word(String),
    /// A whitespace run with this many newlines.
    Space(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hc {
    Block,
    Inline,
    Ws,
}

fn html_is_inline(n: &Hn) -> bool {
    match n {
        Hn::Text(_) | Hn::Raw(_) => true,
        Hn::Elem(e) => {
            (HTML_INLINE.contains(&e.lname.as_str()) || e.lname.contains('-'))
                && e.children.iter().all(html_is_inline)
        }
    }
}

fn html_is_ws(n: &Hn) -> bool {
    matches!(n, Hn::Text(t) if t.trim().is_empty())
}

/// Classifies children; comments and declarations are inline only when glued
/// (no whitespace) to neighbouring inline content.
fn html_classify(children: &[Hn]) -> Vec<Hc> {
    let mut c: Vec<Hc> = children
        .iter()
        .map(|n| match n {
            _ if html_is_ws(n) => Hc::Ws,
            Hn::Raw(_) => Hc::Block,
            _ if html_is_inline(n) => Hc::Inline,
            _ => Hc::Block,
        })
        .collect();
    for i in 0..children.len() {
        if let Hn::Raw(_) = children[i] {
            let glued_prev = i > 0
                && c[i - 1] == Hc::Inline
                && !matches!(&children[i - 1], Hn::Text(t) if t.ends_with(|ch: char| ch.is_whitespace()));
            let glued_next = i + 1 < children.len()
                && html_is_inline(&children[i + 1])
                && !html_is_ws(&children[i + 1])
                && !matches!(&children[i + 1], Hn::Raw(_))
                && !matches!(&children[i + 1], Hn::Text(t) if t.starts_with(|ch: char| ch.is_whitespace()));
            if glued_prev || glued_next {
                c[i] = Hc::Inline;
            }
        }
    }
    c
}

fn html_open_tag(e: &HElem) -> String {
    let mut s = format!("<{}", e.name);
    for a in &e.attrs {
        s.push(' ');
        s.push_str(a);
    }
    s.push_str(if e.self_closing { " />" } else { ">" });
    s
}

fn html_close_tag(e: &HElem) -> String {
    if e.closed && !e.self_closing && !HTML_VOID.contains(&e.lname.as_str()) {
        format!("</{}>", e.name)
    } else {
        String::new()
    }
}

fn html_flatten(n: &Hn, atoms: &mut Vec<Atom>) {
    match n {
        Hn::Text(t) => {
            let mut word = String::new();
            let mut nls: Option<usize> = None;
            for ch in t.chars() {
                if ch.is_whitespace() && ch != '\u{a0}' {
                    if !word.is_empty() {
                        atoms.push(Atom::Word(std::mem::take(&mut word)));
                    }
                    *nls.get_or_insert(0) += usize::from(ch == '\n');
                } else {
                    if let Some(k) = nls.take() {
                        atoms.push(Atom::Space(k));
                    }
                    word.push(ch);
                }
            }
            if !word.is_empty() {
                atoms.push(Atom::Word(word));
            }
            if let Some(k) = nls {
                atoms.push(Atom::Space(k));
            }
        }
        Hn::Raw(r) => atoms.push(Atom::Word(r.clone())),
        Hn::Elem(e) => {
            atoms.push(Atom::Word(html_open_tag(e)));
            if let Some(raw) = &e.raw {
                atoms.push(Atom::Word(raw.clone()));
            }
            for c in &e.children {
                html_flatten(c, atoms);
            }
            let close = html_close_tag(e);
            if !close.is_empty() {
                atoms.push(Atom::Word(close));
            }
        }
    }
}

/// Renders flow atoms as lines: words glued unless separated by whitespace,
/// line breaks kept where the input had them (at most one blank line).
fn html_flow_lines(atoms: &[Atom]) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut pending: Option<usize> = None;
    for a in atoms {
        match a {
            Atom::Space(k) => pending = Some(pending.unwrap_or(0).max(*k)),
            Atom::Word(w) => {
                if let Some(k) = pending.take() {
                    let cur = lines.last().unwrap();
                    if cur.is_empty() {
                        // Leading whitespace of the run.
                    } else if k == 0 {
                        lines.last_mut().unwrap().push(' ');
                    } else {
                        if k >= 2 {
                            lines.push(String::new());
                        }
                        lines.push(String::new());
                    }
                }
                lines.last_mut().unwrap().push_str(w);
            }
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

fn html_push_lines(out: &mut Vec<String>, ind: &str, lines: &[String]) {
    for l in lines {
        if l.is_empty() {
            out.push(String::new());
        } else {
            out.push(format!("{ind}{l}"));
        }
    }
}

/// Re-formats `<script>` / `<style>` contents; `None` keeps them verbatim.
fn html_embedded(e: &HElem, raw: &str, o: &FormatOptions) -> Option<String> {
    match e.lname.as_str() {
        "style" => {
            let f = css_format(raw, o).ok()?;
            (!f.contains("\\\n")).then_some(f)
        }
        "script" => {
            let ty = e.attr("type").unwrap_or_default();
            if ty.is_empty()
                || ty.contains("javascript")
                || ty.contains("ecmascript")
                || ty == "module"
                || ty.contains("typescript")
            {
                if js_has_multiline_literal(raw) {
                    return None;
                }
                js_format(raw, o).ok()
            } else if ty.contains("json") || ty == "importmap" {
                crate::formatters::json_format(raw, o).ok()
            } else {
                None
            }
        }
        _ => None,
    }
}

fn html_open_block_tag(e: &HElem, level: usize, o: &FormatOptions) -> String {
    let flat = html_open_tag(e);
    if e.attrs.len() <= 1 || indent_cols(o, level) + width(&flat) <= o.print_width {
        return flat;
    }
    let mut s = format!("<{}", e.name);
    for a in &e.attrs {
        s.push('\n');
        s.push_str(&indent_str(o, level + 1));
        s.push_str(a);
    }
    let end = if e.self_closing { "/>" } else { ">" };
    if o.bracket_same_line {
        if e.self_closing {
            s.push(' ');
        }
        s.push_str(end);
    } else {
        s.push('\n');
        s.push_str(&indent_str(o, level));
        s.push_str(end);
    }
    s
}

fn html_print_elem(e: &HElem, level: usize, o: &FormatOptions, out: &mut Vec<String>) {
    let ind = indent_str(o, level);
    let open = html_open_block_tag(e, level, o);
    let close = html_close_tag(e);
    if let Some(raw) = &e.raw {
        if raw.trim().is_empty() && e.lname != "pre" && e.lname != "textarea" {
            out.push(format!("{ind}{open}{close}"));
            return;
        }
        if let Some(f) = html_embedded(e, raw, o) {
            out.push(format!("{ind}{open}"));
            let inner = indent_str(o, level + 1);
            for l in f.lines() {
                out.push(if l.is_empty() {
                    String::new()
                } else {
                    format!("{inner}{l}")
                });
            }
            out.push(format!("{ind}{close}"));
        } else {
            out.push(format!("{ind}{open}{raw}{close}"));
        }
        return;
    }
    if e.children.iter().all(html_is_ws) {
        out.push(format!("{ind}{open}{close}"));
        return;
    }
    if e.children.iter().all(html_is_inline) {
        let mut atoms = Vec::new();
        for c in &e.children {
            html_flatten(c, &mut atoms);
        }
        let lines = html_flow_lines(&atoms);
        let joined = lines
            .iter()
            .filter(|l| !l.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        if !open.contains('\n')
            && !joined.contains('\n')
            && indent_cols(o, level) + width(&open) + width(&joined) + width(&close)
                <= o.print_width
        {
            out.push(format!("{ind}{open}{joined}{close}"));
            return;
        }
        out.push(format!("{ind}{open}"));
        html_push_lines(out, &indent_str(o, level + 1), &lines);
        if !close.is_empty() {
            out.push(format!("{ind}{close}"));
        }
        return;
    }
    out.push(format!("{ind}{open}"));
    html_print_children(&e.children, level + 1, o, out);
    if !close.is_empty() {
        out.push(format!("{ind}{close}"));
    }
}

fn html_print_children(children: &[Hn], level: usize, o: &FormatOptions, out: &mut Vec<String>) {
    let classes = html_classify(children);
    let start = out.len();
    let mut blank = false;
    let mut i = 0;
    while i < children.len() {
        match classes[i] {
            Hc::Ws => {
                if let Hn::Text(t) = &children[i]
                    && t.matches('\n').count() >= 2
                {
                    blank = true;
                }
                i += 1;
            }
            Hc::Block => {
                if blank && out.len() > start {
                    out.push(String::new());
                }
                blank = false;
                let ind = indent_str(o, level);
                match &children[i] {
                    Hn::Elem(e) => html_print_elem(e, level, o, out),
                    Hn::Raw(r) => out.push(format!("{ind}{r}")),
                    Hn::Text(t) => out.push(format!("{ind}{}", t.trim())),
                }
                i += 1;
            }
            Hc::Inline => {
                if blank && out.len() > start {
                    out.push(String::new());
                }
                blank = false;
                let mut atoms = Vec::new();
                let mut j = i;
                while j < children.len() && classes[j] != Hc::Block {
                    html_flatten(&children[j], &mut atoms);
                    j += 1;
                }
                // Trailing whitespace of the run may hold a blank line.
                if let Some(Atom::Space(k)) = atoms.last()
                    && *k >= 2
                {
                    blank = true;
                }
                html_push_lines(out, &indent_str(o, level), &html_flow_lines(&atoms));
                i = j;
            }
        }
    }
}

/// Pretty-prints HTML: block elements indented, inline elements and text kept
/// flowing, `<pre>` / `<textarea>` verbatim, `<script>` / `<style>` re-indented.
pub fn html_format(text: &str, o: &FormatOptions) -> Result<String, String> {
    let nodes = HtmlParser {
        s: text,
        b: text.as_bytes(),
        i: 0,
    }
    .parse()?;
    let mut out = Vec::new();
    html_print_children(&nodes, 0, o, &mut out);
    Ok(finish_lines(&out))
}

// ------------------------------------------------------------------ Markdown

/// Leading indentation width (tabs advance to the next multiple of 4).
fn md_indent(line: &str) -> usize {
    let mut w = 0;
    for c in line.chars() {
        match c {
            ' ' => w += 1,
            '\t' => w += 4 - w % 4,
            _ => break,
        }
    }
    w
}

fn md_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// Opening code fence: (fence char, length).
fn md_fence(line: &str) -> Option<(char, usize)> {
    if md_indent(line) > 3 {
        return None;
    }
    let t = line.trim_start();
    let ch = t.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let len = t.chars().take_while(|&c| c == ch).count();
    if len < 3 || (ch == '`' && t[len..].contains('`')) {
        return None;
    }
    Some((ch, len))
}

fn md_fence_close(line: &str, ch: char, len: usize) -> bool {
    if md_indent(line) > 3 {
        return false;
    }
    let t = line.trim();
    t.chars().take_while(|&c| c == ch).count() >= len && t.chars().all(|c| c == ch)
}

/// ATX heading → normalised text (`## Title`).
fn md_heading(line: &str) -> Option<String> {
    if md_indent(line) > 3 {
        return None;
    }
    let t = line.trim_start();
    let hashes = t.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &t[hashes..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let content = rest.trim();
    Some(if content.is_empty() {
        "#".repeat(hashes)
    } else {
        format!("{} {content}", "#".repeat(hashes))
    })
}

fn md_hr(line: &str) -> bool {
    if md_indent(line) > 3 {
        return false;
    }
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    let Some(ch) = t.chars().next() else {
        return false;
    };
    matches!(ch, '-' | '*' | '_') && t.len() >= 3 && t.chars().all(|c| c == ch)
}

fn md_setext_underline(line: &str) -> bool {
    if md_indent(line) > 3 {
        return false;
    }
    let t = line.trim();
    !t.is_empty() && (t.chars().all(|c| c == '=') || t.chars().all(|c| c == '-'))
}

fn md_quote(line: &str) -> bool {
    md_indent(line) <= 3 && line.trim_start().starts_with('>')
}

/// HTML block start: returns what ends the block.
fn md_html_start(line: &str) -> Option<&'static str> {
    if md_indent(line) > 3 {
        return None;
    }
    let t = line.trim_start();
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("<!--") {
        return Some("-->");
    }
    for (tag, end) in [
        ("<script", "</script>"),
        ("<pre", "</pre>"),
        ("<style", "</style>"),
        ("<textarea", "</textarea>"),
    ] {
        if lower.starts_with(tag)
            && lower[tag.len()..].starts_with(|c: char| c == '>' || c.is_whitespace())
        {
            return Some(end);
        }
    }
    let b = t.as_bytes();
    let tag_start = b.first() == Some(&b'<')
        && b.get(1)
            .is_some_and(|&c| c.is_ascii_alphabetic() || c == b'/' || c == b'!' || c == b'?');
    // Autolinks like <https://x> and <a@b.c> are inline, not HTML blocks.
    if !tag_start || t.contains("://") {
        return None;
    }
    let after_lt = &t[1..];
    if !after_lt.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/') {
        return Some("");
    }
    let tail = after_lt.trim_start_matches('/');
    let name_len = tail
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(tail.len());
    if name_len > 0
        && tail[name_len..]
            .chars()
            .next()
            .is_none_or(|c| c == '>' || c == '/' || c.is_whitespace())
    {
        return Some("");
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Align {
    None,
    Left,
    Right,
    Center,
}

/// Splits a table row on unescaped pipes (outer pipes optional).
fn md_split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with('|') && !t.ends_with("\\|") {
        &t[..t.len() - 1]
    } else {
        t
    };
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut escaped = false;
    for c in t.chars() {
        if c == '|' && !escaped {
            cells.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
        escaped = c == '\\' && !escaped;
    }
    cells.push(cur.trim().to_string());
    cells
}

fn md_table_delim(line: &str) -> Option<Vec<Align>> {
    if !line.contains('-') || md_indent(line) > 3 {
        return None;
    }
    let cells = md_split_row(line);
    cells
        .iter()
        .map(|c| {
            let l = c.starts_with(':');
            let r = c.ends_with(':');
            let core = c.trim_matches(':');
            if core.is_empty() || !core.chars().all(|ch| ch == '-') {
                return None;
            }
            Some(match (l, r) {
                (true, true) => Align::Center,
                (true, false) => Align::Left,
                (false, true) => Align::Right,
                _ => Align::None,
            })
        })
        .collect()
}

fn md_table(rows: &[Vec<String>], aligns: &[Align]) -> Vec<String> {
    let cols = rows
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .max(aligns.len());
    let mut widths = vec![3usize; cols];
    for r in rows {
        for (k, c) in r.iter().enumerate() {
            widths[k] = widths[k].max(width(c));
        }
    }
    let align = |k: usize| aligns.get(k).copied().unwrap_or(Align::None);
    let fmt_row = |r: &Vec<String>| {
        let cells: Vec<String> = (0..cols)
            .map(|k| {
                let c = r.get(k).map_or("", String::as_str);
                let pad = widths[k] - width(c);
                match align(k) {
                    Align::Right => format!("{}{c}", " ".repeat(pad)),
                    Align::Center => {
                        format!("{}{c}{}", " ".repeat(pad / 2), " ".repeat(pad - pad / 2))
                    }
                    _ => format!("{c}{}", " ".repeat(pad)),
                }
            })
            .collect();
        format!("| {} |", cells.join(" | "))
    };
    let mut out = vec![fmt_row(&rows[0])];
    let delim: Vec<String> = (0..cols)
        .map(|k| {
            let w = widths[k];
            match align(k) {
                Align::None => "-".repeat(w),
                Align::Left => format!(":{}", "-".repeat(w - 1)),
                Align::Right => format!("{}:", "-".repeat(w - 1)),
                Align::Center => format!(":{}:", "-".repeat(w - 2)),
            }
        })
        .collect();
    out.push(format!("| {} |", delim.join(" | ")));
    for r in &rows[1..] {
        out.push(fmt_row(r));
    }
    out
}

struct MdItem {
    indent: usize,
    ordered: Option<(u64, char)>,
    content: String,
    /// Column where the item's content starts in the input.
    content_col: usize,
}

fn md_list_item(line: &str) -> Option<MdItem> {
    if md_hr(line) {
        return None;
    }
    let indent = md_indent(line);
    let t = line.trim_start();
    let (marker_len, ordered) = if t.starts_with(['-', '*', '+']) {
        (1, None)
    } else {
        let digits = t.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let d = *t.as_bytes().get(digits)?;
        if d != b'.' && d != b')' {
            return None;
        }
        (digits + 1, Some((t[..digits].parse().ok()?, d as char)))
    };
    let rest = &t[marker_len..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let content = rest.trim_start();
    let spaces = rest.len() - content.len();
    let gap = if content.is_empty() || spaces > 4 {
        1
    } else {
        spaces
    };
    Some(MdItem {
        indent,
        ordered,
        content: content.to_string(),
        content_col: indent + marker_len + gap,
    })
}

/// Can this line interrupt a paragraph?
fn md_interrupts(line: &str) -> bool {
    md_fence(line).is_some()
        || md_heading(line).is_some()
        || md_hr(line)
        || md_quote(line)
        || md_list_item(line).is_some_and(|it| {
            md_indent(line) <= 3
                && !it.content.trim().is_empty()
                && it.ordered.is_none_or(|(n, _)| n == 1)
        })
}

fn md_starts_block(line: &str) -> bool {
    md_fence(line).is_some()
        || md_heading(line).is_some()
        || md_hr(line)
        || md_quote(line)
        || md_html_start(line).is_some()
}

fn md_hard_break(line: &str) -> bool {
    line.ends_with("  ") && !md_blank(line)
}

/// Text lines: trailing whitespace removed, two-space hard breaks kept
/// where another line of the same paragraph follows.
fn md_text_lines(lines: &[(String, bool, bool)]) -> Vec<String> {
    // (text, had hard break, is a continuation of the previous line)
    let mut out = Vec::with_capacity(lines.len());
    for (k, (l, hard, _)) in lines.iter().enumerate() {
        let next_cont = lines
            .get(k + 1)
            .is_some_and(|(n, _, cont)| *cont && !n.trim().is_empty());
        let mut s = l.trim_end().to_string();
        if *hard && next_cont {
            s.push_str("  ");
        }
        out.push(s);
    }
    out
}

struct MdLevel {
    marker_indent: usize,
    content_col: usize,
    out_content: usize,
    ordered: bool,
    next: u64,
}

/// Re-indents a list block; `None` when it holds constructs (code fences,
/// deeply indented code) whose indentation must not change.
fn md_list(lines: &[&str]) -> Option<Vec<String>> {
    if lines.iter().any(|l| md_fence(l.trim_start()).is_some()) {
        return None;
    }
    let mut stack: Vec<MdLevel> = Vec::new();
    // (text, hard break, continuation-of-previous)
    let mut out: Vec<(String, bool, bool)> = Vec::new();
    let mut prev_blank = false;
    for l in lines {
        if md_blank(l) {
            if !prev_blank {
                out.push((String::new(), false, false));
            }
            prev_blank = true;
            continue;
        }
        let hard = md_hard_break(l);
        if let Some(it) = md_list_item(l) {
            while let Some(top) = stack.last() {
                if it.indent >= top.content_col || it.indent >= top.marker_indent {
                    break;
                }
                stack.pop();
            }
            let child = stack.last().is_none_or(|top| it.indent >= top.content_col);
            if !child {
                let top = stack.last_mut().unwrap();
                if top.ordered != it.ordered.is_some() {
                    top.ordered = it.ordered.is_some();
                    top.next = it.ordered.map_or(1, |(n, _)| n);
                }
                top.marker_indent = it.indent;
                top.content_col = it.content_col;
            } else {
                let out_indent = stack.last().map_or(0, |t| t.out_content);
                stack.push(MdLevel {
                    marker_indent: it.indent,
                    content_col: it.content_col,
                    out_content: out_indent,
                    ordered: it.ordered.is_some(),
                    next: it.ordered.map_or(1, |(n, _)| n),
                });
            }
            let depth = stack.len();
            let parent_out = if depth >= 2 {
                stack[depth - 2].out_content
            } else {
                0
            };
            let top = stack.last_mut().unwrap();
            let marker = match it.ordered {
                Some((_, d)) => {
                    let m = format!("{}{d}", top.next);
                    top.next += 1;
                    m
                }
                None => "-".to_string(),
            };
            top.out_content = parent_out + marker.len() + 1;
            let text = if it.content.trim().is_empty() {
                format!("{}{marker}", " ".repeat(parent_out))
            } else {
                format!("{}{marker} {}", " ".repeat(parent_out), it.content)
            };
            out.push((text, hard, false));
        } else {
            let w = md_indent(l);
            // Deepest open item this line belongs to (lazy lines go to the last).
            let pos = stack.iter().rposition(|lv| w >= lv.content_col);
            let target = match (pos, prev_blank) {
                (Some(p), true) => {
                    stack.truncate(p + 1);
                    p
                }
                (_, false) if !stack.is_empty() => stack.len() - 1,
                _ => return None,
            };
            if w >= stack[target].content_col + 4 {
                return None;
            }
            let text = format!(
                "{}{}",
                " ".repeat(stack[target].out_content),
                l.trim_start()
            );
            out.push((text, hard, !prev_blank));
        }
        prev_blank = false;
    }
    while out.last().is_some_and(|(t, _, _)| t.is_empty()) {
        out.pop();
    }
    Some(md_text_lines(&out))
}

/// Pretty-prints Markdown (see the module docs for what changes).
pub fn markdown_format(text: &str, _o: &FormatOptions) -> Result<String, String> {
    let lines: Vec<&str> = text.lines().collect();
    let n = lines.len();
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let verbatim = |ls: &[&str]| ls.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut i = 0;
    // YAML front matter.
    if lines.first().is_some_and(|l| l.trim_end() == "---")
        && let Some(e) = lines[1..]
            .iter()
            .position(|l| matches!(l.trim_end(), "---" | "..."))
    {
        blocks.push(verbatim(&lines[..e + 2]));
        i = e + 2;
    }
    while i < n {
        let line = lines[i];
        if md_blank(line) {
            i += 1;
            continue;
        }
        if let Some((ch, len)) = md_fence(line) {
            let mut j = i + 1;
            while j < n && !md_fence_close(lines[j], ch, len) {
                j += 1;
            }
            let end = (j + 1).min(n);
            blocks.push(verbatim(&lines[i..end]));
            i = end;
            continue;
        }
        if md_indent(line) >= 4 {
            let mut j = i + 1;
            while j < n && (md_blank(lines[j]) || md_indent(lines[j]) >= 4) {
                j += 1;
            }
            while j > i + 1 && md_blank(lines[j - 1]) {
                j -= 1;
            }
            blocks.push(verbatim(&lines[i..j]));
            i = j;
            continue;
        }
        if let Some(h) = md_heading(line) {
            blocks.push(vec![h]);
            i += 1;
            continue;
        }
        if md_hr(line) {
            blocks.push(vec![line.trim().to_string()]);
            i += 1;
            continue;
        }
        if md_quote(line) {
            let mut j = i + 1;
            while j < n && !md_blank(lines[j]) {
                j += 1;
            }
            blocks.push(verbatim(&lines[i..j]));
            i = j;
            continue;
        }
        if let Some(end) = md_html_start(line) {
            let mut j = i;
            if end.is_empty() {
                while j < n && !md_blank(lines[j]) {
                    j += 1;
                }
            } else {
                while j < n && !lines[j].to_ascii_lowercase().contains(end) {
                    j += 1;
                }
                j = (j + 1).min(n);
            }
            blocks.push(verbatim(&lines[i..j]));
            i = j;
            continue;
        }
        if i + 1 < n
            && line.contains('|')
            && let Some(aligns) = md_table_delim(lines[i + 1])
            && md_split_row(line).len() == aligns.len()
        {
            let mut rows = vec![md_split_row(line)];
            let mut j = i + 2;
            while j < n && !md_blank(lines[j]) && !md_starts_block(lines[j]) {
                rows.push(md_split_row(lines[j]));
                j += 1;
            }
            blocks.push(md_table(&rows, &aligns));
            i = j;
            continue;
        }
        if let Some(first) = md_list_item(line) {
            let base = first.content_col;
            let mut j = i + 1;
            let mut last = i;
            while j < n {
                let l = lines[j];
                if md_blank(l) {
                    j += 1;
                    continue;
                }
                let prev_blank = md_blank(lines[j - 1]);
                let is_item = md_list_item(l).is_some_and(|it| it.indent < base + 4 || !prev_blank);
                if is_item || md_indent(l) >= base || (!prev_blank && !md_starts_block(l)) {
                    last = j;
                    j += 1;
                } else {
                    break;
                }
            }
            let block = &lines[i..=last];
            blocks.push(md_list(block).unwrap_or_else(|| verbatim(block)));
            i = last + 1;
            continue;
        }
        // Paragraph (possibly with a setext underline).
        let mut j = i + 1;
        while j < n && !md_blank(lines[j]) {
            if md_setext_underline(lines[j]) {
                j += 1;
                break;
            }
            if md_interrupts(lines[j]) {
                break;
            }
            j += 1;
        }
        let para: Vec<(String, bool, bool)> = lines[i..j]
            .iter()
            .enumerate()
            .map(|(k, l)| (l.to_string(), md_hard_break(l), k > 0))
            .collect();
        blocks.push(md_text_lines(&para));
        i = j;
    }
    let mut out: Vec<String> = Vec::new();
    for b in blocks {
        if !out.is_empty() {
            out.push(String::new());
        }
        out.extend(b);
    }
    Ok(finish_lines(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o() -> FormatOptions {
        FormatOptions::default()
    }

    fn strip_ws(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Token sequence ignoring whitespace (comment text compared without
    /// whitespace, since doc comments are re-indented).
    fn js_toks(s: &str) -> Vec<(Jk, String)> {
        js_tokenize(s)
            .unwrap()
            .iter()
            .map(|t| {
                let text = if t.is_comment() {
                    strip_ws(t.text)
                } else {
                    t.text.to_string()
                };
                (t.kind, text)
            })
            .collect()
    }

    /// Formats JS/TS and asserts: same tokens, idempotent. Returns the output.
    fn js(src: &str) -> String {
        js_with(src, &o())
    }

    fn js_with(src: &str, opts: &FormatOptions) -> String {
        let out = js_format(src, opts).unwrap_or_else(|e| panic!("{e}\n--- input:\n{src}"));
        assert_eq!(
            js_toks(src),
            js_toks(&out),
            "token sequence changed:\n{out}"
        );
        let again = js_format(&out, opts).unwrap();
        assert_eq!(out, again, "not idempotent");
        out
    }

    /// CSS: non-whitespace content kept (an added `;` is allowed), idempotent.
    fn css(src: &str) -> String {
        let out = css_format(src, &o()).unwrap_or_else(|e| panic!("{e}\n--- input:\n{src}"));
        assert_eq!(
            strip_ws(src).replace(';', ""),
            strip_ws(&out).replace(';', ""),
            "content changed:\n{out}"
        );
        assert_eq!(out, css_format(&out, &o()).unwrap(), "not idempotent");
        out
    }

    fn html(src: &str) -> String {
        html_with(src, &o())
    }

    fn html_with(src: &str, opts: &FormatOptions) -> String {
        let out = html_format(src, opts).unwrap_or_else(|e| panic!("{e}\n--- input:\n{src}"));
        assert_eq!(out, html_format(&out, opts).unwrap(), "not idempotent");
        out
    }

    /// Asserts the HTML output differs from the input only in whitespace.
    fn html_exact(src: &str) -> String {
        let out = html(src);
        assert_eq!(strip_ws(src), strip_ws(&out), "content changed:\n{out}");
        out
    }

    /// Markdown content with the deliberate changes normalised away: list
    /// markers, table delimiter rows and table pipes.
    fn md_content(s: &str) -> String {
        s.lines()
            .filter(|l| md_table_delim(l).is_none() || !l.contains('|'))
            .map(|l| match md_list_item(l) {
                Some(it) => it.content,
                None => l.to_string(),
            })
            .map(|l| strip_ws(&l).replace('|', ""))
            .collect()
    }

    fn md(src: &str) -> String {
        let out = markdown_format(src, &o()).unwrap_or_else(|e| panic!("{e}\n--- input:\n{src}"));
        assert_eq!(md_content(src), md_content(&out), "content changed:\n{out}");
        assert_eq!(out, markdown_format(&out, &o()).unwrap(), "not idempotent");
        out
    }

    // ---------------------------------------------------------------- CSS

    #[test]
    fn css_basic_layout() {
        assert_eq!(
            css("a,b>c{color:red;background : blue}\n.x{margin:0  auto}"),
            "a,\nb>c {\n  color: red;\n  background: blue;\n}\n\n.x {\n  margin: 0 auto;\n}\n"
        );
    }

    #[test]
    fn css_strings_urls_and_comments() {
        let out = css(
            "/* head */\n.a::before{content:\"a  ;{ }\";background:url( 'x y.png' ) no-repeat;}\n.b{background:url(data:image/png;base64,AAA=)} /* tail */",
        );
        assert_eq!(
            out,
            "/* head */\n.a::before {\n  content: \"a  ;{ }\";\n  background: url( 'x y.png' ) no-repeat;\n}\n\n.b {\n  background: url(data:image/png;base64,AAA=);\n} /* tail */\n"
        );
    }

    #[test]
    fn css_nested_blocks() {
        let src = "@media screen and (max-width: 600px){.a{color:red}.b,.c{color:blue}}\n@supports (display:grid){.g{display:grid}}";
        assert_eq!(
            css(src),
            "@media screen and (max-width: 600px) {\n  .a {\n    color: red;\n  }\n  .b,\n  .c {\n    color: blue;\n  }\n}\n\n@supports (display:grid) {\n  .g {\n    display: grid;\n  }\n}\n"
        );
        // CSS nesting / SCSS.
        let scss = "$main: #333;\n.card{color:$main;&:hover{color:red}.title{font:bold 12px/1.5 a,b}\n// note\n@include shadow(2px);}";
        assert_eq!(
            css(scss),
            "$main: #333;\n\n.card {\n  color: $main;\n  &:hover {\n    color: red;\n  }\n  .title {\n    font: bold 12px/1.5 a, b;\n  }\n  // note\n  @include shadow(2px);\n}\n"
        );
    }

    #[test]
    fn css_values_and_blank_lines() {
        let src = ".a {\n  color: rgba(0,0,0,.5);\n\n\n  --x:  a,b;\n  width: calc(100% - 2px)\n}\n@import url(x.css);\n@charset \"utf-8\";";
        assert_eq!(
            css(src),
            ".a {\n  color: rgba(0, 0, 0, .5);\n\n  --x: a,b;\n  width: calc(100% - 2px);\n}\n\n@import url(x.css);\n@charset \"utf-8\";\n"
        );
        assert_eq!(css(".a{}"), ".a {\n}\n");
        assert_eq!(css(""), "");
        assert_eq!(
            css(
                "@font-face{font-family:\"My  Font\";src:url(a.woff2)format(\"woff2\"),url(a.woff)}"
            ),
            "@font-face {\n  font-family: \"My  Font\";\n  src: url(a.woff2)format(\"woff2\"), url(a.woff);\n}\n"
        );
    }

    #[test]
    fn css_keyframes_and_tabs() {
        let opts = FormatOptions {
            use_tabs: true,
            ..Default::default()
        };
        let out = css_format(
            "@keyframes spin{from{transform:rotate(0)}to{transform:rotate(360deg)}}",
            &opts,
        )
        .unwrap();
        assert_eq!(
            out,
            "@keyframes spin {\n\tfrom {\n\t\ttransform: rotate(0);\n\t}\n\tto {\n\t\ttransform: rotate(360deg);\n\t}\n}\n"
        );
    }

    #[test]
    fn css_errors() {
        assert!(css_format(".a{color:red", &o()).is_err());
        assert!(css_format(".a{color:red}}", &o()).is_err());
        assert!(css_format(".a{content:\"x}", &o()).is_err());
        assert!(css_format("/* open", &o()).is_err());
    }

    #[test]
    fn css_minify_works() {
        let src = "/*! keep */\n/* drop */\na , b > c {\n  color : red ;\n  content: \"a  b ; }\";\n}\n@media (max-width: 10px) and (min-width:1px) {\n  .x { margin: 0 auto; }\n}\n.y{width:calc(1px + 2px)}";
        assert_eq!(
            css_minify(src).unwrap(),
            "/*! keep */a,b > c{color:red;content:\"a  b ; }\"}@media (max-width:10px) and (min-width:1px){.x{margin:0 auto}}.y{width:calc(1px + 2px)}"
        );
        let min = css_minify(src).unwrap();
        assert_eq!(css_minify(&min).unwrap(), min);
        // Minified CSS formats back to a full layout.
        assert!(css(&min).contains("\n  color: red;\n"));
        assert_eq!(css_minify(".a { b : c ; d:e; }").unwrap(), ".a{b:c;d:e}");
        assert_eq!(css_minify("a :hover{x:y}").unwrap(), "a :hover{x:y}");
    }

    // ---------------------------------------------------------------- JS / TS

    const JS_FIXTURE: &str = r#"#!/usr/bin/env node
'use strict';
/**
   * Small utility module.
   * @module util
   */
import fs from 'fs';
import { join, resolve as res } from "path";
import * as os from 'os';
export { default as thing } from './thing.js';

const DEFAULTS = {retries: 3, delay: 250, verbose: false};
const PATTERNS = [/^\s*#/, /\/\*[\s\S]*?\*\//g, /[a-z]+\/[0-9]+/i];
let counter = 0, total = 10 / 2 / 1;

// Division vs regex after different tokens.
const ratio = total / counter++ / 2;
const half = (total) / 2;
const items = [1, 2, 3].map(x => x / 2).filter(x => /\d/.test(String(x)));
if (/^a/.test("abc")) counter += 1;

function debounce(fn, wait = 100) {
  let timer = null
  return function (...args) {
    clearTimeout(timer)
    timer = setTimeout(() => fn.apply(this, args), wait)
  }
}

async function* walk(dir) {
  for await (const entry of await fs.promises.opendir(dir)) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) yield* walk(p);
    else if (entry.isFile()) { yield p; }
  }
}

export default class Queue extends EventTarget {
  #items = [];
  static count = 0;
  constructor(opts = {}) {
    super();
    this.opts = {...DEFAULTS, ...opts};
    Queue.count++;
  }

  get size() { return this.#items.length; }

  push(item) {
    this.#items.push(item)
    this.dispatchEvent(new Event('push'))
    return this
  }

  *[Symbol.iterator]() {
    yield* this.#items;
  }

  async drain(handler) {
    while (this.#items.length > 0) {
      const next = this.#items.shift();
      try {
        await handler(next);
      } catch (err) {
        console.error(`failed on ${next?.id ?? 'unknown'}: ${err.message}`);
      } finally {
        counter--;
      }
    }
  }
}

const tpl = `Hello ${user.name}, you have ${count > 1 ? `${count} messages` : 'one message'}!
  Second line is kept    exactly.`;
const nested = `a${ {x: 1}.x }b${`c${`d${e}`}`}`;
const str = 'it\'s a "quote"' + "and \"another\"" + '\\';

switch (action.type) {
  case 'add': {
    state = [...state, action.payload];
    break;
  }
  case 'remove':
    state = state.filter(s => s.id !== action.id);
    break;
  default:
    break;
}

label: for (let i = 0; i < 10; i++) {
  for (let j = 0; j < 10; j++) {
    if (i * j > 20) break label;
    if (j % 2) continue;
  }
}

do {
  counter -= 1;
} while (counter > 0);

const obj = {
  name: 'x',
  'quoted-key': 1,
  [computed]: 2,
  method() { return 1; },
  get value() { return this._v; },
  set value(v) { this._v = v; },
  arrow: async (a, b) => a + b,
};

const result = condition
  ? someValue
  : otherValue;

const chain = promise
  .then(res => res.json())
  .then(data => data.items)
  .catch(() => []);

let a = b
++c
const d = e
(function () {})()

const neg = -x + +y - -z + - -w;
const bits = ~flags & 0xff | 0b1010 ^ 0o17;
const big = 1_000_000n + 0x1Fn;
const floats = [.5, 1e-7, 2.5E+3, 3.];
x = y ?? z;
x ||= 1; y &&= 2; z ??= 3;
x **= 2; y >>>= 1; z <<= 3;
const typeofCheck = typeof x === 'undefined' || x instanceof Foo || 'k' in obj;
delete obj.prop, void 0;
new Foo;
new Bar(1, 2).baz();

if (a) b(); else c();

if (a) {
  // only a comment
}

function empty() {}

const fnExpr = function named() { /* inline */ return 1 /* trailing */; };
window.addEventListener('load', () => {
  document.querySelectorAll('.btn').forEach((btn, i) => {
    btn.addEventListener('click', e => { e.preventDefault(); console.log(i); });
  });
}, { passive: true });

export const longArray = [100000000, 200000000, 300000000, 400000000, 500000000, 600000000, 700000000];
export function longCall() { return someFunction(argumentNumberOne, argumentNumberTwo, argumentNumberThree); }
module.exports = { debounce, walk, Queue };
"#;

    const TS_FIXTURE: &str = r#"import type { Request, Response } from 'express';
import { Injectable, Inject } from '@angular/core';

export type Id = string | number;
type Mapper<T, U> = (value: T, index: number) => U;
type Partial2<T> = { [K in keyof T]?: T[K] };
type Nested = Array<Array<Map<string, Set<number>>>>;
type Cond<T> = T extends string ? 'str' : T extends number ? 'num' : never;
type Fn = <T>(x: T) => T;

export interface User {
  readonly id: Id;
  name: string;
  email?: string;
  tags: string[];
  meta: Record<string, unknown>;
  greet(other: User): string;
}

interface Admin extends User, Auditable {
  level: 1 | 2 | 3;
}

export enum Color { Red, Green = 'green', Blue = 4 }
const enum Flags {
  None = 0,
  A = 1 << 0,
  B = 1 << 1,
}

declare module 'x' {
  export function f(): void;
}

namespace Utils {
  export const pi = 3.14;
}

@Injectable({ providedIn: 'root' })
export class UserService<T extends User = User> implements OnInit {
  private cache = new Map<string, T>();
  protected static instances: number = 0;
  public readonly name: string;
  declare foo: number;
  #secret?: string;

  constructor(@Inject(TOKEN) private readonly http: HttpClient, public logger?: Logger) {
    this.name = 'users';
  }

  async find(id: Id): Promise<T | undefined> {
    const key = String(id);
    if (this.cache.has(key)) return this.cache.get(key)!;
    const res = await this.http.get<T>(`/api/users/${key}`).toPromise();
    this.cache.set(key, res as T);
    return res ?? undefined;
  }

  map<U>(fn: Mapper<T, U>): U[] {
    return [...this.cache.values()].map(fn);
  }

  get count(): number { return this.cache.size; }

  abstract?: () => void;
}

export function assertIsDefined<T>(val: T, msg?: string): asserts val is NonNullable<T> {
  if (val === undefined || val === null) {
    throw new Error(msg ?? 'Expected value to be defined');
  }
}

function isString(x: unknown): x is string {
  return typeof x === 'string';
}

const identity = <T,>(x: T): T => x;
const pair = <A, B>(a: A, b: B): [A, B] => [a, b];
let tuple: [string, number?, ...boolean[]] = ['a'];
let fnType: (a: number, b?: string) => void = () => {};
let union: 'a' | 'b' = 'a';
const conf = { a: 1, b: 'two' } as const satisfies Record<string, unknown>;
const el = document.getElementById('x') as HTMLInputElement | null;
const value = el?.value?.trim() ?? '';
const len = (value as string).length;
const sure = maybe!;
let generic: Promise<Array<{ id: number; name: string }>>;
const cmp = a < b && c > d;
const shift = x >> 2 >>> 1;
for (const [k, v] of Object.entries(obj) as [string, number][]) {
  console.log(k, v);
}

export abstract class Shape {
  abstract area(): number;
  toString(): string {
    return `${this.constructor.name}(${this.area().toFixed(2)})`;
  }
}

function overload(x: string): string;
function overload(x: number): number;
function overload(x: any) {
  return x;
}

export default async function handler(req: Request, res: Response): Promise<void> {
  const { id, ...rest } = req.params as { id: string; [k: string]: string };
  try {
    const user = await service.find(id);
    res.status(user ? 200 : 404).json(user ?? { error: 'not found' });
  } catch (e: unknown) {
    res.status(500).send((e as Error).message);
  }
}
"#;

    #[test]
    fn js_fixture_round_trips() {
        let out = js(JS_FIXTURE);
        assert!(
            out.contains("const DEFAULTS = { retries: 3, delay: 250, verbose: false };\n"),
            "{out}"
        );
        assert!(out.contains("  Second line is kept    exactly.`;\n"));
        assert!(out.contains("let a = b\n++c\n"), "{out}");
        assert!(out.contains("const d = e\n(function () {})()\n"), "{out}");
        assert!(out.contains("} while (counter > 0);\n"));
        assert!(
            out.contains("/**\n * Small utility module.\n * @module util\n */\n"),
            "{out}"
        );
        assert!(
            out.contains("export const longArray = [\n  100000000,\n"),
            "{out}"
        );
        for opts in [
            FormatOptions {
                tab_width: 4,
                ..Default::default()
            },
            FormatOptions {
                use_tabs: true,
                print_width: 40,
                ..Default::default()
            },
        ] {
            js_with(JS_FIXTURE, &opts);
        }
    }

    #[test]
    fn ts_fixture_round_trips() {
        let out = js(TS_FIXTURE);
        assert!(
            out.contains("type Nested = Array<Array<Map<string, Set<number>>>>;\n"),
            "{out}"
        );
        assert!(
            out.contains("export enum Color {\n  Red,\n  Green = 'green',\n  Blue = 4\n}\n"),
            "{out}"
        );
        assert!(out.contains("const sure = maybe!;\n"), "{out}");
        assert!(out.contains("  email?: string;\n"), "{out}");
        assert!(
            out.contains("@Injectable({ providedIn: 'root' })\nexport class"),
            "{out}"
        );
        js_with(
            TS_FIXTURE,
            &FormatOptions {
                print_width: 50,
                ..Default::default()
            },
        );
    }

    #[test]
    fn js_basic_layout() {
        assert_eq!(
            js("function f(a,b){if(a>b){return a}else{return b}}"),
            "function f(a, b) {\n  if (a>b) {\n    return a\n  } else {\n    return b\n  }\n}\n"
        );
        assert_eq!(js("let x=1;let y=2"), "let x = 1;\nlet y = 2\n");
        assert_eq!(
            js("for(let i=0;i<n;i++){}"),
            "for (let i = 0; i<n; i++) {}\n"
        );
        assert_eq!(
            js("try{a()}\ncatch(e){b()}\nfinally{c()}"),
            "try {\n  a()\n} catch (e) {\n  b()\n} finally {\n  c()\n}\n"
        );
        // Allman braces move up; `else` joins the closing brace.
        assert_eq!(
            js("if (a)\n{\n  b();\n}\nelse\n{\n  c();\n}"),
            "if (a) {\n  b();\n} else {\n  c();\n}\n"
        );
        assert_eq!(js(""), "");
        assert_eq!(js("   \n\n  "), "");
    }

    #[test]
    fn js_blank_lines_and_comments() {
        assert_eq!(
            js("a();\n\n\n\nb(); // trailing\n/* block */\nc();\n"),
            "a();\n\nb(); // trailing\n/* block */\nc();\n"
        );
        // Blank lines at the start and end of a block are dropped.
        assert_eq!(
            js("function f() {\n\n  a();\n\n}"),
            "function f() {\n  a();\n}\n"
        );
        assert_eq!(js("x = 1 // c\ny = 2"), "x = 1 // c\ny = 2\n");
        assert_eq!(js("f(a, // first\n  b)"), "f(a, // first\n  b)\n");
    }

    #[test]
    fn js_asi_newlines_are_kept() {
        // Each of these would change meaning if the lines were joined.
        for src in [
            "return\nx",
            "a = b\n(c || d).e()",
            "a = b\n[1, 2].forEach(f)",
            "let x = y\n`tpl`",
            "i\n++\nj",
            "a\n/re/g.test(s)",
            "const f = async\nfunction g() {}",
            "throw\nnew Error()",
            "x\n-1",
        ] {
            let out = js(src);
            assert_eq!(
                out.matches('\n').count(),
                src.matches('\n').count() + 1,
                "{src:?} -> {out:?}"
            );
        }
    }

    #[test]
    fn js_objects_and_arrays() {
        assert_eq!(js("const o={a:1,b:2}"), "const o = { a: 1, b: 2 }\n");
        assert_eq!(
            js("const o={\na:1,b:2}"),
            "const o = {\n  a: 1,\n  b: 2\n}\n"
        );
        assert_eq!(js("const a=[\n1,2,\n]"), "const a = [\n  1,\n  2,\n]\n");
        assert_eq!(js("f({})\nf([])"), "f({})\nf([])\n");
        assert_eq!(js("const {a,b:{c}}=obj"), "const { a, b: { c } } = obj\n");
        let narrow = FormatOptions {
            print_width: 20,
            ..Default::default()
        };
        assert_eq!(
            js_with("x = {alpha: 1, beta: 2, gamma: 3}", &narrow),
            "x = {\n  alpha: 1,\n  beta: 2,\n  gamma: 3\n}\n"
        );
        // An object containing a function body is always multi-line.
        assert_eq!(
            js("x = { f() { return 1 } }"),
            "x = {\n  f() {\n    return 1\n  }\n}\n"
        );
    }

    #[test]
    fn js_operators_and_spacing() {
        assert_eq!(js("a=b+c*d-e/f%g"), "a = b + c * d - e / f % g\n");
        assert_eq!(js("x=-y;z=!a&&~b"), "x = -y;\nz = !a && ~b\n");
        assert_eq!(js("a = b - -c; d = e + +f"), "a = b - -c;\nd = e + +f\n");
        assert_eq!(js("a=b?c:d"), "a = b ? c : d\n");
        assert_eq!(js("x=a?.b?.[0]?.(1)??2"), "x = a?.b?.[0]?.(1) ?? 2\n");
        assert_eq!(js("i++;--j"), "i++;\n--j\n");
        assert_eq!(js("f=(...a)=>a"), "f = (...a) => a\n");
        assert_eq!(js("x=1 .toString()"), "x = 1 .toString()\n");
        assert_eq!(js("typeof(x);void(0)"), "typeof (x);\nvoid (0)\n");
        assert_eq!(js("a = b/c/d"), "a = b / c / d\n");
        assert_eq!(js("x = a ? (b) : c"), "x = a ? (b) : c\n");
    }

    #[test]
    fn js_regex_and_strings_are_untouched() {
        assert_eq!(js("x = /a b[/]c/gi.exec(s)"), "x = /a b[/]c/gi.exec(s)\n");
        assert_eq!(js("x = a / b / c"), "x = a / b / c\n");
        assert_eq!(js("if (x) /re/.test(y)"), "if (x) /re/.test(y)\n");
        assert_eq!(
            js("s = 'a  b' + \"c\\\"  d\""),
            "s = 'a  b' + \"c\\\"  d\"\n"
        );
        assert_eq!(
            js("t = `x  ${ a  +  `y ${b}` }  z`"),
            "t = `x  ${ a  +  `y ${b}` }  z`\n"
        );
        assert_eq!(js("x = y[0] / 2"), "x = y[0] / 2\n");
    }

    #[test]
    fn js_switch_and_labels() {
        assert_eq!(
            js("switch(x){case 1:a();break;case 2:{b()}default:c()}"),
            "switch (x) {\n  case 1:\n    a();\n    break;\n  case 2: {\n    b()\n  }\n  default:\n    c()\n}\n"
        );
    }

    #[test]
    fn ts_constructs() {
        assert_eq!(
            js("let x:Array<string> = []"),
            "let x: Array<string> = []\n"
        );
        // `>=` may end a generic, so its spacing is kept as written.
        assert_eq!(js("let x:Array<string>=[]"), "let x: Array<string>=[]\n");
        assert_eq!(
            js("function f<T>(a:T):T{return a}"),
            "function f<T>(a: T): T {\n  return a\n}\n"
        );
        assert_eq!(
            js("interface A{a?:number;b:string}"),
            "interface A {\n  a?: number;\n  b: string\n}\n"
        );
        assert_eq!(js("let v=x as unknown as T"), "let v = x as unknown as T\n");
        assert_eq!(
            js("@dec() class A{@prop() x=1}"),
            "@dec() class A {\n  @prop() x = 1\n}\n"
        );
        assert_eq!(js("const f=<T,>(x:T)=>x"), "const f = <T,>(x: T) => x\n");
        assert_eq!(js("type A<T> = {a:T}"), "type A<T> = { a: T }\n");
        js("type A<T>={a:T}");
        assert_eq!(
            js("let m=new Map<string,number[]>()"),
            "let m = new Map<string, number[]>()\n"
        );
    }

    #[test]
    fn js_errors() {
        assert!(js_format("const a = 'open", &o()).is_err());
        assert!(js_format("const a = `open ${x}", &o()).is_err());
        assert!(js_format("f(a", &o()).is_err());
        assert!(js_format("f(a]", &o()).is_err());
        assert!(js_format("/* open", &o()).is_err());
        assert!(js_format("return <div>hi</div>", &o()).is_err());
        assert!(js_format("x = (<any>y)", &o()).is_err());
    }

    // ---------------------------------------------------------------- HTML

    #[test]
    fn html_document_layout() {
        let src = "<!DOCTYPE html><html><head><meta charset=utf-8><title>T</title><link rel=\"stylesheet\" href=\"a.css\"></head><body><div class=\"x\"><h1>Hi</h1><p>Hello <b>world</b>, <a href='#'>link</a>.</p></div></body></html>";
        assert_eq!(
            html_exact(src),
            "<!DOCTYPE html>\n<html>\n  <head>\n    <meta charset=utf-8>\n    <title>T</title>\n    <link rel=\"stylesheet\" href=\"a.css\">\n  </head>\n  <body>\n    <div class=\"x\">\n      <h1>Hi</h1>\n      <p>Hello <b>world</b>, <a href='#'>link</a>.</p>\n    </div>\n  </body>\n</html>\n"
        );
    }

    #[test]
    fn html_inline_flow_and_whitespace() {
        // Adjacent inline elements stay glued; whitespace runs collapse.
        assert_eq!(
            html_exact("<p>a<b>b</b><i>c</i>   d\n\te</p>"),
            "<p>a<b>b</b><i>c</i> d e</p>\n"
        );
        // Too long for one line: content moves inside, input line breaks kept.
        let long = format!(
            "<div><p>{}\n{}</p></div>",
            "word ".repeat(14).trim(),
            "more ".repeat(5).trim()
        );
        assert_eq!(
            html_exact(&long),
            format!(
                "<div>\n  <p>\n    {}\n    {}\n  </p>\n</div>\n",
                "word ".repeat(14).trim(),
                "more ".repeat(5).trim()
            )
        );
        // Mixed text and blocks.
        assert_eq!(
            html_exact("<div>text<div>block</div>tail <em>x</em></div>"),
            "<div>\n  text\n  <div>block</div>\n  tail <em>x</em>\n</div>\n"
        );
    }

    #[test]
    fn html_pre_textarea_verbatim() {
        let src = "<div><pre>  a\n    <b>b</b>\n</pre><textarea name=t>  x\n  y </textarea></div>";
        assert_eq!(
            html_exact(src),
            "<div>\n  <pre>  a\n    <b>b</b>\n</pre>\n  <textarea name=t>  x\n  y </textarea>\n</div>\n"
        );
    }

    #[test]
    fn html_script_and_style() {
        let src = "<head><style>\nbody{margin:0}\n</style><script>if(a){b()}</script><script src=\"x.js\"></script></head>";
        assert_eq!(
            html(src),
            "<head>\n  <style>\n    body {\n      margin: 0;\n    }\n  </style>\n  <script>\n    if (a) {\n      b()\n    }\n  </script>\n  <script src=\"x.js\"></script>\n</head>\n"
        );
        // Unparseable or non-JS contents are kept as they were.
        let bad = "<script>let s = 'open</script>";
        assert_eq!(html_exact(bad), "<script>let s = 'open</script>\n");
        let tpl = "<script type=\"text/template\"><div>  {{x}}</div></script>";
        assert_eq!(html_exact(tpl), format!("{tpl}\n"));
        let ml = "<script>const t = `a\n  b`;</script>";
        assert_eq!(html_exact(ml), format!("{ml}\n"));
        let json = "<script type=\"application/ld+json\">{\"a\":1}</script>";
        assert_eq!(
            html_exact(json),
            "<script type=\"application/ld+json\">\n  { \"a\": 1 }\n</script>\n"
        );
    }

    #[test]
    fn html_implied_end_tags() {
        let src = "<ul><li>One<li>Two <b>bold</b></ul><p>Para one<p>Para two<div>x</div><table><tr><td>a<td>b<tr><td>c</table>";
        assert_eq!(
            html_exact(src),
            "<ul>\n  <li>One\n  <li>Two <b>bold</b>\n</ul>\n<p>Para one\n<p>Para two\n<div>x</div>\n<table>\n  <tr>\n    <td>a\n    <td>b\n  <tr>\n    <td>c\n</table>\n"
        );
    }

    #[test]
    fn html_comments_void_and_attributes() {
        assert_eq!(
            html_exact("<div><!-- note --><br/><img src=a.png alt=\"a b\"><input disabled></div>"),
            "<div><!-- note --><br /><img src=a.png alt=\"a b\"><input disabled></div>\n"
        );
        assert_eq!(
            html_exact("<div>\n<!-- note -->\n<hr><p>x</p></div>"),
            "<div>\n  <!-- note -->\n  <hr>\n  <p>x</p>\n</div>\n"
        );
        // A comment glued to text stays inline.
        assert_eq!(html_exact("<p>a<!-- x -->b</p>"), "<p>a<!-- x -->b</p>\n");
        // Long attribute lists wrap one per line.
        let src = format!(
            "<div id=\"main\" class=\"{}\" data-x=\"1\"><span>x</span></div>",
            "c ".repeat(30).trim()
        );
        let out = html_exact(&src);
        assert!(out.starts_with("<div\n  id=\"main\"\n  class=\""), "{out}");
        let o2 = FormatOptions {
            bracket_same_line: false,
            ..Default::default()
        };
        let out2 = html_with(&src, &o2);
        assert!(
            out2.contains("\n  data-x=\"1\"\n>\n  <span>x</span>\n</div>"),
            "{out2}"
        );
        // Stray end tags and bare `<` are kept as text.
        assert_eq!(html_exact("<p>a < b</p></span>"), "<p>a < b</p>\n</span>\n");
    }

    #[test]
    fn html_blank_lines_and_fragments() {
        assert_eq!(
            html_exact("<div>\n<p>a</p>\n\n\n<p>b</p>\n</div>"),
            "<div>\n  <p>a</p>\n\n  <p>b</p>\n</div>\n"
        );
        assert_eq!(html_exact("Just text"), "Just text\n");
        assert_eq!(html_exact(""), "");
        assert_eq!(
            html_exact("<my-widget foo=1></my-widget>"),
            "<my-widget foo=1></my-widget>\n"
        );
        assert_eq!(
            html_exact("<svg viewBox=\"0 0 1 1\"><path d=\"M0 0\"/></svg>"),
            "<svg viewBox=\"0 0 1 1\">\n  <path d=\"M0 0\" />\n</svg>\n"
        );
    }

    #[test]
    fn html_errors() {
        assert!(html_format("<div", &o()).is_err());
        assert!(html_format("<!-- open", &o()).is_err());
        assert!(html_format("<a href=\"x>", &o()).is_err());
        assert!(html_format("<script>x", &o()).is_err());
    }

    // ---------------------------------------------------------------- Markdown

    #[test]
    fn md_headings_and_spacing() {
        assert_eq!(
            md("#   Title  \nText line\n##Not heading\n### Sub ###\nmore"),
            "# Title\n\nText line\n##Not heading\n\n### Sub ###\n\nmore\n"
        );
        assert_eq!(md("a\n\n\n\nb\n"), "a\n\nb\n");
        assert_eq!(md("Setext\n===\nbody"), "Setext\n===\n\nbody\n");
        assert_eq!(md(""), "");
    }

    #[test]
    fn md_hard_breaks() {
        assert_eq!(md("one  \ntwo   \nthree  "), "one  \ntwo  \nthree\n");
    }

    #[test]
    fn md_lists() {
        assert_eq!(
            md("Intro\n* a\n* b\n    + nested\n    + more\n* c\n"),
            "Intro\n\n- a\n- b\n  - nested\n  - more\n- c\n"
        );
        assert_eq!(md("3. x\n7. y\n1. z"), "3. x\n4. y\n5. z\n");
        assert_eq!(
            md("1) a\n   1) inner\n   1) inner2\n2) b"),
            "1) a\n   1) inner\n   2) inner2\n2) b\n"
        );
        // Lazy and indented continuation lines are re-indented to the item.
        assert_eq!(
            md("- item\ncontinued\n- two\n\n     para of two"),
            "- item\n  continued\n- two\n\n  para of two\n"
        );
        // Loose lists keep one blank line between items.
        assert_eq!(md("- a\n\n\n- b"), "- a\n\n- b\n");
        // Thematic breaks are not bullets.
        assert_eq!(md("* * *\n- - -"), "* * *\n\n- - -\n");
        // Lists with code fences inside are left alone.
        let fenced = "* a\n\n  ```\n  code\n  ```\n* b";
        assert_eq!(md(fenced), format!("{fenced}\n"));
        assert_eq!(md("- [ ] todo\n- [x] done"), "- [ ] todo\n- [x] done\n");
    }

    #[test]
    fn md_tables() {
        assert_eq!(
            md("|a|b|c|\n|:-|-:|:-:|\n|long cell|2|x|\n|1|\n"),
            "| a         |   b |  c  |\n| :-------- | --: | :-: |\n| long cell |   2 |  x  |\n| 1         |     |     |\n"
        );
        assert_eq!(
            md("Name | Value\n--- | ---\n`a\\|b` | é漢\n"),
            "| Name   | Value |\n| ------ | ----- |\n| `a\\|b` | é漢   |\n"
        );
    }

    #[test]
    fn md_code_and_html_are_verbatim() {
        let src = "Text\n```rust\nfn  main() {}   \n\n\n  * not a list\n```\n\n    indented   code\n\n<div>\n  *raw*   \n</div>\n\n~~~\n# not heading\n~~~";
        assert_eq!(
            md(src),
            "Text\n\n```rust\nfn  main() {}   \n\n\n  * not a list\n```\n\n    indented   code\n\n<div>\n  *raw*   \n</div>\n\n~~~\n# not heading\n~~~\n"
        );
        // Unclosed fences run to the end of the document.
        assert_eq!(md("```\n#  x\n\n\n"), "```\n#  x\n");
        let fm = "---\ntitle:  x\n---\n# H";
        assert_eq!(md(fm), "---\ntitle:  x\n---\n\n# H\n");
        assert_eq!(md("> quote  \nlazy\n\nafter"), "> quote  \nlazy\n\nafter\n");
    }

    // ---------------------------------------------------------------- realistic files

    #[test]
    fn js_layout_ignores_input_indentation() {
        for fixture in [JS_FIXTURE, TS_FIXTURE] {
            let dedented: String = fixture
                .lines()
                .map(|l| format!("{}\n", l.trim_start()))
                .collect();
            // Template literal contents are the only indentation that matters.
            let dedented = dedented.replace(
                "\nSecond line is kept    exactly.`",
                "\n  Second line is kept    exactly.`",
            );
            assert_eq!(js(&dedented), js(fixture));
        }
    }

    const CSS_FIXTURE: &str = r#"@charset "utf-8";
@import url("reset.css") screen;
:root{--brand:#0af;--gap: 8px ;--font-stack:"Helvetica Neue",Arial,sans-serif}
/* Layout */
html,body{margin:0;padding:0}
body{font:16px/1.5 var(--font-stack);color:#222;background:#fff url("img/bg.png") repeat-x}
.container{max-width:1200px;margin:0 auto;padding:0 calc(var(--gap)*2)}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(200px,1fr));gap:var(--gap)}
a:not(.btn):hover,a:focus-visible{text-decoration:underline}
input[type="text"],input[type='email']{border:1px solid #ccc}
.btn::after{content:"\2192";margin-left:.25em}
.icon{background-image:url(data:image/svg+xml;charset=utf8,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%3E%3C%2Fsvg%3E)}
@media (min-width:768px) and (max-width:1024px){
  .container{padding:0 16px}
  .grid{grid-template-columns:1fr 1fr}
}
@media print{*{color:#000!important}}
@supports (backdrop-filter:blur(4px)){.glass{backdrop-filter:blur(4px)}}
@font-face{font-family:"Inter";src:url(inter.woff2) format("woff2");font-display:swap}
@keyframes pulse{0%{opacity:1}50%{opacity:.5}100%{opacity:1}}
.card{
  padding:16px;

  & > h2{margin-top:0}
  &.active{border-color:var(--brand)}
  .meta{color:#999}
}
"#;

    #[test]
    fn css_fixture() {
        let out = css(CSS_FIXTURE);
        assert!(
            out.contains(":root {\n  --brand: #0af;\n  --gap: 8px;\n"),
            "{out}"
        );
        assert!(
            out.contains("a:not(.btn):hover,\na:focus-visible {\n"),
            "{out}"
        );
        assert!(
            out.contains("  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));\n"),
            "{out}"
        );
        assert!(
            out.contains("@media print {\n  * {\n    color: #000!important;\n  }\n}\n"),
            "{out}"
        );
        assert!(
            out.contains("  padding: 16px;\n\n  & > h2 {\n    margin-top: 0;\n  }\n"),
            "{out}"
        );
        let min = css_minify(CSS_FIXTURE).unwrap();
        assert!(!min.contains('\n') && !min.contains("/* Layout */"));
        assert!(min.contains("content:\"\\2192\""));
        css(&min);
    }

    const HTML_FIXTURE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Example page</title>
<link rel="icon" href="/favicon.ico">
<style>
body { font-family: sans-serif }
.hero h1{font-size:3rem}
</style>
</head>
<body class="home">
<!-- Navigation -->
<nav><ul><li><a href="/">Home</a></li><li><a href="/about" class=active>About us</a></li></ul></nav>
<main id=content>
<section class="hero"><h1>Welcome to <em>our</em> site</h1>
<p>We build <strong>fast</strong> things. Read the <a href="/docs">docs</a> or
<a href="/blog">blog</a> to learn more.</p>
<img src="hero.jpg" alt="Hero image" width=600 height=400>
</section>
<form action="/subscribe" method="post"><label for=email>Email</label> <input type=email id=email name=email required><button type=submit>Subscribe</button></form>
<table><thead><tr><th>Name<th>Price</thead><tbody><tr><td>Tea<td>$2<tr><td>Coffee<td>$3</tbody></table>
<pre><code>fn main() {
    println!("hi");
}</code></pre>
<dl><dt>Term<dd>Definition</dl>
<select name=s><option value=1 selected>One<option value=2>Two</select>
</main>
<footer><p>&copy; 2026 Example &mdash; all rights reserved.<br>Made with care.</p></footer>
<script src="/app.js" defer></script>
<script>
document.addEventListener('DOMContentLoaded',()=>{const n=document.querySelectorAll('nav a');n.forEach(a=>a.addEventListener('click',e=>console.log(e)))})
</script>
</body>
</html>
"#;

    #[test]
    fn html_fixture() {
        let out = html(HTML_FIXTURE);
        // Only whitespace changes, plus the `;` CSS adds and the JS layout.
        assert_eq!(
            strip_ws(HTML_FIXTURE).replace(';', ""),
            strip_ws(&out).replace(';', "")
        );
        assert!(
            out.contains("      <pre><code>fn main() {\n    println!(\"hi\");\n}</code></pre>\n"),
            "{out}"
        );
        assert!(
            out.contains("    <nav>\n      <ul>\n        <li><a href=\"/\">Home</a></li>\n"),
            "{out}"
        );
        assert!(
            out.contains("          <tr>\n            <td>Tea\n            <td>$2\n"),
            "{out}"
        );
        assert!(
            out.contains("      document.addEventListener('DOMContentLoaded', () => {\n"),
            "{out}"
        );
        assert!(
            out.contains("      .hero h1 {\n        font-size: 3rem;\n"),
            "{out}"
        );
        html_with(
            HTML_FIXTURE,
            &FormatOptions {
                tab_width: 4,
                print_width: 60,
                ..Default::default()
            },
        );
    }

    const MD_FIXTURE: &str = "superfastnotepad\n================\n\nA *fast* editor.   \n\n## Features\n* Syntax highlighting\n* Big files\n   * up to 2 GB\n   * memory mapped\n* Plugins\n\n## Install\n1. Download\n1. Unzip\n1. Run `./sfn`  \n   from a terminal\n\n```sh\ncargo build --release   \n```\n\nOption | Default | Notes\n:--|:-:|--:\n`tab_width` | 2 | spaces\n`use_tabs` | false | \\| pipes\n\n> **Note**\n> Works on macOS.\n\n<details>\n<summary>More</summary>\n\nHidden *text*.\n\n</details>\n\n---\nLast line\n";

    #[test]
    fn md_fixture() {
        let out = md(MD_FIXTURE);
        assert!(
            out.contains("## Features\n\n- Syntax highlighting\n- Big files\n  - up to 2 GB\n"),
            "{out}"
        );
        assert!(
            out.contains("1. Download\n2. Unzip\n3. Run `./sfn`  \n   from a terminal\n"),
            "{out}"
        );
        assert!(
            out.contains("```sh\ncargo build --release   \n```"),
            "{out}"
        );
        assert!(
            out.contains(
                "| Option      | Default |    Notes |\n| :---------- | :-----: | -------: |\n"
            ),
            "{out}"
        );
        assert!(out.contains("A *fast* editor.\n"), "{out}");
    }

    /// Every prefix of every fixture either errors cleanly or formats
    /// idempotently without losing content (no panics on partial input).
    #[test]
    fn truncated_inputs_never_panic_or_corrupt() {
        let cut = |s: &'static str| {
            (0..s.len())
                .step_by(3)
                .filter(move |&i| s.is_char_boundary(i))
                .map(move |i| &s[..i])
        };
        for src in cut(JS_FIXTURE).chain(cut(TS_FIXTURE)) {
            if let Ok(out) = js_format(src, &o()) {
                assert_eq!(js_toks(src), js_toks(&out), "{src}");
                assert_eq!(js_format(&out, &o()).unwrap(), out, "{src}");
            }
        }
        for src in cut(CSS_FIXTURE) {
            if let Ok(out) = css_format(src, &o()) {
                assert_eq!(
                    strip_ws(src).replace(';', ""),
                    strip_ws(&out).replace(';', ""),
                    "{src}"
                );
                assert_eq!(css_format(&out, &o()).unwrap(), out, "{src}");
            }
            let _ = css_minify(src);
        }
        for src in cut(HTML_FIXTURE) {
            if let Ok(out) = html_format(src, &o()) {
                assert_eq!(html_format(&out, &o()).unwrap(), out, "{src}");
            }
        }
        for src in cut(MD_FIXTURE) {
            let out = markdown_format(src, &o()).unwrap();
            assert_eq!(md_content(src), md_content(&out), "{src}");
            assert_eq!(markdown_format(&out, &o()).unwrap(), out, "{src:?}");
        }
    }
}
