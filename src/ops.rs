//! Editing operations on a document + its view selections.
//!
//! These are the editor key commands and Edit-menu
//! commands, expressed over our `Document` and multi-selection model.

use crate::document::{Document, EditKind, Selection, normalize_selections};
use crate::editor::ViewState;
use crate::encoding::Eol;

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Space,
    Word,
    Punct,
    Eol,
}

fn class_of(c: char) -> CharClass {
    if c == '\n' || c == '\r' {
        CharClass::Eol
    } else if c.is_whitespace() {
        CharClass::Space
    } else if is_word_char(c) {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

/// Range of the word at `pos` (or empty range if none).
pub fn word_at(doc: &Document, pos: usize) -> (usize, usize) {
    let len = doc.len_chars();
    let at = |p: usize| doc.char_at(p).is_some_and(is_word_char);
    let mut s = pos.min(len);
    let mut e = pos.min(len);
    if !at(s) && s > 0 && at(s - 1) {
        s -= 1;
        e = s;
    }
    if !at(s) {
        return (pos, pos);
    }
    while s > 0 && at(s - 1) {
        s -= 1;
    }
    while e < len && at(e) {
        e += 1;
    }
    (s, e)
}

pub fn word_left(doc: &Document, pos: usize) -> usize {
    let mut p = pos;
    // Skip whitespace, then a run of one class.
    while p > 0
        && doc
            .char_at(p - 1)
            .is_some_and(|c| class_of(c) == CharClass::Space)
    {
        p -= 1;
    }
    if p > 0 {
        let cls = class_of(doc.char_at(p - 1).unwrap_or(' '));
        if cls == CharClass::Eol {
            return step_left(doc, p);
        }
        while p > 0 && doc.char_at(p - 1).is_some_and(|c| class_of(c) == cls) {
            p -= 1;
        }
    }
    p
}

pub fn word_right(doc: &Document, pos: usize) -> usize {
    let len = doc.len_chars();
    let mut p = pos;
    if p < len {
        let cls = class_of(doc.char_at(p).unwrap_or(' '));
        if cls == CharClass::Eol {
            return step_right(doc, p);
        }
        while p < len && doc.char_at(p).is_some_and(|c| class_of(c) == cls) {
            p += 1;
        }
    }
    while p < len
        && doc
            .char_at(p)
            .is_some_and(|c| class_of(c) == CharClass::Space)
    {
        p += 1;
    }
    p
}

/// One position left, treating CRLF as a single unit.
pub fn step_left(doc: &Document, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    if pos >= 2 && doc.char_at(pos - 1) == Some('\n') && doc.char_at(pos - 2) == Some('\r') {
        pos - 2
    } else {
        pos - 1
    }
}

pub fn step_right(doc: &Document, pos: usize) -> usize {
    let len = doc.len_chars();
    if pos >= len {
        return len;
    }
    if doc.char_at(pos) == Some('\r') && doc.char_at(pos + 1) == Some('\n') {
        pos + 2
    } else {
        pos + 1
    }
}

fn carets_at_ends(finals: &[(usize, usize)]) -> Vec<Selection> {
    finals.iter().map(|&(_, e)| Selection::caret(e)).collect()
}

/// Replaces every selection with `text` (multi-caret typing / paste).
pub fn insert_text(doc: &mut Document, vs: &mut ViewState, text: &str, kind: EditKind) {
    if doc.read_only {
        return;
    }
    let before = vs.sels.clone();
    let edits = vs
        .sels
        .iter()
        .map(|s| (s.start(), s.end(), text.to_string()))
        .collect();
    vs.sels = doc.edit(edits, &before, kind, carets_at_ends);
    vs.after_edit(doc);
}

/// Inserts different text at each selection (multi-paste).
pub fn insert_per_selection(doc: &mut Document, vs: &mut ViewState, texts: &[String]) {
    if doc.read_only {
        return;
    }
    let before = vs.sels.clone();
    let mut sorted = vs.sels.clone();
    sorted.sort_by_key(|s| s.start());
    let edits = sorted
        .iter()
        .zip(texts.iter())
        .map(|(s, t)| (s.start(), s.end(), t.clone()))
        .collect();
    vs.sels = doc.edit(edits, &before, EditKind::Other, carets_at_ends);
    vs.after_edit(doc);
}

const PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\'')];

/// Typed text: handles overtype and auto-closing of brackets/quotes.
pub fn type_text(
    doc: &mut Document,
    vs: &mut ViewState,
    text: &str,
    overtype: bool,
    auto_brackets: bool,
    auto_quotes: bool,
) {
    if doc.read_only {
        return;
    }
    let mut chars = text.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    // Typing a closing char right before the same auto-inserted closer: just step over it.
    if let Some(c) = single
        && (auto_brackets || auto_quotes)
        && PAIRS.iter().any(|p| p.1 == c)
        && vs
            .sels
            .iter()
            .all(|s| s.is_empty() && doc.char_at(s.head) == Some(c))
        && (c != '"' && c != '\'' || auto_quotes)
    {
        for s in &mut vs.sels {
            *s = Selection::caret(s.head + 1);
        }
        return;
    }
    let before = vs.sels.clone();
    let pair = single.and_then(|c| {
        PAIRS.iter().find(|p| p.0 == c).filter(|p| {
            let is_quote = p.0 == '"' || p.0 == '\'';
            if is_quote { auto_quotes } else { auto_brackets }
        })
    });
    let mut edits = Vec::new();
    for s in &vs.sels {
        let mut end = s.end();
        if overtype && s.is_empty() {
            let (line, _) = doc.pos_to_line_col(s.head);
            if s.head < doc.line_end(line) {
                end = s.head + 1;
            }
        }
        let t = match pair {
            Some(&(open, close)) if s.is_empty() => {
                // Don't auto-close a quote inside a word.
                let prev_word = s.head > 0 && doc.char_at(s.head - 1).is_some_and(is_word_char);
                if (open == '"' || open == '\'') && prev_word {
                    open.to_string()
                } else {
                    format!("{open}{close}")
                }
            }
            Some(&(open, close)) => format!("{open}{}{close}", doc.slice(s.start(), s.end())),
            None => text.to_string(),
        };
        edits.push((s.start(), end, t));
    }
    let had_selection: Vec<bool> = {
        let mut sorted = vs.sels.clone();
        sorted.sort_by_key(|s| s.start());
        sorted.iter().map(|s| !s.is_empty()).collect()
    };
    let paired = pair.is_some();
    vs.sels = doc.edit(edits, &before, EditKind::Typing, |finals| {
        finals
            .iter()
            .enumerate()
            .map(|(i, &(s, e))| {
                if paired && had_selection.get(i).copied().unwrap_or(false) {
                    Selection::new(s + 1, e - 1)
                } else if paired && e - s == 2 {
                    Selection::caret(s + 1)
                } else {
                    Selection::caret(e)
                }
            })
            .collect()
    });
    vs.after_edit(doc);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteUnit {
    CharBack,
    CharForward,
    WordBack,
    WordForward,
    LineStart,
    LineEnd,
}

pub fn delete(doc: &mut Document, vs: &mut ViewState, unit: DeleteUnit) {
    if doc.read_only {
        return;
    }
    let before = vs.sels.clone();
    let any_sel = vs.sels.iter().any(|s| !s.is_empty());
    let edits: Vec<_> = vs
        .sels
        .iter()
        .map(|s| {
            if any_sel {
                return (s.start(), s.end(), String::new());
            }
            let p = s.head;
            let (line, _) = doc.pos_to_line_col(p);
            let (a, b) = match unit {
                DeleteUnit::CharBack => (step_left(doc, p), p),
                DeleteUnit::CharForward => (p, step_right(doc, p)),
                DeleteUnit::WordBack => (word_left(doc, p), p),
                DeleteUnit::WordForward => (p, word_right(doc, p)),
                DeleteUnit::LineStart => (doc.line_start(line), p),
                DeleteUnit::LineEnd => (p, doc.line_end(line)),
            };
            (a, b, String::new())
        })
        .collect();
    let kind = if matches!(unit, DeleteUnit::CharBack | DeleteUnit::CharForward) && !any_sel {
        EditKind::Typing
    } else {
        EditKind::Other
    };
    vs.sels = doc.edit(edits, &before, kind, carets_at_ends);
    normalize_selections(&mut vs.sels);
    vs.after_edit(doc);
}

fn leading_ws(s: &str) -> &str {
    &s[..s.len() - s.trim_start_matches([' ', '\t']).len()]
}

/// Enter: inserts EOL with auto-indent.
pub fn newline(doc: &mut Document, vs: &mut ViewState, auto_indent: bool) {
    if doc.read_only {
        return;
    }
    let before = vs.sels.clone();
    let eol = doc.eol.as_str();
    let edits = vs
        .sels
        .iter()
        .map(|s| {
            let (line, _) = doc.pos_to_line_col(s.start());
            let indent = if auto_indent {
                let text = doc.line_text(line);
                let col = s.start() - doc.line_start(line);
                let ws = leading_ws(&text);
                ws.chars().take(col).collect::<String>()
            } else {
                String::new()
            };
            (s.start(), s.end(), format!("{eol}{indent}"))
        })
        .collect();
    vs.sels = doc.edit(edits, &before, EditKind::Other, carets_at_ends);
    vs.after_edit(doc);
}

/// Lines covered by a selection; a selection ending at column 0 doesn't
/// include that last line (like other line-based commands).
pub fn sel_lines(doc: &Document, s: Selection) -> (usize, usize) {
    let (l1, _) = doc.pos_to_line_col(s.start());
    let (mut l2, c2) = doc.pos_to_line_col(s.end());
    if !s.is_empty() && c2 == 0 && l2 > l1 {
        l2 -= 1;
    }
    (l1, l2)
}

/// Union of lines covered by all selections.
pub fn all_sel_lines(doc: &Document, vs: &ViewState) -> Vec<usize> {
    let mut lines: Vec<usize> = vs
        .sels
        .iter()
        .flat_map(|s| {
            let (a, b) = sel_lines(doc, *s);
            a..=b
        })
        .collect();
    lines.sort_unstable();
    lines.dedup();
    lines
}

pub fn indent_unit(tab: usize, use_spaces: bool) -> String {
    if use_spaces {
        " ".repeat(tab)
    } else {
        "\t".into()
    }
}

/// Tab key: multi-line selection indents lines, otherwise inserts a tab.
pub fn tab(doc: &mut Document, vs: &mut ViewState, tab: usize, use_spaces: bool) {
    if doc.read_only {
        return;
    }
    let multiline = vs.sels.iter().any(|s| {
        let (a, b) = sel_lines(doc, *s);
        a != b
    });
    if multiline {
        indent_lines(doc, vs, tab, use_spaces, true);
        return;
    }
    if use_spaces {
        let before = vs.sels.clone();
        let edits = vs
            .sels
            .iter()
            .map(|s| {
                let col = vs.display_col(doc, s.start());
                let n = tab - col % tab.max(1);
                (s.start(), s.end(), " ".repeat(n))
            })
            .collect();
        vs.sels = doc.edit(edits, &before, EditKind::Typing, carets_at_ends);
        vs.after_edit(doc);
    } else {
        insert_text(doc, vs, "\t", EditKind::Typing);
    }
}

pub fn indent_lines(
    doc: &mut Document,
    vs: &mut ViewState,
    tab: usize,
    use_spaces: bool,
    indent: bool,
) {
    if doc.read_only {
        return;
    }
    let lines = all_sel_lines(doc, vs);
    let unit = indent_unit(tab, use_spaces);
    let before = vs.sels.clone();
    let mut edits = Vec::new();
    for &l in &lines {
        let start = doc.line_start(l);
        if indent {
            if doc.line_len(l) > 0 || lines.len() == 1 {
                edits.push((start, start, unit.clone()));
            }
        } else {
            let text = doc.line_text(l);
            let ws = leading_ws(&text);
            // Remove up to one indent level.
            let mut remove = 0;
            let mut width = 0;
            for c in ws.chars() {
                if width >= tab {
                    break;
                }
                remove += 1;
                if c == '\t' {
                    break;
                }
                width += 1;
            }
            if remove > 0 {
                edits.push((start, start + remove, String::new()));
            }
        }
    }
    if edits.is_empty() {
        return;
    }
    // Keep selections spanning the same lines.
    let old = vs.sels.clone();
    let ver = doc.version;
    doc.edit(edits, &before, EditKind::Other, |_| old.clone());
    vs.sels = old
        .iter()
        .map(|s| {
            let a = doc.map_pos(s.anchor, ver);
            let h = doc.map_pos(s.head, ver);
            Selection::new(a, h)
        })
        .collect();
    // Extend multi-line selections to full lines.
    for s in &mut vs.sels {
        if !s.is_empty() {
            let (l1, l2) = sel_lines(doc, *s);
            let a = doc.line_start(l1);
            let b = if l2 + 1 < doc.line_count() {
                doc.line_start(l2 + 1)
            } else {
                doc.line_end(l2)
            };
            *s = if s.head < s.anchor {
                Selection::new(b, a)
            } else {
                Selection::new(a, b)
            };
        }
    }
    vs.after_edit(doc);
}

/// Replaces lines `first..=last` (text only, keeps the final EOL) and selects the result.
pub fn replace_lines(
    doc: &mut Document,
    vs: &mut ViewState,
    first: usize,
    last: usize,
    new_lines: &[String],
) {
    if doc.read_only {
        return;
    }
    let start = doc.line_start(first);
    let end = doc.line_end(last);
    let eol = doc.eol.as_str();
    let text = new_lines.join(eol);
    let before = vs.sels.clone();
    vs.sels = doc.edit(vec![(start, end, text)], &before, EditKind::Other, |f| {
        vec![Selection::new(f[0].0, f[0].1)]
    });
    vs.after_edit(doc);
}

/// Range of lines an Edit > Line Operations command applies to: the
/// selected lines, or the whole document when nothing is selected.
pub fn target_lines(doc: &Document, vs: &ViewState) -> (usize, usize) {
    let s = vs.primary();
    if s.is_empty() || vs.sels.len() > 1 && vs.sels.iter().all(|s| s.is_empty()) {
        (0, doc.line_count().saturating_sub(1))
    } else {
        let a = vs
            .sels
            .iter()
            .map(|s| sel_lines(doc, *s).0)
            .min()
            .unwrap_or(0);
        let b = vs
            .sels
            .iter()
            .map(|s| sel_lines(doc, *s).1)
            .max()
            .unwrap_or(0);
        (a, b)
    }
}

pub fn transform_lines(
    doc: &mut Document,
    vs: &mut ViewState,
    f: impl FnOnce(Vec<String>) -> Vec<String>,
) {
    let (a, b) = target_lines(doc, vs);
    let lines: Vec<String> = (a..=b).map(|l| doc.line_text(l)).collect();
    let new = f(lines.clone());
    if new != lines {
        replace_lines(doc, vs, a, b, &new);
    }
}

pub fn transform_each_line(doc: &mut Document, vs: &mut ViewState, f: impl Fn(&str) -> String) {
    transform_lines(doc, vs, |lines| lines.iter().map(|l| f(l)).collect());
}

/// Applies `f` to the text of every non-empty selection.
pub fn transform_selections(doc: &mut Document, vs: &mut ViewState, f: impl Fn(&str) -> String) {
    if doc.read_only {
        return;
    }
    let before = vs.sels.clone();
    let edits: Vec<_> = vs
        .sels
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| (s.start(), s.end(), f(&doc.slice(s.start(), s.end()))))
        .collect();
    if edits.is_empty() {
        return;
    }
    vs.sels = doc.edit(edits, &before, EditKind::Other, |f| {
        f.iter().map(|&(s, e)| Selection::new(s, e)).collect()
    });
    vs.after_edit(doc);
}

pub fn duplicate(doc: &mut Document, vs: &mut ViewState) {
    if doc.read_only {
        return;
    }
    let before = vs.sels.clone();
    let eol = doc.eol.as_str().to_string();
    let mut edits = Vec::new();
    for s in &vs.sels {
        if s.is_empty() {
            let (l, _) = doc.pos_to_line_col(s.head);
            let end = doc.line_end(l);
            edits.push((end, end, format!("{eol}{}", doc.line_text(l))));
        } else {
            edits.push((s.end(), s.end(), doc.slice(s.start(), s.end())));
        }
    }
    let old = vs.sels.clone();
    doc.edit(edits, &before, EditKind::Other, |_| old.clone());
    vs.after_edit(doc);
}

pub fn delete_lines(doc: &mut Document, vs: &mut ViewState) {
    if doc.read_only {
        return;
    }
    let lines = all_sel_lines(doc, vs);
    let before = vs.sels.clone();
    let n = doc.line_count();
    let mut edits = Vec::new();
    for &l in &lines {
        let s = doc.line_start(l);
        let e = if l + 1 < n {
            doc.line_start(l + 1)
        } else {
            doc.len_chars()
        };
        // Deleting the last line also removes the preceding EOL.
        let s = if l + 1 >= n && l > 0 && !lines.contains(&(l - 1)) {
            doc.line_end(l - 1)
        } else {
            s
        };
        edits.push((s, e, String::new()));
    }
    vs.sels = doc.edit(edits, &before, EditKind::Other, |f| {
        vec![Selection::caret(f.first().map_or(0, |x| x.0))]
    });
    vs.after_edit(doc);
}

pub fn move_lines(doc: &mut Document, vs: &mut ViewState, up: bool) {
    if doc.read_only {
        return;
    }
    let (a, b) = {
        let lines = all_sel_lines(doc, vs);
        (*lines.first().unwrap_or(&0), *lines.last().unwrap_or(&0))
    };
    let n = doc.line_count();
    if (up && a == 0) || (!up && b + 1 >= n) {
        return;
    }
    let (lo, hi) = if up { (a - 1, b) } else { (a, b + 1) };
    let mut lines: Vec<String> = (lo..=hi).map(|l| doc.line_text(l)).collect();
    if up {
        lines.rotate_left(1);
    } else {
        lines.rotate_right(1);
    }
    let old: Vec<(usize, usize)> = vs
        .sels
        .iter()
        .map(|s| {
            let (al, ac) = doc.pos_to_line_col(s.anchor);
            let (hl, hc) = doc.pos_to_line_col(s.head);
            (al * 1_000_000 + ac, hl * 1_000_000 + hc)
        })
        .collect();
    replace_lines(doc, vs, lo, hi, &lines);
    let shift = |packed: usize| {
        let (l, c) = (packed / 1_000_000, packed % 1_000_000);
        let nl = if up { l - 1 } else { l + 1 };
        doc.line_start(nl) + c.min(doc.line_len(nl))
    };
    vs.sels = old
        .iter()
        .map(|&(a, h)| Selection::new(shift(a), shift(h)))
        .collect();
    vs.after_edit(doc);
}

pub fn transpose_line(doc: &mut Document, vs: &mut ViewState) {
    let (l, _) = doc.pos_to_line_col(vs.primary().head);
    if l == 0 {
        return;
    }
    let lines = vec![doc.line_text(l), doc.line_text(l - 1)];
    let col = vs.primary().head - doc.line_start(l);
    replace_lines(doc, vs, l - 1, l, &lines);
    let p = doc.line_start(l) + col.min(doc.line_len(l));
    vs.set_single(p);
}

pub fn join_lines(doc: &mut Document, vs: &mut ViewState) {
    let (a, mut b) = target_lines(doc, vs);
    if a == b {
        b = (a + 1).min(doc.line_count() - 1);
    }
    if a == b {
        return;
    }
    let mut out = doc.line_text(a).trim_end().to_string();
    for l in a + 1..=b {
        let t = doc.line_text(l);
        let t = t.trim_start();
        if !t.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(t);
        }
    }
    replace_lines(doc, vs, a, b, &[out]);
}

pub fn insert_blank_line(doc: &mut Document, vs: &mut ViewState, above: bool) {
    if doc.read_only {
        return;
    }
    let (l, _) = doc.pos_to_line_col(vs.primary().head);
    let eol = doc.eol.as_str().to_string();
    let before = vs.sels.clone();
    if above {
        let s = doc.line_start(l);
        vs.sels = doc.edit(vec![(s, s, eol)], &before, EditKind::Other, |f| {
            vec![Selection::caret(f[0].0)]
        });
    } else {
        let e = doc.line_end(l);
        vs.sels = doc.edit(vec![(e, e, eol)], &before, EditKind::Other, |f| {
            vec![Selection::caret(f[0].1)]
        });
    }
    vs.after_edit(doc);
}

pub fn convert_eol(doc: &mut Document, vs: &mut ViewState, eol: Eol) {
    let text = doc.text();
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let target = if eol == Eol::Lf {
        normalized
    } else {
        normalized.replace('\n', eol.as_str())
    };
    doc.eol = eol;
    if *text != target {
        let len = doc.len_chars();
        let caret = vs.primary().head;
        let (line, col) = doc.pos_to_line_col(caret);
        let before = vs.sels.clone();
        doc.edit(vec![(0, len, target)], &before, EditKind::Other, |_| {
            vec![Selection::caret(0)]
        });
        let p = doc.line_start(line) + col.min(doc.line_len(line));
        vs.set_single(p);
    }
    doc.force_dirty |= doc.is_dirty();
}

/// Finds the bracket matching the one at/before `pos`.
pub fn matching_brace(doc: &Document, pos: usize) -> Option<(usize, usize)> {
    let opens = ['(', '[', '{', '<'];
    let closes = [')', ']', '}', '>'];
    let check = |p: usize| -> Option<(usize, usize)> {
        let c = doc.char_at(p)?;
        let (idx, fwd) = if let Some(i) = opens.iter().position(|&o| o == c) {
            (i, true)
        } else {
            (closes.iter().position(|&o| o == c)?, false)
        };
        if idx == 3 {
            return None; // angle brackets only when explicitly on them? keep off for code
        }
        let (o, cl) = (opens[idx], closes[idx]);
        let mut depth = 0i64;
        let len = doc.len_chars();
        let limit = 2_000_000usize;
        if fwd {
            let mut q = p;
            let mut steps = 0;
            while q < len && steps < limit {
                let ch = doc.rope.char(q);
                if ch == o {
                    depth += 1;
                } else if ch == cl {
                    depth -= 1;
                    if depth == 0 {
                        return Some((p, q));
                    }
                }
                q += 1;
                steps += 1;
            }
        } else {
            let mut q = p as isize;
            let mut steps = 0;
            while q >= 0 && steps < limit {
                let ch = doc.rope.char(q.cast_unsigned());
                if ch == cl {
                    depth += 1;
                } else if ch == o {
                    depth -= 1;
                    if depth == 0 {
                        return Some((p, q.cast_unsigned()));
                    }
                }
                q -= 1;
                steps += 1;
            }
        }
        Some((p, usize::MAX))
    };
    if pos > 0
        && let Some(r) = check(pos - 1)
    {
        return Some(r);
    }
    check(pos)
}

/// Toggle single-line comment (Ctrl+Q) and friends.
pub fn line_comment(doc: &mut Document, vs: &mut ViewState, prefix: &str, mode: CommentMode) {
    let lines = all_sel_lines(doc, vs);
    let (Some(&a), Some(&b)) = (lines.first(), lines.last()) else {
        return;
    };
    let texts: Vec<String> = (a..=b).map(|l| doc.line_text(l)).collect();
    let new = match mode {
        CommentMode::Toggle => crate::textops::toggle_line_comment(&texts, prefix),
        CommentMode::Comment => crate::textops::comment_lines(&texts, prefix),
        CommentMode::Uncomment => texts
            .iter()
            .map(|l| crate::textops::uncomment_line(l, prefix))
            .collect(),
    };
    if new != texts {
        let caret_line_col = doc.pos_to_line_col(vs.primary().head);
        let single = vs.primary().is_empty();
        replace_lines(doc, vs, a, b, &new);
        if single {
            let (l, c) = caret_line_col;
            let delta = new[l - a].chars().count().cast_signed()
                - texts[l - a].chars().count().cast_signed();
            let col = (c.cast_signed() + delta).max(0).cast_unsigned();
            vs.set_single(doc.line_start(l) + col.min(doc.line_len(l)));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CommentMode {
    Toggle,
    Comment,
    Uncomment,
}

pub fn stream_comment(doc: &mut Document, vs: &mut ViewState, start: &str, end: &str, add: bool) {
    if doc.read_only {
        return;
    }
    let s = vs.primary();
    let (a, b) = if s.is_empty() {
        word_at(doc, s.head)
    } else {
        (s.start(), s.end())
    };
    let text = doc.slice(a, b);
    let before = vs.sels.clone();
    let new = if add {
        format!("{start}{text}{end}")
    } else {
        let t = text.trim();
        match t.strip_prefix(start).and_then(|x| x.strip_suffix(end)) {
            Some(inner) => inner.to_string(),
            None => {
                // Look around the selection for delimiters.
                let sl = start.chars().count();
                let el = end.chars().count();
                if a >= sl && doc.slice(a - sl, a) == start && doc.slice(b, b + el) == end {
                    vs.sels = doc.edit(
                        vec![(a - sl, a, String::new()), (b, b + el, String::new())],
                        &before,
                        EditKind::Other,
                        |f| vec![Selection::new(f[0].0, f[1].0)],
                    );
                    vs.after_edit(doc);
                }
                return;
            }
        }
    };
    vs.sels = doc.edit(vec![(a, b, new)], &before, EditKind::Other, |f| {
        vec![Selection::new(f[0].0, f[0].1)]
    });
    vs.after_edit(doc);
}

/// Multi-select: add the next occurrence of the main selection's text.
pub fn select_next_occurrence(
    doc: &Document,
    vs: &mut ViewState,
    match_case: bool,
    whole_word: bool,
    all: bool,
) {
    let p = vs.primary();
    let (a, b) = if p.is_empty() {
        word_at(doc, p.head)
    } else {
        (p.start(), p.end())
    };
    if a == b {
        return;
    }
    let needle = doc.slice(a, b);
    let opts = crate::search::SearchOptions {
        match_case,
        whole_word,
        ..Default::default()
    };
    let Ok(re) = crate::search::build_regex(&needle, &opts) else {
        return;
    };
    let text = doc.text();
    if p.is_empty() {
        vs.sels = vec![Selection::new(a, b)];
        vs.primary_idx = 0;
        if !all {
            return;
        }
    }
    let matches = crate::search::find_all(&re, &text);
    let to_sel =
        |&(s, e): &(usize, usize)| Selection::new(doc.byte_to_char(s), doc.byte_to_char(e));
    if all {
        vs.sels = matches.iter().map(to_sel).collect();
        vs.primary_idx = vs.sels.iter().position(|s| s.start() == a).unwrap_or(0);
    } else {
        let last_end = vs.sels.iter().map(|s| s.end()).max().unwrap_or(0);
        let next = matches
            .iter()
            .map(to_sel)
            .find(|s| s.start() >= last_end && !vs.sels.contains(s))
            .or_else(|| matches.iter().map(to_sel).find(|s| !vs.sels.contains(s)));
        if let Some(s) = next {
            vs.sels.push(s);
            vs.primary_idx = vs.sels.len() - 1;
        }
    }
    vs.scroll_to_caret = true;
}

/// Column Editor: inserts `texts[i]` at display column `col` of consecutive lines.
pub fn column_insert(
    doc: &mut Document,
    vs: &mut ViewState,
    first_line: usize,
    col: usize,
    texts: &[String],
) {
    if doc.read_only {
        return;
    }
    let before = vs.sels.clone();
    let mut edits = Vec::new();
    for (i, t) in texts.iter().enumerate() {
        let l = first_line + i;
        if l >= doc.line_count() {
            break;
        }
        let line = doc.line_text(l);
        let (pos, cur_col) = vs.col_to_char_in(&line, col);
        let pad = " ".repeat(col.saturating_sub(cur_col));
        let at = doc.line_start(l) + pos;
        edits.push((at, at, format!("{pad}{t}")));
    }
    vs.sels = doc.edit(edits, &before, EditKind::Other, carets_at_ends);
    vs.after_edit(doc);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langs::LangRegistry;

    fn setup(text: &str) -> (Document, ViewState) {
        let langs = LangRegistry::load();
        let d = Document::from_text(1, "t".into(), text, &langs, langs.normal());
        (d, ViewState::default())
    }

    #[test]
    fn typing_with_multiple_carets() {
        let (mut d, mut v) = setup("ab\ncd");
        v.sels = vec![Selection::caret(1), Selection::caret(4)];
        type_text(&mut d, &mut v, "X", false, false, false);
        assert_eq!(d.rope.to_string(), "aXb\ncXd");
        assert_eq!(v.sels, vec![Selection::caret(2), Selection::caret(6)]);
    }

    #[test]
    fn auto_close_and_step_over() {
        let (mut d, mut v) = setup("");
        type_text(&mut d, &mut v, "(", false, true, true);
        assert_eq!(d.rope.to_string(), "()");
        assert_eq!(v.sels, vec![Selection::caret(1)]);
        type_text(&mut d, &mut v, ")", false, true, true);
        assert_eq!(d.rope.to_string(), "()");
        assert_eq!(v.sels, vec![Selection::caret(2)]);
    }

    #[test]
    fn newline_keeps_indent() {
        let (mut d, mut v) = setup("    abc");
        v.set_single(7);
        newline(&mut d, &mut v, true);
        assert_eq!(d.rope.to_string(), "    abc\n    ");
    }

    #[test]
    fn duplicate_and_move() {
        let (mut d, mut v) = setup("one\ntwo\nthree");
        v.set_single(5);
        duplicate(&mut d, &mut v);
        assert_eq!(d.rope.to_string(), "one\ntwo\ntwo\nthree");
        move_lines(&mut d, &mut v, true);
        assert_eq!(d.rope.to_string(), "two\none\ntwo\nthree");
        assert_eq!(d.pos_to_line_col(v.primary().head).0, 0);
    }

    #[test]
    fn crlf_is_one_step() {
        let (mut d, mut v) = setup("a\r\nb");
        v.set_single(3);
        delete(&mut d, &mut v, DeleteUnit::CharBack);
        assert_eq!(d.rope.to_string(), "ab");
    }

    #[test]
    fn word_navigation() {
        let (d, _) = setup("foo.bar  baz");
        assert_eq!(word_right(&d, 0), 3);
        assert_eq!(word_right(&d, 3), 4);
        assert_eq!(word_right(&d, 4), 9);
        assert_eq!(word_left(&d, 9), 4);
        assert_eq!(word_at(&d, 5), (4, 7));
    }

    #[test]
    fn indent_and_unindent() {
        let (mut d, mut v) = setup("a\nb");
        v.sels = vec![Selection::new(0, 3)];
        tab(&mut d, &mut v, 4, true);
        assert_eq!(d.rope.to_string(), "    a\n    b");
        indent_lines(&mut d, &mut v, 4, true, false);
        assert_eq!(d.rope.to_string(), "a\nb");
    }

    #[test]
    fn braces_match() {
        let (d, _) = setup("f(a[1]) {}");
        assert_eq!(matching_brace(&d, 1), Some((1, 6)));
        assert_eq!(matching_brace(&d, 7), Some((6, 1)));
    }

    #[test]
    fn select_all_occurrences() {
        let (d, mut v) = setup("cat dog cat");
        v.set_single(1);
        select_next_occurrence(&d, &mut v, true, true, true);
        assert_eq!(v.sels, vec![Selection::new(0, 3), Selection::new(8, 11)]);
    }

    #[test]
    fn eol_conversion() {
        let (mut d, mut v) = setup("a\nb\nc");
        convert_eol(&mut d, &mut v, Eol::Crlf);
        assert_eq!(d.rope.to_string(), "a\r\nb\r\nc");
        assert_eq!(d.line_count(), 3);
    }
}
