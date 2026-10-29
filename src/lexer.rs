//! A generic, line-incremental syntax lexer.
//!
//! Instead of one hand-written lexer per language, we drive a small
//! number of lexer *families* (code, markup, ini, diff, markdown) with the
//! keyword lists and comment delimiters from `langs.model.xml`. Each line is
//! lexed from the state left by the previous line, so the document only needs
//! to cache one small `LexState` per line.

use std::collections::HashSet;

use crate::langs::{Family, FoldMode, LangRegistry, Language, LexProfile};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Default,
    Comment,
    CommentLine,
    CommentDoc,
    Number,
    String,
    Char,
    Operator,
    Preprocessor,
    Variable,
    Tag,
    Attribute,
    AttrValue,
    Entity,
    Section,
    Key,
    Assign,
    DiffAdd,
    DiffDel,
    DiffHeader,
    DiffPosition,
    Heading,
    Emphasis,
    Code,
    Link,
    /// Index into `Language::keywords`.
    Keyword(u8),
}

impl TokenKind {
    pub const COUNT: usize = 25;

    pub fn index(self) -> usize {
        match self {
            Self::Default | Self::Keyword(_) => 0,
            Self::Comment => 1,
            Self::CommentLine => 2,
            Self::CommentDoc => 3,
            Self::Number => 4,
            Self::String => 5,
            Self::Char => 6,
            Self::Operator => 7,
            Self::Preprocessor => 8,
            Self::Variable => 9,
            Self::Tag => 10,
            Self::Attribute => 11,
            Self::AttrValue => 12,
            Self::Entity => 13,
            Self::Section => 14,
            Self::Key => 15,
            Self::Assign => 16,
            Self::DiffAdd => 17,
            Self::DiffDel => 18,
            Self::DiffHeader => 19,
            Self::DiffPosition => 20,
            Self::Heading => 21,
            Self::Emphasis => 22,
            Self::Code => 23,
            Self::Link => 24,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum CodeState {
    #[default]
    Normal,
    Block {
        doc: bool,
    },
    Str {
        delim: char,
        triple: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LexState {
    Code(CodeState),
    MkText,
    MkComment,
    MkCData,
    MkPi,
    MkTag { script: u8 },
    MkAttrStr { delim: char, script: u8 },
    MkScript { kind: u8, inner: CodeState },
    MkPhp(CodeState),
    Fenced,
}

impl Default for LexState {
    fn default() -> Self {
        Self::Code(CodeState::Normal)
    }
}

pub type Span = (usize, usize, TokenKind);

struct Keywords {
    classes: Vec<HashSet<String>>,
    case_insensitive: bool,
}

impl Keywords {
    fn from_lang(lang: &Language) -> Self {
        Self {
            classes: lang.keywords.iter().map(|(_, s)| s.clone()).collect(),
            case_insensitive: lang.profile.case_insensitive,
        }
    }

    fn lookup(&self, word: &str) -> Option<u8> {
        if self.classes.is_empty() {
            return None;
        }
        let owned;
        let w = if self.case_insensitive {
            owned = word.to_lowercase();
            owned.as_str()
        } else {
            word
        };
        self.classes
            .iter()
            .position(|set| set.contains(w))
            .and_then(|i| u8::try_from(i).ok())
    }
}

struct CodeRules {
    comment_line: Option<String>,
    comment_line_is_word: bool,
    comment_start: Option<String>,
    comment_end: Option<String>,
    profile: LexProfile,
    multiline_strings: bool,
    rust_like: bool,
}

pub struct Lexer {
    family: Family,
    rules: CodeRules,
    kw: Keywords,
    /// Rules + keywords for `<script>` blocks inside markup.
    script_rules: CodeRules,
    script_kw: Keywords,
    is_php: bool,
    is_html: bool,
}

const HTML_VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr", "!doctype",
];

fn is_ident_char(c: char, extra: &str) -> bool {
    c.is_alphanumeric() || c == '_' || extra.contains(c)
}

fn code_rules(lang: &Language) -> CodeRules {
    let cl = lang.comment_line.clone();
    let is_word = cl
        .as_deref()
        .is_some_and(|c| c.chars().all(char::is_alphabetic));
    CodeRules {
        comment_line_is_word: is_word,
        comment_line: cl,
        comment_start: lang.comment_start.clone(),
        comment_end: lang.comment_end.clone(),
        multiline_strings: matches!(
            lang.name.as_str(),
            "rust"
                | "bash"
                | "perl"
                | "ruby"
                | "sql"
                | "php"
                | "powershell"
                | "r"
                | "toml"
                | "nim"
                | "swift"
                | "lua"
                | "julia"
                | "elixir"
        ),
        rust_like: lang.name == "rust",
        profile: lang.profile.clone(),
    }
}

impl Lexer {
    pub fn new(lang: &Language, registry: &LangRegistry) -> Self {
        let js = registry.by_name("javascript").map(|i| &registry.langs[i]);
        let (script_rules, script_kw) = js.map_or_else(
            || {
                (
                    code_rules(lang),
                    Keywords {
                        classes: Vec::new(),
                        case_insensitive: false,
                    },
                )
            },
            |js| (code_rules(js), Keywords::from_lang(js)),
        );
        Self {
            family: lang.profile.family,
            rules: code_rules(lang),
            kw: Keywords::from_lang(lang),
            script_rules,
            script_kw,
            is_php: lang.name == "php",
            is_html: matches!(lang.name.as_str(), "html" | "php"),
        }
    }

    pub fn initial_state(&self) -> LexState {
        match self.family {
            Family::Markup => LexState::MkText,
            _ => LexState::Code(CodeState::Normal),
        }
    }

    pub fn fold_mode(&self) -> FoldMode {
        self.rules.profile.fold
    }

    /// Lexes one line (without its EOL). Returns the state at the end of the
    /// line and the change in fold depth caused by this line.
    pub fn lex_line(&self, line: &str, state: LexState, spans: &mut Vec<Span>) -> (LexState, i32) {
        spans.clear();
        match self.family {
            Family::Plain => (state, 0),
            Family::Code => {
                let cs = match state {
                    LexState::Code(c) => c,
                    _ => CodeState::Normal,
                };
                let mut fold = 0;
                let (cs, _) = lex_code(&self.rules, &self.kw, line, 0, cs, None, spans, &mut fold);
                (LexState::Code(cs), fold)
            }
            Family::Markup => self.lex_markup(line, state, spans),
            Family::Ini => (lex_ini(line, &self.rules, spans), 0),
            Family::Diff => {
                lex_diff(line, spans);
                (state, 0)
            }
            Family::Markdown => (lex_markdown(line, state, spans), 0),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn lex_markup(
        &self,
        line: &str,
        mut state: LexState,
        spans: &mut Vec<Span>,
    ) -> (LexState, i32) {
        let b = line.as_bytes();
        let mut i = 0;
        let mut fold = 0;
        let lower = line.to_ascii_lowercase();
        while i < b.len() {
            match state {
                LexState::MkComment => {
                    if let Some(e) = line[i..].find("-->") {
                        spans.push((i, i + e + 3, TokenKind::Comment));
                        i += e + 3;
                        state = LexState::MkText;
                    } else {
                        spans.push((i, b.len(), TokenKind::Comment));
                        i = b.len();
                    }
                }
                LexState::MkCData => {
                    if let Some(e) = line[i..].find("]]>") {
                        spans.push((i, i + e + 3, TokenKind::String));
                        i += e + 3;
                        state = LexState::MkText;
                    } else {
                        spans.push((i, b.len(), TokenKind::String));
                        i = b.len();
                    }
                }
                LexState::MkPi => {
                    if let Some(e) = line[i..].find("?>") {
                        spans.push((i, i + e + 2, TokenKind::Preprocessor));
                        i += e + 2;
                        state = LexState::MkText;
                    } else {
                        spans.push((i, b.len(), TokenKind::Preprocessor));
                        i = b.len();
                    }
                }
                LexState::MkPhp(cs) => {
                    let (cs2, stop) = lex_code(
                        &self.rules,
                        &self.kw,
                        line,
                        i,
                        cs,
                        Some("?>"),
                        spans,
                        &mut 0,
                    );
                    if let Some(p) = stop {
                        spans.push((p, p + 2, TokenKind::Preprocessor));
                        i = p + 2;
                        state = LexState::MkText;
                    } else {
                        state = LexState::MkPhp(cs2);
                        i = b.len();
                    }
                }
                LexState::MkScript { kind, inner } => {
                    let stop = if kind == 1 { "</script" } else { "</style" };
                    let end = lower[i..].find(stop).map(|p| p + i);
                    let segment_end = end.unwrap_or(b.len());
                    let (cs2, _) = lex_code(
                        &self.script_rules,
                        if kind == 1 {
                            &self.script_kw
                        } else {
                            &EMPTY_KW
                        },
                        &line[..segment_end],
                        i,
                        inner,
                        None,
                        spans,
                        &mut 0,
                    );
                    if let Some(p) = end {
                        i = p;
                        state = LexState::MkText;
                    } else {
                        state = LexState::MkScript { kind, inner: cs2 };
                        i = b.len();
                    }
                }
                LexState::MkAttrStr { delim, script } => {
                    let rest = &line[i..];
                    if let Some(e) = rest.find(delim) {
                        spans.push((i, i + e + 1, TokenKind::AttrValue));
                        i += e + 1;
                        state = LexState::MkTag { script };
                    } else {
                        spans.push((i, b.len(), TokenKind::AttrValue));
                        i = b.len();
                    }
                }
                LexState::MkTag { script } => {
                    let c = b[i];
                    if c == b'>' {
                        spans.push((i, i + 1, TokenKind::Tag));
                        i += 1;
                        state = if script > 0 {
                            LexState::MkScript {
                                kind: script,
                                inner: CodeState::Normal,
                            }
                        } else {
                            LexState::MkText
                        };
                    } else if line[i..].starts_with("/>") {
                        spans.push((i, i + 2, TokenKind::Tag));
                        i += 2;
                        fold -= 1;
                        state = LexState::MkText;
                    } else if c == b'"' || c == b'\'' {
                        state = LexState::MkAttrStr {
                            delim: c as char,
                            script,
                        };
                        spans.push((i, i + 1, TokenKind::AttrValue));
                        i += 1;
                    } else if c == b'=' {
                        spans.push((i, i + 1, TokenKind::Operator));
                        i += 1;
                    } else if c.is_ascii_whitespace() {
                        i += 1;
                    } else {
                        let s = i;
                        while i < b.len()
                            && !b[i].is_ascii_whitespace()
                            && !matches!(b[i], b'=' | b'>' | b'"' | b'\'')
                            && !line[i..].starts_with("/>")
                        {
                            i += 1;
                        }
                        if i == s {
                            i += 1;
                        }
                        spans.push((s, i, TokenKind::Attribute));
                    }
                }
                LexState::MkText | LexState::Code(_) | LexState::Fenced => {
                    let rest = &line[i..];
                    if rest.starts_with("<!--") {
                        state = LexState::MkComment;
                        spans.push((i, i + 4, TokenKind::Comment));
                        i += 4;
                    } else if rest.starts_with("<![CDATA[") {
                        state = LexState::MkCData;
                        spans.push((i, i + 9, TokenKind::String));
                        i += 9;
                    } else if rest.starts_with("<?") {
                        if self.is_php && !rest.starts_with("<?xml") {
                            let len = if rest.to_ascii_lowercase().starts_with("<?php") {
                                5
                            } else if rest.starts_with("<?=") {
                                3
                            } else {
                                2
                            };
                            spans.push((i, i + len, TokenKind::Preprocessor));
                            i += len;
                            state = LexState::MkPhp(CodeState::Normal);
                        } else {
                            state = LexState::MkPi;
                            spans.push((i, i + 2, TokenKind::Preprocessor));
                            i += 2;
                        }
                    } else if b[i] == b'<'
                        && i + 1 < b.len()
                        && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'/' || b[i + 1] == b'!')
                    {
                        let s = i;
                        i += 1;
                        let closing = b[i] == b'/';
                        if closing {
                            i += 1;
                        }
                        let ns = i;
                        while i < b.len()
                            && (b[i].is_ascii_alphanumeric()
                                || matches!(b[i], b'-' | b'_' | b':' | b'.' | b'!'))
                        {
                            i += 1;
                        }
                        let name = lower[ns..i].to_string();
                        spans.push((s, i, TokenKind::Tag));
                        let void = self.is_html && HTML_VOID.contains(&name.as_str());
                        if closing {
                            fold -= 1;
                        } else if !void && !name.starts_with('!') {
                            fold += 1;
                        }
                        let script = if !closing && self.is_html && name == "script" {
                            1
                        } else if !closing && self.is_html && name == "style" {
                            2
                        } else {
                            0
                        };
                        state = LexState::MkTag { script };
                    } else if b[i] == b'&' {
                        let s = i;
                        i += 1;
                        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'#') {
                            i += 1;
                        }
                        if i < b.len() && b[i] == b';' {
                            i += 1;
                            spans.push((s, i, TokenKind::Entity));
                        }
                    } else {
                        i += utf8_len(b[i]);
                    }
                }
            }
        }
        (state, fold)
    }
}

static EMPTY_KW: Keywords = Keywords {
    classes: Vec::new(),
    case_insensitive: false,
};

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

fn starts_with_ci(hay: &str, needle: &str) -> bool {
    hay.len() >= needle.len()
        && hay.is_char_boundary(needle.len())
        && hay[..needle.len()].eq_ignore_ascii_case(needle)
}

/// Lexes `line[start..]` as code. Stops early (returning the stop position)
/// if `stop` is found outside of a string/comment.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn lex_code(
    rules: &CodeRules,
    kw: &Keywords,
    line: &str,
    start: usize,
    mut cs: CodeState,
    stop: Option<&str>,
    spans: &mut Vec<Span>,
    fold: &mut i32,
) -> (CodeState, Option<usize>) {
    let b = line.as_bytes();
    let p = &rules.profile;
    let mut i = start;
    let first_non_blank = line
        .char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(i, _)| i);
    while i < b.len() {
        match cs {
            CodeState::Block { doc } => {
                let end = rules.comment_end.as_deref().unwrap_or("*/");
                let kind = if doc {
                    TokenKind::CommentDoc
                } else {
                    TokenKind::Comment
                };
                if let Some(e) = line[i..].find(end) {
                    spans.push((i, i + e + end.len(), kind));
                    i += e + end.len();
                    cs = CodeState::Normal;
                } else {
                    spans.push((i, b.len(), kind));
                    return (cs, None);
                }
            }
            CodeState::Str { delim, triple } => {
                let s = i;
                let mut closed = false;
                while i < b.len() {
                    let c = b[i];
                    if p.escapes && c == b'\\' {
                        i += 2;
                        continue;
                    }
                    if c as char == delim {
                        if triple {
                            let d3: String = std::iter::repeat_n(delim, 3).collect();
                            if line[i..].starts_with(&d3) {
                                i += 3;
                                closed = true;
                                break;
                            }
                        } else {
                            i += 1;
                            closed = true;
                            break;
                        }
                    }
                    i += utf8_len(c);
                }
                let i2 = i.min(b.len());
                let kind = if delim == '\'' && !triple {
                    TokenKind::Char
                } else {
                    TokenKind::String
                };
                spans.push((s, i2, kind));
                i = i2;
                if closed {
                    cs = CodeState::Normal;
                } else {
                    let continues = triple
                        || delim == '`'
                        || rules.multiline_strings
                        || (p.escapes && line.ends_with('\\'));
                    return (if continues { cs } else { CodeState::Normal }, None);
                }
            }
            CodeState::Normal => {
                let rest = &line[i..];
                if let Some(st) = stop
                    && rest.starts_with(st)
                {
                    return (cs, Some(i));
                }
                let c = rest.chars().next().unwrap_or(' ');
                // Line comments.
                if let Some(cl) = rules.comment_line.as_deref() {
                    let hit = if rules.comment_line_is_word {
                        starts_with_ci(rest, cl)
                            && rest[cl.len()..]
                                .chars()
                                .next()
                                .is_none_or(|n| !n.is_alphanumeric())
                            && (i == 0
                                || !is_ident_char(line[..i].chars().last().unwrap_or(' '), ""))
                    } else {
                        rest.starts_with(cl)
                    };
                    if hit {
                        let doc =
                            rules.rust_like && (rest.starts_with("///") || rest.starts_with("//!"));
                        spans.push((
                            i,
                            b.len(),
                            if doc {
                                TokenKind::CommentDoc
                            } else {
                                TokenKind::CommentLine
                            },
                        ));
                        return (cs, None);
                    }
                }
                if let Some(cstart) = rules.comment_start.as_deref()
                    && rest.starts_with(cstart)
                {
                    let doc =
                        cstart == "/*" && rest.starts_with("/**") && !rest.starts_with("/**/");
                    spans.push((
                        i,
                        i + cstart.len(),
                        if doc {
                            TokenKind::CommentDoc
                        } else {
                            TokenKind::Comment
                        },
                    ));
                    i += cstart.len();
                    cs = CodeState::Block { doc };
                    continue;
                }
                // Preprocessor directives.
                if let Some(pp) = p.preprocessor
                    && c == pp
                    && first_non_blank == Some(i)
                {
                    let mut e = b.len();
                    for marker in ["//", "/*"] {
                        if let Some(m) = rest.find(marker) {
                            e = e.min(i + m);
                        }
                    }
                    spans.push((i, e, TokenKind::Preprocessor));
                    i = e;
                    continue;
                }
                if p.string_delims.contains(c) {
                    if rules.rust_like && c == '\'' {
                        // Distinguish char literals from lifetimes.
                        let mut it = rest.char_indices().skip(1);
                        let is_char = match it.next() {
                            Some((_, '\\')) => true,
                            Some((_, _)) => it.next().is_some_and(|(_, n)| n == '\''),
                            None => false,
                        };
                        if !is_char {
                            let s = i;
                            i += 1;
                            while i < b.len() && is_ident_char(b[i] as char, "") {
                                i += 1;
                            }
                            spans.push((s, i, TokenKind::Operator));
                            continue;
                        }
                    }
                    let triple =
                        p.triple_quotes && (rest.starts_with("\"\"\"") || rest.starts_with("'''"));
                    let s = i;
                    i += if triple { 3 } else { 1 };
                    cs = CodeState::Str { delim: c, triple };
                    // Re-enter the loop in string state; mark the opening quote.
                    spans.push((
                        s,
                        i,
                        if c == '\'' && !triple {
                            TokenKind::Char
                        } else {
                            TokenKind::String
                        },
                    ));
                    continue;
                }
                if p.dollar_vars && c == '$' {
                    let s = i;
                    i += 1;
                    if line[i..].starts_with('{') {
                        if let Some(e) = line[i..].find('}') {
                            i += e + 1;
                        }
                    } else {
                        let start_var = i;
                        while i < b.len() && is_ident_char(b[i] as char, "") {
                            i += 1;
                        }
                        if i == start_var && i < b.len() {
                            i += 1; // $1, $#, $? ...
                        }
                    }
                    spans.push((s, i, TokenKind::Variable));
                    continue;
                }
                if c.is_ascii_digit()
                    || (c == '.' && rest[1..].chars().next().is_some_and(|n| n.is_ascii_digit()))
                {
                    let prev_ident = i > 0
                        && is_ident_char(line[..i].chars().last().unwrap_or(' '), p.ident_extra);
                    if !prev_ident {
                        let s = i;
                        i += 1;
                        while i < b.len() {
                            let ch = b[i];
                            let exp_sign = (ch == b'-' || ch == b'+')
                                && matches!(b[i - 1], b'e' | b'E')
                                && !line[s..i].starts_with("0x");
                            if ch.is_ascii_alphanumeric()
                                || ch == b'_'
                                || exp_sign
                                || (ch == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit())
                            {
                                i += 1;
                            } else {
                                break;
                            }
                        }
                        spans.push((s, i, TokenKind::Number));
                        continue;
                    }
                }
                if is_ident_char(c, p.ident_extra) && !c.is_ascii_digit() {
                    let s = i;
                    for (off, ch) in rest.char_indices() {
                        if !is_ident_char(ch, p.ident_extra) {
                            i = s + off;
                            break;
                        }
                        i = s + off + ch.len_utf8();
                    }
                    let word = &line[s..i];
                    if p.fold == FoldMode::Keywords {
                        let matches = |list: &[&str]| {
                            list.iter().any(|k| {
                                if p.case_insensitive {
                                    k.eq_ignore_ascii_case(word)
                                } else {
                                    *k == word
                                }
                            })
                        };
                        if matches(p.fold_open) {
                            *fold += 1;
                        } else if matches(p.fold_close) {
                            *fold -= 1;
                        }
                    }
                    if let Some(k) = kw.lookup(word) {
                        spans.push((s, i, TokenKind::Keyword(k)));
                    }
                    continue;
                }
                if c.is_ascii_punctuation() {
                    match c {
                        '{' if matches!(p.fold, FoldMode::Braces | FoldMode::Keywords) => {
                            *fold += 1
                        }
                        '}' if matches!(p.fold, FoldMode::Braces | FoldMode::Keywords) => {
                            *fold -= 1
                        }
                        '[' if p.fold == FoldMode::Braces && rules.comment_line.is_none() => {
                            *fold += 1
                        }
                        ']' if p.fold == FoldMode::Braces && rules.comment_line.is_none() => {
                            *fold -= 1
                        }
                        _ => {}
                    }
                    spans.push((i, i + 1, TokenKind::Operator));
                    i += 1;
                    continue;
                }
                i += c.len_utf8();
            }
        }
    }
    (cs, None)
}

fn lex_ini(line: &str, rules: &CodeRules, spans: &mut Vec<Span>) -> LexState {
    let t = line.trim_start();
    let off = line.len() - t.len();
    if t.starts_with('[') {
        let e = t.find(']').map_or(line.len(), |p| off + p + 1);
        spans.push((off, e, TokenKind::Section));
    } else if t.starts_with(';')
        || t.starts_with('#')
        || rules
            .comment_line
            .as_deref()
            .is_some_and(|c| t.starts_with(c))
    {
        spans.push((off, line.len(), TokenKind::CommentLine));
    } else if let Some(p) = t.find(['=', ':']) {
        spans.push((off, off + p, TokenKind::Key));
        spans.push((off + p, off + p + 1, TokenKind::Assign));
    }
    LexState::Code(CodeState::Normal)
}

fn lex_diff(line: &str, spans: &mut Vec<Span>) {
    let kind = if line.starts_with("+++") || line.starts_with("---") {
        TokenKind::DiffHeader
    } else if line.starts_with("diff ") || line.starts_with("index ") || line.starts_with("Index:")
    {
        TokenKind::DiffHeader
    } else if line.starts_with("@@") || line.starts_with("***") {
        TokenKind::DiffPosition
    } else if line.starts_with('+') || line.starts_with('>') {
        TokenKind::DiffAdd
    } else if line.starts_with('-') || line.starts_with('<') {
        TokenKind::DiffDel
    } else {
        return;
    };
    spans.push((0, line.len(), kind));
}

fn lex_markdown(line: &str, state: LexState, spans: &mut Vec<Span>) -> LexState {
    let t = line.trim_start();
    if t.starts_with("```") || t.starts_with("~~~") {
        spans.push((0, line.len(), TokenKind::Code));
        return if state == LexState::Fenced {
            LexState::Code(CodeState::Normal)
        } else {
            LexState::Fenced
        };
    }
    if state == LexState::Fenced {
        spans.push((0, line.len(), TokenKind::Code));
        return state;
    }
    if t.starts_with('#') {
        spans.push((0, line.len(), TokenKind::Heading));
        return state;
    }
    if t.starts_with('>') {
        spans.push((0, line.len(), TokenKind::Comment));
        return state;
    }
    let off = line.len() - t.len();
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        spans.push((off, off + 1, TokenKind::Operator));
    }
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'`' => {
                if let Some(e) = line[i + 1..].find('`') {
                    spans.push((i, i + e + 2, TokenKind::Code));
                    i += e + 2;
                    continue;
                }
            }
            b'*' | b'_' => {
                let d = b[i];
                let dbl = i + 1 < b.len() && b[i + 1] == d;
                let marker = if dbl { 2 } else { 1 };
                let needle = &line[i..i + marker];
                if i + marker < b.len()
                    && !b[i + marker].is_ascii_whitespace()
                    && let Some(e) = line[i + marker..].find(needle)
                    && e > 0
                {
                    spans.push((
                        i,
                        i + marker + e + marker,
                        if dbl {
                            TokenKind::Heading
                        } else {
                            TokenKind::Emphasis
                        },
                    ));
                    i += marker + e + marker;
                    continue;
                }
            }
            b'[' => {
                if let Some(close) = line[i..].find("](")
                    && let Some(end) = line[i + close..].find(')')
                {
                    spans.push((i, i + close + end + 1, TokenKind::Link));
                    i += close + end + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += utf8_len(b[i]);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(lang: &str, lines: &[&str]) -> Vec<Vec<Span>> {
        let reg = LangRegistry::load();
        let l = &reg.langs[reg.by_name(lang).unwrap()];
        let lexer = Lexer::new(l, &reg);
        let mut st = lexer.initial_state();
        let mut out = Vec::new();
        for line in lines {
            let mut spans = Vec::new();
            st = lexer.lex_line(line, st, &mut spans).0;
            out.push(spans);
        }
        out
    }

    fn kind_at(spans: &[Span], pos: usize) -> TokenKind {
        spans
            .iter()
            .find(|(s, e, _)| *s <= pos && pos < *e)
            .map_or(TokenKind::Default, |s| s.2)
    }

    #[test]
    fn cpp_tokens() {
        let r = lex(
            "cpp",
            &[
                "int x = 42; // hi",
                "/* a",
                " b */ return \"s\";",
                "#include <x>",
            ],
        );
        assert!(matches!(kind_at(&r[0], 0), TokenKind::Keyword(_)));
        assert_eq!(kind_at(&r[0], 8), TokenKind::Number);
        assert_eq!(kind_at(&r[0], 13), TokenKind::CommentLine);
        assert_eq!(kind_at(&r[1], 3), TokenKind::Comment);
        assert_eq!(kind_at(&r[2], 1), TokenKind::Comment);
        assert_eq!(kind_at(&r[2], 14), TokenKind::String);
        assert_eq!(kind_at(&r[3], 2), TokenKind::Preprocessor);
    }

    #[test]
    fn python_triple_quotes_span_lines() {
        let r = lex("python", &["x = '''abc", "def", "'''", "def f(): pass"]);
        assert_eq!(kind_at(&r[1], 1), TokenKind::String);
        assert!(matches!(kind_at(&r[3], 0), TokenKind::Keyword(_)));
    }

    #[test]
    fn rust_lifetimes_are_not_chars() {
        let r = lex("rust", &["fn f<'a>(x: &'a str) -> char { 'c' }"]);
        assert_eq!(kind_at(&r[0], 31), TokenKind::Char);
        assert_ne!(kind_at(&r[0], 16), TokenKind::Char);
    }

    #[test]
    fn markup_fold_and_tags() {
        let reg = LangRegistry::load();
        let l = &reg.langs[reg.by_name("html").unwrap()];
        let lexer = Lexer::new(l, &reg);
        let mut spans = Vec::new();
        let (st, f) = lexer.lex_line("<div class=\"a\"><br>", lexer.initial_state(), &mut spans);
        assert_eq!(st, LexState::MkText);
        assert_eq!(f, 1);
        assert_eq!(kind_at(&spans, 1), TokenKind::Tag);
        assert_eq!(kind_at(&spans, 6), TokenKind::Attribute);
        assert_eq!(kind_at(&spans, 12), TokenKind::AttrValue);
        let (_, f2) = lexer.lex_line("</div>", st, &mut spans);
        assert_eq!(f2, -1);
    }

    #[test]
    fn brace_folding() {
        let reg = LangRegistry::load();
        let l = &reg.langs[reg.by_name("cpp").unwrap()];
        let lexer = Lexer::new(l, &reg);
        let mut spans = Vec::new();
        assert_eq!(
            lexer
                .lex_line("int main() {", lexer.initial_state(), &mut spans)
                .1,
            1
        );
        assert_eq!(
            lexer
                .lex_line("  s = \"{\"; }", lexer.initial_state(), &mut spans)
                .1,
            -1
        );
    }

    #[test]
    fn batch_rem_comment_case_insensitive() {
        let r = lex("batch", &["rem hello", "echo remove"]);
        assert_eq!(kind_at(&r[0], 5), TokenKind::CommentLine);
        assert_ne!(kind_at(&r[1], 6), TokenKind::CommentLine);
    }
}
