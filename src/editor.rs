//! The editing widget.
//!
//! Renders a `Document` with syntax colours, margins (line numbers,
//! bookmarks, change history, folding), wrapping, multiple selections,
//! column selection, whitespace symbols, indent guides and highlights, and
//! turns keyboard/mouse input into edits. Monospace layout is assumed:
//! every char occupies 1 (or 2 for wide CJK) display columns.

use egui::{
    Color32, CornerRadius, CursorIcon, Event, FontId, Key, Modifiers, Pos2, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use unicode_width::UnicodeWidthChar;

use crate::commands::Command;
use crate::compare::LineKind;
use crate::config::Settings;
use crate::document::{Document, EditKind, Selection, normalize_selections};
use crate::lexer::TokenKind;
use crate::ops::{self, DeleteUnit};
use crate::theme::{Globals, LangStyles};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    Home,
    End,
    DocStart,
    DocEnd,
    PageUp,
    PageDown,
}

/// Editor key commands produced from keyboard input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Move { m: Move, extend: bool },
    RectMove(Move),
    Delete(DeleteUnit),
    Newline,
    Tab,
    BackTab,
    ToggleOvertype,
    Escape,
    ScrollLines(i32),
}

#[derive(Clone, Debug)]
pub struct AutoComplete {
    pub start: usize,
    pub items: Vec<String>,
    pub selected: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    Text,
    Word(usize, usize),
    Line(usize),
    Rect,
    Gutter(usize),
    VScroll { grab: f32 },
    HScroll { grab: f32 },
}

/// Clipboard metadata shared across views (line-copy and
/// rectangular-copy modes).
#[derive(Clone, Debug, Default)]
pub struct ClipState {
    pub text: String,
    pub line_mode: bool,
    pub rect_mode: bool,
    pub history: Vec<String>,
}

impl ClipState {
    fn record(&mut self, text: &str, line_mode: bool, rect_mode: bool) {
        self.text = text.to_string();
        self.line_mode = line_mode;
        self.rect_mode = rect_mode;
        self.history.retain(|h| h != text);
        self.history.insert(0, text.to_string());
        self.history.truncate(30);
    }
}

#[derive(Default)]
struct LayoutCache {
    key: (u64, u64, u64, usize, usize, usize, u64),
    /// Compare spacer rows before each visible line (`None` outside a compare).
    spacers: Option<Vec<u32>>,
    /// Visible (unfolded) doc lines; `None` = all lines.
    lines: Option<Vec<usize>>,
    /// First visual row of each visible line; `None` = 1 row per line.
    prefix: Option<Vec<usize>>,
}

pub struct ViewState {
    pub sels: Vec<Selection>,
    pub primary_idx: usize,
    pub seen_version: u64,
    pub top_row: f64,
    pub scroll_x: f32,
    pub preferred_col: Option<usize>,
    /// Column selection anchor/head as (line, display column).
    pub rect: Option<((usize, usize), (usize, usize))>,
    pub scroll_to_caret: bool,
    pub center_caret: bool,
    pub visible_rows: usize,
    pub tab_width: usize,
    pub autocomplete: Option<AutoComplete>,
    /// Parameter hint shown while typing a call: (position of its `(`, text).
    pub call_tip: Option<(usize, String)>,
    pub begin_select: Option<usize>,
    pub max_cols: usize,
    drag: Option<Drag>,
    last_press: Option<(f64, Pos2, u8)>,
    layout: LayoutCache,
    /// Last brace match: ((doc version, caret), result). Recomputed only when
    /// either changes; a match can span the whole document.
    brace_cache: Option<BraceCache>,
    wrap_cols: usize,
    pub last_text_rect: Rect,
    /// This view's side of an active file compare.
    pub compare: Option<std::sync::Arc<crate::compare::SideDecor>>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            sels: vec![Selection::caret(0)],
            primary_idx: 0,
            seen_version: 0,
            top_row: 0.0,
            scroll_x: 0.0,
            preferred_col: None,
            rect: None,
            scroll_to_caret: false,
            center_caret: false,
            visible_rows: 30,
            tab_width: 4,
            autocomplete: None,
            call_tip: None,
            begin_select: None,
            max_cols: 0,
            drag: None,
            last_press: None,
            layout: LayoutCache::default(),
            brace_cache: None,
            wrap_cols: 0,
            last_text_rect: Rect::NOTHING,
            compare: None,
        }
    }
}

/// ((document version, caret), matching brace pair) of the last lookup.
type BraceCache = ((u64, usize), Option<(usize, usize)>);

/// How far brace highlighting searches for the matching bracket (chars):
/// about 0.25 ms per keystroke, enough for blocks a few thousand lines long.
/// Go to Matching Brace still searches the whole document.
const BRACE_HIGHLIGHT_LIMIT: usize = 100_000;

/// Display columns of each char boundary in `line` (len = chars + 1).
pub fn columns(line: &str, tab: usize) -> Vec<usize> {
    let tab = tab.max(1);
    let mut cols = Vec::with_capacity(line.len() + 1);
    let mut c = 0;
    cols.push(0);
    for ch in line.chars() {
        c += char_cols(ch, c, tab);
        cols.push(c);
    }
    cols
}

fn char_cols(ch: char, col: usize, tab: usize) -> usize {
    if ch == '\t' {
        tab - col % tab
    } else if (ch as u32) < 0x20 || ch == '\u{7f}' {
        1
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(1)
    }
}

/// Char indices where each wrapped sub-row starts (first is always 0).
pub fn wrap_breaks(line: &str, tab: usize, max_cols: usize) -> Vec<usize> {
    let mut breaks = vec![0];
    if max_cols == 0 {
        return breaks;
    }
    let chars: Vec<char> = line.chars().collect();
    let cols = columns(line, tab);
    let mut row_start = 0;
    let mut last_space: Option<usize> = None;
    let mut i = 0;
    while i < chars.len() {
        let width_so_far = cols[i + 1] - cols[row_start];
        if width_so_far > max_cols && i > row_start && chars[i] != ' ' && chars[i] != '\t' {
            let brk = match last_space {
                Some(s) if s > row_start => s,
                _ => i,
            };
            breaks.push(brk);
            row_start = brk;
            last_space = None;
            i = brk;
            continue;
        }
        if chars[i] == ' ' || chars[i] == '\t' {
            last_space = Some(i + 1);
        }
        i += 1;
    }
    breaks
}

impl ViewState {
    pub fn primary(&self) -> Selection {
        self.sels
            .get(self.primary_idx)
            .or_else(|| self.sels.last())
            .copied()
            .unwrap_or_default()
    }

    pub fn set_single(&mut self, pos: usize) {
        self.sels = vec![Selection::caret(pos)];
        self.primary_idx = 0;
        self.rect = None;
        self.preferred_col = None;
        self.scroll_to_caret = true;
    }

    pub fn set_selection(&mut self, anchor: usize, head: usize) {
        self.sels = vec![Selection::new(anchor, head)];
        self.primary_idx = 0;
        self.rect = None;
        self.preferred_col = None;
        self.scroll_to_caret = true;
    }

    pub fn after_edit(&mut self, doc: &Document) {
        self.seen_version = doc.version;
        self.preferred_col = None;
        self.rect = None;
        self.scroll_to_caret = true;
        let len = doc.len_chars();
        for s in &mut self.sels {
            s.anchor = s.anchor.min(len);
            s.head = s.head.min(len);
        }
        if self.sels.is_empty() {
            self.sels.push(Selection::caret(0));
        }
        normalize_selections(&mut self.sels);
        self.primary_idx = self.primary_idx.min(self.sels.len() - 1);
    }

    /// Re-maps selections after edits made through another view.
    pub fn sync(&mut self, doc: &Document) {
        if self.seen_version != doc.version {
            let v = self.seen_version;
            for s in &mut self.sels {
                s.anchor = doc.map_pos(s.anchor, v);
                s.head = doc.map_pos(s.head, v);
            }
            self.seen_version = doc.version;
            normalize_selections(&mut self.sels);
            if self.sels.is_empty() {
                self.sels.push(Selection::caret(0));
            }
            self.primary_idx = self.primary_idx.min(self.sels.len() - 1);
        }
    }

    pub fn display_col(&self, doc: &Document, pos: usize) -> usize {
        let (line, col) = doc.pos_to_line_col(pos);
        let text = doc.line_text(line);
        let cols = columns(&text, self.tab_width);
        cols[col.min(cols.len() - 1)]
    }

    /// Char index in `line` nearest to display column `col`, and that char's column.
    pub fn col_to_char_in(&self, line: &str, col: usize) -> (usize, usize) {
        let cols = columns(line, self.tab_width);
        let mut best = cols.len() - 1;
        for (i, &c) in cols.iter().enumerate() {
            if c >= col {
                best = if i > 0 && col - cols[i - 1] < c - col {
                    i - 1
                } else {
                    i
                };
                break;
            }
        }
        (best, cols[best])
    }

    // ------------------------------------------------------------ layout

    /// The compare decoration, if it still matches the document.
    pub fn compare_decor(&self, doc: &Document) -> Option<&crate::compare::SideDecor> {
        self.compare
            .as_deref()
            .filter(|d| d.valid_for(doc.version, doc.line_count()))
    }

    /// Word wrap is off while comparing so both sides stay level.
    pub fn effective_wrap(&self, doc: &Document, wrap: bool) -> bool {
        wrap && self.compare_decor(doc).is_none()
    }

    fn ensure_layout(&mut self, doc: &mut Document, wrap: bool) {
        let decor = self.compare_decor(doc).cloned();
        let wrap = wrap && decor.is_none();
        let wrap_cols = if wrap { self.wrap_cols } else { 0 };
        let key = (
            doc.version,
            doc.fold_gen,
            doc.id,
            wrap_cols,
            self.tab_width,
            doc.line_count(),
            decor.as_ref().map_or(0, |d| d.generation),
        );
        if self.layout.key == key
            && (self.layout.lines.is_some()
                || self.layout.prefix.is_some()
                || (!doc.has_folds() && !wrap && decor.is_none() && doc.line_filter.is_none()))
        {
            return;
        }
        self.layout.key = key;
        let n = doc.line_count();
        self.layout.lines = if let Some(f) = &doc.line_filter
            && decor.is_none()
        {
            // An edit may have removed lines since the filter was computed.
            let mut v: Vec<usize> = f.iter().copied().take_while(|&l| l < n).collect();
            if v.is_empty() {
                v.push(0);
            }
            Some(v)
        } else if doc.has_folds() {
            let hidden = doc.hidden_ranges();
            let mut v = Vec::with_capacity(n);
            let mut hi = hidden.iter().peekable();
            let mut l = 0;
            while l < n {
                if let Some(&&(s, e)) = hi.peek()
                    && l >= s
                {
                    l = e + 1;
                    hi.next();
                    continue;
                }
                v.push(l);
                l += 1;
            }
            Some(v)
        } else {
            None
        };
        let count = self.layout.lines.as_ref().map_or(n, Vec::len);
        self.layout.spacers = decor.as_ref().map(|d| {
            (0..count)
                .map(|i| d.spacers_before[self.layout.lines.as_ref().map_or(i, |v| v[i])])
                .collect()
        });
        self.layout.prefix = if wrap_cols > 0 || decor.is_some() {
            let mut p = Vec::with_capacity(count + 1);
            let mut acc = 0;
            for i in 0..count {
                p.push(acc);
                let line = self.layout.lines.as_ref().map_or(i, |v| v[i]);
                acc += self.layout.spacers.as_ref().map_or(0, |s| s[i] as usize);
                acc += if wrap_cols == 0 {
                    1
                } else {
                    let text = doc.line_text(line);
                    if text.len() <= wrap_cols / 2 {
                        1
                    } else {
                        wrap_breaks(&text, self.tab_width, wrap_cols).len()
                    }
                };
            }
            // Spacers after the last line.
            acc += decor.as_ref().map_or(0, |d| d.spacers_before[n] as usize);
            p.push(acc);
            Some(p)
        } else {
            None
        };
    }

    fn visible_count(&self, doc: &Document) -> usize {
        self.layout
            .lines
            .as_ref()
            .map_or(doc.line_count(), Vec::len)
    }

    fn total_rows(&self, doc: &Document) -> usize {
        self.layout
            .prefix
            .as_ref()
            .map_or_else(|| self.visible_count(doc), |p| *p.last().unwrap_or(&0))
    }

    fn visible_index(&self, line: usize) -> usize {
        match &self.layout.lines {
            None => line,
            Some(v) => v.partition_point(|&l| l <= line).saturating_sub(1),
        }
    }

    fn line_of_index(&self, idx: usize) -> usize {
        self.layout
            .lines
            .as_ref()
            .map_or(idx, |v| v.get(idx).copied().unwrap_or(0))
    }

    fn spacers_at(&self, idx: usize) -> usize {
        self.layout
            .spacers
            .as_ref()
            .and_then(|s| s.get(idx))
            .map_or(0, |&x| x as usize)
    }

    fn line_first_row(&self, line: usize) -> usize {
        let idx = self.visible_index(line);
        self.layout
            .prefix
            .as_ref()
            .map_or(idx, |p| p[idx.min(p.len() - 1)] + self.spacers_at(idx))
    }

    /// Whether a visual row is a compare spacer (no text).
    fn is_spacer_row(&self, row: usize, doc: &Document) -> bool {
        let (Some(p), Some(_)) = (&self.layout.prefix, &self.layout.spacers) else {
            return false;
        };
        let count = self.visible_count(doc);
        let idx = p
            .partition_point(|&r| r <= row)
            .saturating_sub(1)
            .min(count.saturating_sub(1));
        let raw = row - p[idx];
        let sp = self.spacers_at(idx);
        raw < sp || raw > sp
    }

    /// (doc line, sub-row) of a visual row.
    fn row_to_line(&self, row: usize, doc: &Document) -> (usize, usize) {
        let count = self.visible_count(doc);
        match &self.layout.prefix {
            None => (self.line_of_index(row.min(count.saturating_sub(1))), 0),
            Some(p) => {
                let idx = p
                    .partition_point(|&r| r <= row)
                    .saturating_sub(1)
                    .min(count.saturating_sub(1));
                let raw = row - p[idx];
                (
                    self.line_of_index(idx),
                    raw.saturating_sub(self.spacers_at(idx)),
                )
            }
        }
    }

    fn breaks_for(&self, text: &str, wrap: bool) -> Vec<usize> {
        if wrap && self.wrap_cols > 0 && text.len() > self.wrap_cols / 2 {
            wrap_breaks(text, self.tab_width, self.wrap_cols)
        } else {
            vec![0]
        }
    }

    /// Visual row and column (relative to the row start) of a position.
    fn pos_to_row_col(&self, doc: &Document, pos: usize, wrap: bool) -> (usize, usize) {
        let (line, col) = doc.pos_to_line_col(pos);
        let text = doc.line_text(line);
        let breaks = self.breaks_for(&text, wrap);
        let cols = columns(&text, self.tab_width);
        let sub = breaks.partition_point(|&b| b <= col).saturating_sub(1);
        let row = self.line_first_row(line) + sub;
        (row, cols[col.min(cols.len() - 1)] - cols[breaks[sub]])
    }

    /// Position at visual `row`, nearest to row-relative display column `col`.
    fn row_col_to_pos(&self, doc: &Document, row: usize, col: usize, wrap: bool) -> usize {
        let (line, sub) = self.row_to_line(row, doc);
        let text = doc.line_text(line);
        let breaks = self.breaks_for(&text, wrap);
        let sub = sub.min(breaks.len() - 1);
        let cols = columns(&text, self.tab_width);
        let start = breaks[sub];
        let end = breaks.get(sub + 1).copied().unwrap_or(cols.len() - 1);
        let base = cols[start];
        let mut best = start;
        for i in start..=end {
            let c = cols[i] - base;
            if c >= col {
                best = if i > start && col - (cols[i - 1] - base) < c - col {
                    i - 1
                } else {
                    i
                };
                break;
            }
            best = i;
        }
        // On a wrapped row boundary the caret belongs to the next row.
        if sub + 1 < breaks.len() && best == end && end > start {
            best = end - 1;
        }
        doc.line_start(line) + best
    }

    // ------------------------------------------------------------ actions

    #[allow(clippy::too_many_lines)]
    pub fn exec(
        &mut self,
        doc: &mut Document,
        action: KeyAction,
        settings: &Settings,
        overtype: &mut bool,
    ) {
        let wrap = self.effective_wrap(doc, settings.word_wrap);
        self.ensure_layout(doc, wrap);
        match action {
            KeyAction::Move { m, extend } => {
                self.rect = None;
                let vertical = matches!(m, Move::Up | Move::Down | Move::PageUp | Move::PageDown);
                let page = self.visible_rows.saturating_sub(1).max(1);
                let total = self.total_rows(doc);
                let any_sel = self.sels.iter().any(|s| !s.is_empty());
                let mut new = Vec::with_capacity(self.sels.len());
                let mut pref = self.preferred_col;
                for (i, s) in self.sels.clone().into_iter().enumerate() {
                    let p = s.head;
                    let (line, _) = doc.pos_to_line_col(p);
                    let target = match m {
                        Move::Left if !extend && any_sel => s.start(),
                        Move::Right if !extend && any_sel => s.end(),
                        Move::Left => ops::step_left(doc, p),
                        Move::Right => ops::step_right(doc, p),
                        Move::WordLeft => ops::word_left(doc, p),
                        Move::WordRight => ops::word_right(doc, p),
                        Move::Home => {
                            let start = doc.line_start(line);
                            let text = doc.line_text(line);
                            let indent =
                                text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
                            if p == start + indent {
                                start
                            } else {
                                start + indent
                            }
                        }
                        Move::End => doc.line_end(line),
                        Move::DocStart => 0,
                        Move::DocEnd => doc.len_chars(),
                        Move::Up | Move::Down | Move::PageUp | Move::PageDown => {
                            let (row, col) = self.pos_to_row_col(doc, p, wrap);
                            let want = if i == self.primary_idx {
                                *pref.get_or_insert(col)
                            } else {
                                col
                            };
                            let n = if matches!(m, Move::PageUp | Move::PageDown) {
                                page
                            } else {
                                1
                            };
                            let up = matches!(m, Move::Up | Move::PageUp);
                            if up && row == 0 {
                                if matches!(m, Move::PageUp) { 0 } else { p }
                            } else if !up && row + 1 >= total {
                                if matches!(m, Move::PageDown) {
                                    doc.len_chars()
                                } else {
                                    p
                                }
                            } else {
                                let r = if up {
                                    row.saturating_sub(n)
                                } else {
                                    (row + n).min(total - 1)
                                };
                                self.row_col_to_pos(doc, r, want, wrap)
                            }
                        }
                    };
                    new.push(if extend {
                        Selection::new(s.anchor, target)
                    } else {
                        Selection::caret(target)
                    });
                }
                if matches!(m, Move::PageUp | Move::PageDown) {
                    let delta = page as f64;
                    self.top_row = if matches!(m, Move::PageUp) {
                        (self.top_row - delta).max(0.0)
                    } else {
                        self.top_row + delta
                    };
                }
                self.sels = new;
                if !vertical {
                    pref = None;
                }
                self.preferred_col = pref;
                normalize_selections(&mut self.sels);
                self.primary_idx = self.primary_idx.min(self.sels.len() - 1);
                self.scroll_to_caret = true;
                if !extend {
                    self.begin_select = None;
                }
            }
            KeyAction::RectMove(m) => {
                let (anchor, head) = self.rect.unwrap_or_else(|| {
                    let p = self.primary();
                    let a = doc.pos_to_line_col(p.anchor).0;
                    let h = doc.pos_to_line_col(p.head).0;
                    (
                        (a, self.display_col(doc, p.anchor)),
                        (h, self.display_col(doc, p.head)),
                    )
                });
                let (mut hl, mut hc) = head;
                let n = doc.line_count();
                match m {
                    Move::Up => hl = hl.saturating_sub(1),
                    Move::Down => hl = (hl + 1).min(n - 1),
                    Move::Left => hc = hc.saturating_sub(1),
                    Move::Right => hc += 1,
                    Move::PageUp => hl = hl.saturating_sub(self.visible_rows),
                    Move::PageDown => hl = (hl + self.visible_rows).min(n - 1),
                    Move::Home => hc = 0,
                    Move::End => hc = doc.line_len(hl),
                    _ => {}
                }
                self.set_rect(doc, anchor, (hl, hc));
            }
            KeyAction::Delete(unit) => {
                self.autocomplete = None;
                ops::delete(doc, self, unit);
            }
            KeyAction::Newline => {
                ops::newline(doc, self, settings.auto_indent);
            }
            KeyAction::Tab => ops::tab(doc, self, settings.tab_size, settings.use_spaces),
            KeyAction::BackTab => {
                ops::indent_lines(doc, self, settings.tab_size, settings.use_spaces, false)
            }
            KeyAction::ToggleOvertype => *overtype = !*overtype,
            KeyAction::Escape => {
                if self.autocomplete.take().is_none() && self.call_tip.take().is_none() {
                    let p = self.primary();
                    self.sels = vec![Selection::caret(p.head)];
                    self.primary_idx = 0;
                    self.rect = None;
                }
            }
            KeyAction::ScrollLines(n) => {
                self.top_row = (self.top_row + f64::from(n)).max(0.0);
            }
        }
    }

    /// Materialises a column selection into one selection per line.
    pub fn set_rect(&mut self, doc: &Document, anchor: (usize, usize), head: (usize, usize)) {
        self.rect = Some((anchor, head));
        let (l1, l2) = (anchor.0.min(head.0), anchor.0.max(head.0));
        let (c1, c2) = (anchor.1.min(head.1), anchor.1.max(head.1));
        let mut sels = Vec::new();
        for l in l1..=l2 {
            let text = doc.line_text(l);
            let (a, _) = self.col_to_char_in(&text, c1);
            let (b, _) = self.col_to_char_in(&text, c2);
            let s = doc.line_start(l);
            let (a, b) = if anchor.1 <= head.1 { (a, b) } else { (b, a) };
            sels.push(Selection::new(s + a, s + b));
        }
        self.primary_idx = if head.0 >= anchor.0 {
            sels.len() - 1
        } else {
            0
        };
        self.sels = sels;
        self.scroll_to_caret = true;
    }

    pub fn selected_texts(&self, doc: &Document) -> Vec<String> {
        let mut s = self.sels.clone();
        s.sort_by_key(|x| x.start());
        s.iter()
            .filter(|x| !x.is_empty())
            .map(|x| doc.slice(x.start(), x.end()))
            .collect()
    }
}

/// Everything the editor needs from the application for one frame.
pub struct EditorEnv<'a> {
    pub settings: &'a Settings,
    pub globals: &'a Globals,
    pub styles: &'a LangStyles,
    pub keywords: &'a [String],
    pub clip: &'a mut ClipState,
    pub overtype: &'a mut bool,
    pub font_size: f32,
}

#[derive(Default)]
pub struct EditorOutput {
    pub commands: Vec<Command>,
    pub clicked: bool,
    pub changed: bool,
    pub response: Option<Response>,
}

struct Metrics {
    cw: f32,
    lh: f32,
    gutter_ln: f32,
    gutter_bm: f32,
    gutter_ch: f32,
    gutter_fold: f32,
}

impl Metrics {
    fn gutter(&self) -> f32 {
        self.gutter_ln + self.gutter_bm + self.gutter_ch + self.gutter_fold
    }
}

/// The part of selection `ss..se` to highlight on a row showing characters
/// `row_s..row_e`, followed by a line break of `eol` characters (0 except on
/// a line's last row): the characters, and whether the line break is
/// selected. `None` if the selection doesn't touch the row.
fn selection_on_row(
    ss: usize,
    se: usize,
    row_s: usize,
    row_e: usize,
    eol: usize,
) -> Option<(usize, usize, bool)> {
    // A selection starting at the next line's first character is past this
    // row's line break, not on it.
    let past = if eol > 0 {
        ss >= row_e + eol
    } else {
        ss > row_e
    };
    if se < row_s || past {
        return None;
    }
    Some((
        ss.max(row_s).min(row_e),
        se.min(row_e),
        eol > 0 && se > row_e,
    ))
}

fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (f32::from(x) * (1.0 - t) + f32::from(y) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

pub fn translucent(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut Ui,
    id: egui::Id,
    vs: &mut ViewState,
    doc: &mut Document,
    env: &mut EditorEnv,
) -> EditorOutput {
    let mut out = EditorOutput::default();
    let settings = env.settings;
    let g = env.globals;
    vs.tab_width = settings.tab_size.max(1);
    doc.tab_width = vs.tab_width;
    vs.sync(doc);

    let size = env.font_size.clamp(5.0, 72.0);
    let font = FontId::monospace(size);
    let (cw, lh) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let lh = lh.ceil();
    let small_font = FontId::monospace((size * 0.8).max(6.0));

    let rect = ui.available_rect_before_wrap();
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let has_focus = response.has_focus();
    if has_focus && settings.caret_blink {
        // Caret is visible for the first 0.6 s of each second; wake up only
        // when it next turns on or off.
        let frac = ui.input(|i| i.time).fract();
        let wait = if frac < 0.6 { 0.6 - frac } else { 1.0 - frac } + 0.01;
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(wait));
    }
    if response.clicked() || response.drag_started() || response.secondary_clicked() {
        response.request_focus();
        out.clicked = true;
    }
    if has_focus {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
    }

    let digits = doc.line_count().to_string().len().max(3) as f32;
    let m = Metrics {
        cw,
        lh,
        gutter_ln: if settings.show_line_numbers {
            digits * cw + cw
        } else {
            0.0
        },
        gutter_bm: if settings.show_bookmark_margin {
            lh.max(14.0)
        } else {
            0.0
        },
        gutter_ch: if settings.show_change_history {
            4.0
        } else {
            0.0
        },
        gutter_fold: if settings.show_fold_margin && doc.fold_mode() != crate::langs::FoldMode::None
        {
            lh.max(12.0) * 0.9
        } else {
            0.0
        },
    };
    let sb = 12.0;
    let gutter_rect = Rect::from_min_size(rect.min, vec2(m.gutter(), rect.height()));
    let text_rect = Rect::from_min_max(
        pos2(rect.min.x + m.gutter(), rect.min.y),
        pos2(rect.max.x - sb, rect.max.y - sb),
    );
    vs.last_text_rect = text_rect;
    let text_left = text_rect.min.x + 4.0;
    vs.visible_rows = ((text_rect.height() / lh).floor() as usize).max(1);
    let wrap = vs.effective_wrap(doc, settings.word_wrap);
    vs.wrap_cols = (((text_rect.width() - 8.0) / cw).floor() as usize).max(10);
    if wrap {
        vs.scroll_x = 0.0;
    }

    // ------------------------------------------------------------ input
    if has_focus {
        handle_keys(ui, vs, doc, env, &mut out);
    }
    vs.ensure_layout(doc, wrap);
    handle_mouse(
        ui,
        &response,
        vs,
        doc,
        env,
        &m,
        text_rect,
        gutter_rect,
        text_left,
    );

    // Scroll to caret.
    vs.ensure_layout(doc, wrap);
    let total_rows = vs.total_rows(doc).max(1);
    if vs.scroll_to_caret || vs.center_caret {
        let head = vs.primary().head;
        let (row, col) = vs.pos_to_row_col(doc, head, wrap);
        let rowf = row as f64;
        let vis = vs.visible_rows as f64;
        if vs.center_caret {
            vs.top_row = (rowf - vis / 2.0).max(0.0);
        } else if rowf < vs.top_row {
            vs.top_row = rowf;
        } else if rowf > vs.top_row + vis - 1.0 {
            vs.top_row = rowf - vis + 1.0;
        }
        if !wrap {
            let x = col as f32 * cw;
            let w = text_rect.width() - 8.0;
            if x < vs.scroll_x {
                vs.scroll_x = (x - w / 3.0).max(0.0);
            } else if x > vs.scroll_x + w - cw {
                vs.scroll_x = x - w + w / 3.0;
            }
        }
        vs.scroll_to_caret = false;
        vs.center_caret = false;
    }
    let max_top = (total_rows as f64 - 1.0).max(0.0);
    vs.top_row = vs.top_row.clamp(0.0, max_top);

    // ------------------------------------------------------------ paint
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, g.bg);
    let first_row = vs.top_row.floor() as usize;
    let frac = (vs.top_row - first_row as f64) as f32;
    let rows_to_draw = vs.visible_rows + 2;
    let caret_lines: Vec<usize> = vs
        .sels
        .iter()
        .map(|s| doc.pos_to_line_col(s.head).0)
        .collect();
    let primary = vs.primary();
    let brace = if settings.brace_highlight && primary.is_empty() {
        let key = (doc.version, primary.head);
        match vs.brace_cache {
            Some((k, r)) if k == key => r,
            _ => {
                let r = ops::matching_brace_within(doc, primary.head, BRACE_HIGHLIGHT_LIMIT);
                vs.brace_cache = Some((key, r));
                r
            }
        }
    } else {
        None
    };
    // Smart highlighting: occurrences of the selected word.
    let smart: Option<String> =
        if settings.smart_highlight && !primary.is_empty() && vs.sels.len() == 1 {
            let t = doc.slice(primary.start(), primary.end());
            let ok = !t.contains(['\n', '\r'])
                && !t.trim().is_empty()
                && (!settings.smart_whole_word || {
                    let (ws, we) = ops::word_at(doc, primary.start());
                    ws == primary.start() && we == primary.end()
                });
            ok.then_some(t)
        } else {
            None
        };
    let text_painter = painter.with_clip_rect(text_rect);
    let decor = vs.compare_decor(doc).cloned();
    let cc = compare_colors(g.is_dark);
    let mut max_cols_seen = 0usize;
    let mut last_drawn_line = usize::MAX;
    for i in 0..rows_to_draw {
        let row = first_row + i;
        if row >= total_rows {
            break;
        }
        let (line, sub) = vs.row_to_line(row, doc);
        let y = text_rect.min.y + (i as f32 - frac) * lh;
        if y > text_rect.max.y {
            break;
        }
        if vs.is_spacer_row(row, doc) {
            // Compare spacer: a hatched row with no text.
            let r = Rect::from_min_size(pos2(text_rect.min.x, y), vec2(text_rect.width(), lh));
            text_painter.rect_filled(r, CornerRadius::ZERO, cc.spacer);
            let mut x = r.min.x - (y % 8.0);
            while x < r.max.x {
                text_painter.line_segment(
                    [pos2(x, y + lh), pos2(x + lh, y)],
                    Stroke::new(1.0, cc.spacer_line),
                );
                x += 8.0;
            }
            painter.rect_filled(
                Rect::from_min_size(pos2(gutter_rect.min.x, y), vec2(m.gutter(), lh)),
                CornerRadius::ZERO,
                g.line_number_bg,
            );
            continue;
        }
        let text = doc.line_text(line);
        let breaks = vs.breaks_for(&text, wrap);
        let sub = sub.min(breaks.len() - 1);
        let cols = columns(&text, vs.tab_width);
        let chars: Vec<char> = text.chars().collect();
        let rs = breaks[sub];
        let re = breaks.get(sub + 1).copied().unwrap_or(chars.len());
        let base_col = cols[rs];
        max_cols_seen = max_cols_seen.max(cols[chars.len()]);
        let x_of = |ci: usize| {
            text_left - vs.scroll_x + (cols[ci.min(cols.len() - 1)] - base_col) as f32 * cw
        };
        let line_start = doc.line_start(line);
        let row_rect = Rect::from_min_size(pos2(text_rect.min.x, y), vec2(text_rect.width(), lh));

        // Current line.
        if settings.highlight_current_line && caret_lines.contains(&line) && primary.is_empty() {
            text_painter.rect_filled(row_rect, CornerRadius::ZERO, g.current_line_bg);
        }
        // Compare: whole-line and changed-word backgrounds.
        if let Some(d) = &decor {
            let (bg, strong) = match d.kinds.get(line) {
                Some(LineKind::Removed) => (Some(cc.removed), cc.removed_strong),
                Some(LineKind::Added) => (Some(cc.added), cc.added_strong),
                Some(LineKind::Changed) => (Some(cc.changed), cc.changed_strong),
                Some(LineKind::Moved) => (Some(cc.moved), cc.moved_strong),
                _ => (None, cc.changed_strong),
            };
            if let Some(bg) = bg {
                text_painter.rect_filled(row_rect, CornerRadius::ZERO, bg);
                if let Some(ranges) = d.inline.get(&line) {
                    for &(a, b) in ranges {
                        if b <= rs || a >= re {
                            continue;
                        }
                        let r = Rect::from_min_max(
                            pos2(x_of(a.max(rs)), y),
                            pos2(x_of(b.min(re)).max(x_of(a.max(rs)) + 2.0), y + lh),
                        );
                        text_painter.rect_filled(r, CornerRadius::same(2), strong);
                    }
                }
            }
        }
        // Marks (find mark, style tokens, incremental highlight).
        let row_s = line_start + rs;
        let row_e = line_start + re;
        let mark_lo = doc.marks.partition_point(|mk| mk.end < row_s);
        for mk in doc.marks[mark_lo..]
            .iter()
            .take_while(|mk| mk.start <= row_e)
        {
            if mk.end < row_s || mk.start > row_e || mk.start == mk.end {
                continue;
            }
            let a = mk.start.max(row_s) - line_start;
            let b = mk.end.min(row_e) - line_start;
            let color = match mk.style {
                0 => g.find_mark,
                1..=5 => g.mark_styles[(mk.style - 1) as usize],
                _ => g.incremental,
            };
            let r = Rect::from_min_max(pos2(x_of(a), y), pos2(x_of(b).max(x_of(a) + 2.0), y + lh));
            if mk.style == 0 || mk.style > 5 {
                text_painter.rect_filled(r, CornerRadius::same(2), translucent(color, 70));
                text_painter.rect_stroke(
                    r,
                    CornerRadius::same(2),
                    Stroke::new(1.0, translucent(color, 200)),
                    StrokeKind::Inside,
                );
            } else {
                text_painter.rect_filled(r, CornerRadius::ZERO, translucent(color, 150));
            }
        }
        // Smart highlight.
        if let Some(word) = &smart {
            let hay: String = chars[rs..re].iter().collect();
            let (h2, w2) = if settings.smart_match_case {
                (hay.clone(), word.clone())
            } else {
                (hay.to_lowercase(), word.to_lowercase())
            };
            if h2.len() == hay.len() {
                let mut from = 0;
                while let Some(p) = h2[from..].find(&w2) {
                    let bs = from + p;
                    let be = bs + w2.len();
                    let cs = rs + hay[..bs].chars().count();
                    let ce = cs + hay[bs..be].chars().count();
                    let boundary_ok = !settings.smart_whole_word
                        || ((cs == 0 || !ops::is_word_char(chars[cs - 1]))
                            && (ce >= chars.len() || !ops::is_word_char(chars[ce])));
                    if boundary_ok && line_start + cs != primary.start() {
                        let r = Rect::from_min_max(pos2(x_of(cs), y), pos2(x_of(ce), y + lh));
                        text_painter.rect_filled(
                            r,
                            CornerRadius::same(2),
                            translucent(g.smart_highlight, 90),
                        );
                        text_painter.rect_stroke(
                            r,
                            CornerRadius::same(2),
                            Stroke::new(1.0, translucent(g.smart_highlight, 220)),
                            StrokeKind::Inside,
                        );
                    }
                    from = be.max(bs + 1);
                    if from >= h2.len() {
                        break;
                    }
                }
            }
        }
        // Selections.
        let line_end_pos = line_start + chars.len();
        let eol_len = {
            let next = if line + 1 < doc.line_count() {
                doc.line_start(line + 1)
            } else {
                doc.len_chars()
            };
            next - line_end_pos
        };
        let eol = if sub + 1 == breaks.len() { eol_len } else { 0 };
        for s in &vs.sels {
            if s.is_empty() {
                continue;
            }
            let Some((a, b, with_eol)) = selection_on_row(s.start(), s.end(), row_s, row_e, eol)
            else {
                continue;
            };
            let (a, b) = (a - line_start, b - line_start);
            let mut x1 = x_of(b);
            if with_eol {
                x1 += cw;
            }
            let r = Rect::from_min_max(pos2(x_of(a), y), pos2(x1.max(x_of(a)), y + lh));
            text_painter.rect_filled(r, CornerRadius::ZERO, g.selection_bg);
        }
        // Indent guides.
        if settings.show_indent_guides && sub == 0 {
            let indent_chars = chars
                .iter()
                .take_while(|c| **c == ' ' || **c == '\t')
                .count();
            let indent_cols = if indent_chars == chars.len() {
                // Blank line: use the indentation of the next non-blank line.
                let mut l2 = line + 1;
                let mut ind = 0;
                while l2 < doc.line_count() && l2 < line + 50 {
                    let t2 = doc.line_text(l2);
                    if !t2.trim().is_empty() {
                        let c2 = columns(&t2, vs.tab_width);
                        ind = c2[t2.chars().take_while(|c| *c == ' ' || *c == '\t').count()];
                        break;
                    }
                    l2 += 1;
                }
                ind
            } else {
                cols[indent_chars]
            };
            let mut c = vs.tab_width;
            while c < indent_cols {
                let x = text_left - vs.scroll_x + c as f32 * cw + 0.5;
                let mut yy = y;
                while yy < y + lh {
                    text_painter.line_segment(
                        [pos2(x, yy), pos2(x, (yy + 1.0).min(y + lh))],
                        Stroke::new(1.0, g.indent_guide),
                    );
                    yy += 2.0;
                }
                c += vs.tab_width;
            }
        }
        // Text with syntax colours.
        let spans = if doc.len_chars() > 0 {
            doc.line_spans(line, &text)
        } else {
            Vec::new()
        };
        let byte_to_ci = {
            let mut v = vec![0usize; text.len() + 1];
            let mut ci = 0;
            for (bi, ch) in text.char_indices() {
                for k in 0..ch.len_utf8() {
                    v[bi + k] = ci;
                }
                ci += 1;
            }
            v[text.len()] = ci;
            v
        };
        let mut kind_at = vec![TokenKind::Default; chars.len()];
        for (s, e, k) in &spans {
            let (cs, ce) = (
                byte_to_ci[(*s).min(text.len())],
                byte_to_ci[(*e).min(text.len())],
            );
            for slot in kind_at.iter_mut().take(ce).skip(cs) {
                *slot = *k;
            }
        }
        let mut ci = rs;
        let vis_start_col = (vs.scroll_x / cw).floor() as usize + base_col;
        let vis_end_col = vis_start_col + (text_rect.width() / cw).ceil() as usize + 2;
        while ci < re {
            let k = kind_at[ci];
            let mut cj = ci;
            let mut seg = String::new();
            while cj < re && kind_at[cj] == k && cj - ci < 256 {
                let ch = chars[cj];
                if ch == '\t' {
                    seg.extend(std::iter::repeat_n(' ', cols[cj + 1] - cols[cj]));
                } else if (ch as u32) < 0x20 || ch == '\u{7f}' {
                    break;
                } else {
                    seg.push(ch);
                }
                cj += 1;
            }
            if cj == ci {
                // Control character: draw it as an inverted box.
                let r = Rect::from_min_size(pos2(x_of(ci), y + 1.0), vec2(cw, lh - 2.0));
                text_painter.rect_filled(r, CornerRadius::same(1), g.fg);
                text_painter.text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    "?",
                    small_font.clone(),
                    g.bg,
                );
                ci += 1;
                continue;
            }
            if cols[cj] >= vis_start_col && cols[ci] <= vis_end_col {
                let style = env.styles.get(k);
                if let Some(bg) = style.bg {
                    let r = Rect::from_min_max(pos2(x_of(ci), y), pos2(x_of(cj), y + lh));
                    text_painter.rect_filled(r, CornerRadius::ZERO, bg);
                }
                let mut fmt = egui::TextFormat {
                    font_id: font.clone(),
                    color: style.fg,
                    italics: style.italic,
                    ..Default::default()
                };
                if style.underline {
                    fmt.underline = Stroke::new(1.0, style.fg);
                }
                let mut job = egui::text::LayoutJob::single_section(seg, fmt);
                job.wrap.max_width = f32::INFINITY;
                let galley = text_painter.layout_job(job);
                let p = pos2(x_of(ci), y);
                if style.bold {
                    text_painter.galley(p + vec2(0.6, 0.0), galley.clone(), style.fg);
                }
                text_painter.galley(p, galley, style.fg);
            }
            ci = cj;
        }
        // Whitespace symbols.
        if settings.show_whitespace {
            for c in rs..re {
                let ch = chars[c];
                if ch == ' ' {
                    let cx = x_of(c) + cw / 2.0;
                    text_painter.circle_filled(
                        pos2(cx, y + lh / 2.0),
                        (cw / 8.0).max(1.0),
                        g.whitespace,
                    );
                } else if ch == '\t' {
                    let x0 = x_of(c) + 1.0;
                    let x1 = x_of(c + 1) - 2.0;
                    let cy = y + lh / 2.0;
                    let st = Stroke::new(1.0, g.whitespace);
                    text_painter.line_segment([pos2(x0, cy), pos2(x1, cy)], st);
                    text_painter.line_segment([pos2(x1 - 3.0, cy - 3.0), pos2(x1, cy)], st);
                    text_painter.line_segment([pos2(x1 - 3.0, cy + 3.0), pos2(x1, cy)], st);
                }
            }
        }
        // EOL markers.
        if settings.show_eol && sub + 1 == breaks.len() && eol_len > 0 {
            let label = match (doc.char_at(line_end_pos), doc.char_at(line_end_pos + 1)) {
                (Some('\r'), Some('\n')) => "CRLF",
                (Some('\r'), _) => "CR",
                _ => "LF",
            };
            let x = x_of(chars.len()) + 2.0;
            let galley = text_painter.layout_no_wrap(label.to_string(), small_font.clone(), g.bg);
            let r = Rect::from_min_size(pos2(x, y + 1.0), vec2(galley.size().x + 4.0, lh - 2.0));
            text_painter.rect_filled(r, CornerRadius::same(2), g.whitespace);
            text_painter.galley(
                pos2(x + 2.0, y + (lh - galley.size().y) / 2.0),
                galley,
                g.bg,
            );
        }
        if wrap && settings.show_wrap_symbol && sub + 1 < breaks.len() {
            // Hooked return arrow, drawn as lines so it needs no glyph.
            let x = x_of(re) + 3.0;
            let (w, my) = (m.cw * 0.8, y + lh * 0.5);
            let stroke = Stroke::new(1.0, g.whitespace);
            text_painter.line_segment([pos2(x + w, y + lh * 0.25), pos2(x + w, my)], stroke);
            text_painter.line_segment([pos2(x + w, my), pos2(x, my)], stroke);
            text_painter.line_segment([pos2(x, my), pos2(x + w * 0.35, my - w * 0.35)], stroke);
            text_painter.line_segment([pos2(x, my), pos2(x + w * 0.35, my + w * 0.35)], stroke);
        }
        // Brace highlight.
        if let Some((a, b)) = brace {
            for (p, bad) in [(a, b == usize::MAX), (b, false)] {
                if p == usize::MAX || p < row_s || p >= row_e {
                    continue;
                }
                let c = p - line_start;
                let color = if bad { g.bad_brace_fg } else { g.brace_fg };
                let r = Rect::from_min_size(pos2(x_of(c), y), vec2(cw, lh));
                text_painter.rect_filled(r, CornerRadius::ZERO, g.bg);
                let galley = text_painter.layout_no_wrap(chars[c].to_string(), font.clone(), color);
                text_painter.galley(r.min, galley.clone(), color);
                text_painter.galley(r.min + vec2(0.6, 0.0), galley, color);
            }
        }
        // Carets.
        let blink_on = !settings.caret_blink || ui.input(|i| i.time).fract() < 0.6;
        if has_focus && blink_on {
            for s in &vs.sels {
                let h = s.head;
                if h < row_s || h > row_e || (h == row_e && sub + 1 < breaks.len()) {
                    continue;
                }
                let c = h - line_start;
                let x = x_of(c);
                if *env.overtype {
                    text_painter.rect_filled(
                        Rect::from_min_size(pos2(x, y + lh - 2.0), vec2(cw, 2.0)),
                        CornerRadius::ZERO,
                        g.caret,
                    );
                } else {
                    text_painter.rect_filled(
                        Rect::from_min_size(
                            pos2(x - settings.caret_width / 2.0, y),
                            vec2(settings.caret_width, lh),
                        ),
                        CornerRadius::ZERO,
                        g.caret,
                    );
                }
            }
        }
        // ---- gutter for this row
        let gy = y;
        let mut gx = gutter_rect.min.x;
        if m.gutter_ln > 0.0 {
            let r = Rect::from_min_size(pos2(gx, gy), vec2(m.gutter_ln, lh));
            painter.rect_filled(r, CornerRadius::ZERO, g.line_number_bg);
            if sub == 0 {
                let current = caret_lines.contains(&line);
                let color = if current {
                    blend(g.line_number_fg, g.fg, 0.6)
                } else {
                    g.line_number_fg
                };
                painter.text(
                    pos2(gx + m.gutter_ln - cw / 2.0, gy + lh / 2.0),
                    egui::Align2::RIGHT_CENTER,
                    (line + 1).to_string(),
                    font.clone(),
                    color,
                );
            }
            gx += m.gutter_ln;
        }
        if m.gutter_bm > 0.0 {
            let r = Rect::from_min_size(pos2(gx, gy), vec2(m.gutter_bm, lh));
            painter.rect_filled(r, CornerRadius::ZERO, g.bookmark_margin_bg);
            if sub == 0 && doc.meta.get(line).is_some_and(|mm| mm.bookmark) {
                let c = r.center();
                let rad = (lh * 0.35).min(m.gutter_bm * 0.4);
                painter.circle_filled(c, rad, Color32::from_rgb(0x3a, 0x6e, 0xd8));
                painter.circle_stroke(
                    c,
                    rad,
                    Stroke::new(1.0, Color32::from_rgb(0x1c, 0x3c, 0x90)),
                );
            }
            gx += m.gutter_bm;
        }
        if m.gutter_ch > 0.0 {
            let r = Rect::from_min_size(pos2(gx, gy), vec2(m.gutter_ch, lh));
            painter.rect_filled(r, CornerRadius::ZERO, g.bookmark_margin_bg);
            match doc.meta.get(line).map_or(0, |mm| mm.change) {
                1 => {
                    painter.rect_filled(
                        r.shrink2(vec2(0.5, 0.0)),
                        CornerRadius::ZERO,
                        g.change_modified,
                    );
                }
                2 => {
                    painter.rect_filled(
                        r.shrink2(vec2(0.5, 0.0)),
                        CornerRadius::ZERO,
                        g.change_saved,
                    );
                }
                _ => {}
            }
            gx += m.gutter_ch;
        }
        if m.gutter_fold > 0.0 {
            let r = Rect::from_min_size(pos2(gx, gy), vec2(m.gutter_fold, lh));
            painter.rect_filled(r, CornerRadius::ZERO, g.fold_margin_bg);
            if sub == 0 && line != last_drawn_line && doc.is_fold_header(line) {
                let folded = doc.meta.get(line).is_some_and(|mm| mm.folded);
                let sz = (m.gutter_fold * 0.6).min(lh * 0.6).floor();
                let b = Rect::from_center_size(r.center(), vec2(sz, sz));
                painter.rect_filled(b, CornerRadius::ZERO, g.fold_bg);
                painter.rect_stroke(
                    b,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, g.fold_fg),
                    StrokeKind::Inside,
                );
                let c = b.center();
                let h = sz * 0.3;
                painter.line_segment(
                    [pos2(c.x - h, c.y), pos2(c.x + h, c.y)],
                    Stroke::new(1.0, g.fold_fg),
                );
                if folded {
                    painter.line_segment(
                        [pos2(c.x, c.y - h), pos2(c.x, c.y + h)],
                        Stroke::new(1.0, g.fold_fg),
                    );
                    text_painter.line_segment(
                        [
                            pos2(text_rect.min.x, y + lh - 0.5),
                            pos2(text_rect.max.x, y + lh - 0.5),
                        ],
                        Stroke::new(1.0, g.fold_fg),
                    );
                }
            }
        }
        last_drawn_line = line;
    }
    // Remaining gutter area below the last line.
    let drawn_rows = total_rows.saturating_sub(first_row).min(rows_to_draw);
    let y_end = text_rect.min.y + (drawn_rows as f32 - frac) * lh;
    if y_end < rect.max.y && m.gutter() > 0.0 {
        let mut gx = gutter_rect.min.x;
        for (w, c) in [
            (m.gutter_ln, g.line_number_bg),
            (m.gutter_bm, g.bookmark_margin_bg),
            (m.gutter_ch, g.bookmark_margin_bg),
            (m.gutter_fold, g.fold_margin_bg),
        ] {
            if w > 0.0 {
                painter.rect_filled(
                    Rect::from_min_max(pos2(gx, y_end.max(rect.min.y)), pos2(gx + w, rect.max.y)),
                    CornerRadius::ZERO,
                    c,
                );
                gx += w;
            }
        }
    }
    // Edge column.
    if settings.edge_column > 0 && !wrap {
        let x = text_left - vs.scroll_x + settings.edge_column as f32 * cw;
        text_painter.line_segment(
            [pos2(x, text_rect.min.y), pos2(x, text_rect.max.y)],
            Stroke::new(1.0, g.edge),
        );
    }
    vs.max_cols = vs.max_cols.max(max_cols_seen);
    if vs.max_cols > max_cols_seen * 4 + 200 {
        vs.max_cols = max_cols_seen;
    }

    // ------------------------------------------------------------ scrollbars
    let sb_bg = if g.is_dark {
        Color32::from_gray(40)
    } else {
        Color32::from_gray(235)
    };
    let sb_fg = if g.is_dark {
        Color32::from_gray(100)
    } else {
        Color32::from_gray(180)
    };
    let vtrack = Rect::from_min_max(
        pos2(rect.max.x - sb, rect.min.y),
        pos2(rect.max.x, rect.max.y - sb),
    );
    painter.rect_filled(vtrack, CornerRadius::ZERO, sb_bg);
    let vis = vs.visible_rows as f32;
    let total = (total_rows as f32 + vis - 1.0).max(1.0);
    let thumb_h = (vtrack.height() * vis / total).clamp(20.0, vtrack.height());
    let thumb_y = vtrack.min.y
        + (vtrack.height() - thumb_h)
            * (vs.top_row as f32 / (total - vis).max(1.0)).clamp(0.0, 1.0);
    let vthumb = Rect::from_min_size(pos2(vtrack.min.x + 2.0, thumb_y), vec2(sb - 4.0, thumb_h));
    painter.rect_filled(vthumb, CornerRadius::same(4), sb_fg);
    // Bookmark / find-mark ticks on the scrollbar.
    if doc.line_count() > 0 {
        let n = doc.line_count() as f32;
        for l in doc.bookmarked_lines().iter().take(2000) {
            let yy = vtrack.min.y + vtrack.height() * (*l as f32 / n);
            painter.line_segment(
                [pos2(vtrack.min.x, yy), pos2(vtrack.max.x, yy)],
                Stroke::new(2.0, Color32::from_rgb(0x3a, 0x6e, 0xd8)),
            );
        }
    }
    // Compare differences on the scrollbar.
    if let Some(d) = &decor {
        let total = total_rows.max(1) as f32;
        for &(a, b, k) in d.runs.iter().take(20_000) {
            let color = match k {
                LineKind::Removed => cc.removed_strong,
                LineKind::Added => cc.added_strong,
                LineKind::Moved => cc.moved_strong,
                _ => cc.changed_strong,
            };
            let y0 = vtrack.min.y + vtrack.height() * (vs.line_first_row(a) as f32 / total);
            let y1 = vtrack.min.y
                + vtrack.height() * (vs.line_first_row(b.max(a + 1) - 1) as f32 + 1.0) / total;
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(vtrack.min.x + 1.0, y0),
                    pos2(vtrack.max.x - 1.0, y1.max(y0 + 2.0)),
                ),
                CornerRadius::ZERO,
                color,
            );
        }
    }
    let htrack = Rect::from_min_max(
        pos2(text_rect.min.x, rect.max.y - sb),
        pos2(rect.max.x - sb, rect.max.y),
    );
    painter.rect_filled(htrack, CornerRadius::ZERO, sb_bg);
    painter.rect_filled(
        Rect::from_min_max(
            pos2(rect.min.x, rect.max.y - sb),
            pos2(text_rect.min.x, rect.max.y),
        ),
        CornerRadius::ZERO,
        sb_bg,
    );
    painter.rect_filled(
        Rect::from_min_max(pos2(rect.max.x - sb, rect.max.y - sb), rect.max),
        CornerRadius::ZERO,
        sb_bg,
    );
    let content_w = (vs.max_cols as f32 + 4.0) * cw;
    let view_w = text_rect.width();
    let hthumb = if !wrap && content_w > view_w {
        let w = (htrack.width() * view_w / content_w).clamp(20.0, htrack.width());
        let x = htrack.min.x
            + (htrack.width() - w) * (vs.scroll_x / (content_w - view_w)).clamp(0.0, 1.0);
        let r = Rect::from_min_size(pos2(x, htrack.min.y + 2.0), vec2(w, sb - 4.0));
        painter.rect_filled(r, CornerRadius::same(4), sb_fg);
        Some(r)
    } else {
        vs.scroll_x = if wrap {
            0.0
        } else {
            vs.scroll_x.min((content_w - view_w).max(0.0))
        };
        None
    };
    // Scrollbar interaction.
    let pointer = ui.input(|i| i.pointer.clone());
    // Presses over a window in front of the editor (such as Find) are not
    // for the editor.
    let on_top = response.contains_pointer();
    if let Some(pp) = pointer.interact_pos() {
        if pointer.primary_pressed() && on_top && vtrack.contains(pp) {
            if vthumb.contains(pp) {
                vs.drag = Some(Drag::VScroll {
                    grab: pp.y - vthumb.min.y,
                });
            } else {
                let page = f64::from(vis);
                vs.top_row += if pp.y < vthumb.min.y { -page } else { page };
            }
        }
        if pointer.primary_pressed()
            && on_top
            && htrack.contains(pp)
            && let Some(h) = hthumb
        {
            if h.contains(pp) {
                vs.drag = Some(Drag::HScroll {
                    grab: pp.x - h.min.x,
                });
            } else {
                vs.scroll_x += if pp.x < h.min.x { -view_w } else { view_w };
                vs.scroll_x = vs.scroll_x.max(0.0);
            }
        }
        match vs.drag {
            Some(Drag::VScroll { grab }) if pointer.primary_down() => {
                let t = ((pp.y - grab - vtrack.min.y) / (vtrack.height() - thumb_h).max(1.0))
                    .clamp(0.0, 1.0);
                vs.top_row = f64::from(t * (total - vis).max(0.0)).floor();
            }
            Some(Drag::HScroll { grab }) if pointer.primary_down() => {
                if let Some(h) = hthumb {
                    let t = ((pp.x - grab - htrack.min.x) / (htrack.width() - h.width()).max(1.0))
                        .clamp(0.0, 1.0);
                    vs.scroll_x = t * (content_w - view_w);
                }
            }
            _ => {}
        }
    }

    // Auto-completion popup.
    if let Some(ac) = &vs.autocomplete
        && has_focus
    {
        let (row, col) = vs.pos_to_row_col(doc, ac.start, wrap);
        let y = text_rect.min.y + (row as f32 - vs.top_row as f32 + 1.0) * lh;
        let x = text_left - vs.scroll_x + col as f32 * cw;
        let mut chosen = None;
        egui::Area::new(id.with("ac"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(x, y))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(160.0);
                    let start = ac.selected.saturating_sub(9);
                    for (i, item) in ac.items.iter().enumerate().skip(start).take(10) {
                        let r = ui.selectable_label(
                            i == ac.selected,
                            egui::RichText::new(item).font(font.clone()),
                        );
                        if r.clicked() {
                            chosen = Some(i);
                        }
                    }
                });
            });
        if let Some(i) = chosen {
            accept_autocomplete(vs, doc, i);
        }
    }

    // Call tip: shown above the line while the caret is inside the call.
    if let Some((open, tip)) = &vs.call_tip {
        let head = vs.primary().head;
        let inside = head > *open
            && head <= doc.len_chars()
            && crate::calltip::active_param(&doc.slice(open + 1, head)).is_some();
        if !inside || !has_focus {
            vs.call_tip = None;
        } else {
            let param = crate::calltip::active_param(&doc.slice(open + 1, head)).unwrap_or(0);
            let (row, col) = vs.pos_to_row_col(doc, *open, wrap);
            let y = text_rect.min.y + (row as f32 - vs.top_row as f32) * lh;
            let x = text_left - vs.scroll_x + col as f32 * cw;
            let mut job = egui::text::LayoutJob::default();
            let normal = egui::TextFormat::simple(font.clone(), ui.visuals().text_color());
            let strong = egui::TextFormat {
                color: ui.visuals().strong_text_color(),
                underline: Stroke::new(1.0, ui.visuals().strong_text_color()),
                ..normal.clone()
            };
            // Underline the parameter the caret is in.
            let (pre, params) = tip.split_at(tip.find('(').unwrap_or(tip.len()));
            job.append(pre, 0.0, normal.clone());
            let mut depth = 0;
            let mut index = 0;
            let mut seg = String::new();
            for c in params.chars() {
                let at_top = depth == 1;
                match c {
                    '(' | '[' | '{' | '<' => depth += 1,
                    ')' | ']' | '}' | '>' => depth -= 1,
                    _ => {}
                }
                seg.push(c);
                if (c == ',' && at_top) || (c == ')' && depth == 0) {
                    let fmt = if index == param {
                        strong.clone()
                    } else {
                        normal.clone()
                    };
                    job.append(&seg, 0.0, fmt);
                    seg.clear();
                    index += 1;
                }
            }
            job.append(&seg, 0.0, normal);
            egui::Area::new(id.with("calltip"))
                .order(egui::Order::Foreground)
                .pivot(egui::Align2::LEFT_BOTTOM)
                .fixed_pos(pos2(x, y - 2.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(job);
                    });
                });
        }
    }

    // IME position.
    if has_focus {
        let (row, col) = vs.pos_to_row_col(doc, vs.primary().head, wrap);
        let y = text_rect.min.y + (row as f32 - vs.top_row as f32) * lh;
        let x = text_left - vs.scroll_x + col as f32 * cw;
        let cursor_rect = Rect::from_min_size(pos2(x, y), vec2(2.0, lh));
        ui.ctx().output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Normal,
                rect: text_rect,
                cursor_rect,
                should_interrupt_composition: false,
            });
        });
    }

    // Context menu.
    response.context_menu(|ui| {
        let items: &[(&str, Command)] = &[
            ("Cut", Command::Cut),
            ("Copy", Command::Copy),
            ("Paste", Command::Paste),
            ("Delete", Command::DeleteSelection),
            ("Select All", Command::SelectAll),
        ];
        for (label, cmd) in items {
            if ui.button(*label).clicked() {
                out.commands.push(cmd.clone());
                ui.close();
            }
        }
        ui.separator();
        ui.menu_button("Style all occurrences of token", |ui| {
            for i in 1..=5u8 {
                if ui
                    .button(format!("Using {}{} style", i, ordinal(i)))
                    .clicked()
                {
                    out.commands.push(Command::StyleAll(i));
                    ui.close();
                }
            }
        });
        ui.menu_button("Clear style", |ui| {
            for i in 1..=5u8 {
                if ui
                    .button(format!("Clear {}{} style", i, ordinal(i)))
                    .clicked()
                {
                    out.commands.push(Command::ClearStyle(i));
                    ui.close();
                }
            }
            if ui.button("Clear all styles").clicked() {
                out.commands.push(Command::ClearAllStyles);
                ui.close();
            }
        });
        ui.separator();
        for (label, cmd) in [
            (
                "UPPERCASE",
                Command::ConvertCase(crate::textops::CaseMode::Upper),
            ),
            (
                "lowercase",
                Command::ConvertCase(crate::textops::CaseMode::Lower),
            ),
        ] {
            if ui.button(label).clicked() {
                out.commands.push(cmd);
                ui.close();
            }
        }
        ui.separator();
        for (label, cmd) in [
            ("Toggle Single Line Comment", Command::ToggleComment),
            ("Block Comment", Command::BlockComment),
            ("Block Uncomment", Command::BlockUncomment),
        ] {
            if ui.button(label).clicked() {
                out.commands.push(cmd);
                ui.close();
            }
        }
        ui.separator();
        for (label, cmd) in [
            ("Hide/Show folding: fold current", Command::FoldCurrent),
            ("Unfold current", Command::UnfoldCurrent),
            ("Open as File", Command::OpenSelectedFile),
            ("Search on the Web", Command::SearchOnInternet),
            ("Redact", Command::Redact),
        ] {
            if ui.button(label).clicked() {
                out.commands.push(cmd);
                ui.close();
            }
        }
    });
    out.response = Some(response);
    out
}

struct CompareColors {
    removed: Color32,
    added: Color32,
    changed: Color32,
    removed_strong: Color32,
    added_strong: Color32,
    changed_strong: Color32,
    moved: Color32,
    moved_strong: Color32,
    spacer: Color32,
    spacer_line: Color32,
}

/// Marker colour for a compared line, as on the scrollbar (None if unchanged).
pub fn compare_marker_color(kind: LineKind, dark: bool) -> Option<Color32> {
    let cc = compare_colors(dark);
    match kind {
        LineKind::Same => None,
        LineKind::Removed => Some(cc.removed_strong),
        LineKind::Added => Some(cc.added_strong),
        LineKind::Changed => Some(cc.changed_strong),
        LineKind::Moved => Some(cc.moved_strong),
    }
}

fn compare_colors(dark: bool) -> CompareColors {
    let c = Color32::from_rgb;
    if dark {
        CompareColors {
            removed: c(0x4a, 0x22, 0x26),
            added: c(0x1f, 0x42, 0x26),
            changed: c(0x45, 0x3d, 0x1c),
            removed_strong: c(0x8c, 0x33, 0x3a),
            added_strong: c(0x2f, 0x7a, 0x3d),
            changed_strong: c(0x86, 0x6f, 0x22),
            moved: c(0x1e, 0x33, 0x52),
            moved_strong: c(0x35, 0x5c, 0x9a),
            spacer: c(0x26, 0x26, 0x28),
            spacer_line: c(0x34, 0x34, 0x38),
        }
    } else {
        CompareColors {
            removed: c(0xff, 0xdc, 0xdc),
            added: c(0xd8, 0xf5, 0xd8),
            changed: c(0xff, 0xf4, 0xbf),
            removed_strong: c(0xff, 0x9c, 0x9c),
            added_strong: c(0x8f, 0xdc, 0x8f),
            changed_strong: c(0xff, 0xcc, 0x55),
            moved: c(0xdc, 0xe8, 0xff),
            moved_strong: c(0x8f, 0xb4, 0xf0),
            spacer: c(0xf2, 0xf2, 0xf2),
            spacer_line: c(0xdd, 0xdd, 0xdd),
        }
    }
}

fn ordinal(i: u8) -> &'static str {
    match i {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    }
}

fn accept_autocomplete(vs: &mut ViewState, doc: &mut Document, idx: usize) {
    let Some(ac) = vs.autocomplete.take() else {
        return;
    };
    let Some(word) = ac.items.get(idx) else {
        return;
    };
    let head = vs.primary().head;
    let (_, we) = ops::word_at(doc, head);
    let end = we.max(head);
    let before = vs.sels.clone();
    vs.sels = doc.edit(
        vec![(ac.start, end, word.clone())],
        &before,
        EditKind::Other,
        |f| vec![Selection::caret(f[0].1)],
    );
    vs.after_edit(doc);
}

/// Shows the parameters of the function whose call was just opened with `(`,
/// if it's defined in this document.
fn open_call_tip(vs: &mut ViewState, doc: &Document) {
    let head = vs.primary().head;
    // With auto-close the caret sits between `(` and `)`.
    let Some(open) = (0..head)
        .rev()
        .take(2)
        .find(|&p| doc.char_at(p) == Some('('))
    else {
        return;
    };
    let mut start = open;
    while start > 0 && doc.char_at(start - 1).is_some_and(ops::is_word_char) {
        start -= 1;
    }
    let name = doc.slice(start, open);
    if name.is_empty() {
        return;
    }
    let text = doc.text();
    if let Some(tip) = crate::calltip::find_signature(&text, &name) {
        vs.call_tip = Some((open, tip));
    }
}

/// Builds the auto-completion list for the word before the caret.
pub fn open_autocomplete(
    vs: &mut ViewState,
    doc: &Document,
    keywords: &[String],
    use_doc_words: bool,
    use_keywords: bool,
    min_chars: usize,
    force: bool,
) {
    let head = vs.primary().head;
    let mut start = head;
    while start > 0 && doc.char_at(start - 1).is_some_and(ops::is_word_char) {
        start -= 1;
    }
    let prefix = doc.slice(start, head);
    if !force && prefix.chars().count() < min_chars {
        vs.autocomplete = None;
        return;
    }
    let lower = prefix.to_lowercase();
    // After `.`, `->` or `::` only members make sense: names that appear
    // after one of those elsewhere in the file. Keywords don't.
    let before = doc.slice(start.saturating_sub(2), start);
    let member = before.ends_with('.') || before.ends_with("->") || before.ends_with("::");
    let mut items: Vec<String> = Vec::new();
    if use_keywords && !member {
        items.extend(
            keywords
                .iter()
                .filter(|k| k.to_lowercase().starts_with(&lower) && **k != prefix)
                .cloned(),
        );
    }
    // Names the file declares (`let x`, `fn x`, `class X`, ...) are listed
    // first, then the other words.
    let mut declared: Vec<String> = Vec::new();
    if use_doc_words {
        // Only the text around the caret: scanning a whole multi-megabyte
        // file on every keystroke would cost more than a frame.
        const WINDOW: usize = 256 * 1024;
        const DECL: &[&str] = &[
            "let",
            "var",
            "const",
            "fn",
            "def",
            "function",
            "func",
            "fun",
            "class",
            "struct",
            "enum",
            "interface",
            "type",
            "trait",
            "impl",
            "sub",
            "val",
            "mut",
            "static",
            "int",
            "long",
            "float",
            "double",
            "bool",
            "char",
            "string",
            "String",
            "auto",
            "void",
        ];
        let hay = doc.slice(
            head.saturating_sub(WINDOW),
            (head + WINDOW).min(doc.len_chars()),
        );
        let has_prefix = |w: &str| {
            if prefix.is_ascii() {
                w.as_bytes()
                    .get(..prefix.len())
                    .is_some_and(|h| h.eq_ignore_ascii_case(prefix.as_bytes()))
            } else {
                w.to_lowercase().starts_with(&lower)
            }
        };
        let mut seen = std::collections::HashSet::new();
        let mut prev_word = "";
        let mut word_start = None;
        let bytes = hay.as_bytes();
        for (i, c) in hay.char_indices().chain(std::iter::once((hay.len(), ' '))) {
            if ops::is_word_char(c) {
                word_start.get_or_insert(i);
                continue;
            }
            let Some(ws) = word_start.take() else {
                continue;
            };
            let w = &hay[ws..i];
            let after_member = (ws >= 1 && bytes[ws - 1] == b'.')
                || (ws >= 2 && (&bytes[ws - 2..ws] == b"->" || &bytes[ws - 2..ws] == b"::"));
            let fits = w.len() > prefix.len() && has_prefix(w) && w != prefix;
            if fits && (!member || after_member) && seen.insert(w) {
                if DECL.contains(&prev_word) {
                    declared.push(w.to_string());
                } else {
                    items.push(w.to_string());
                }
            } else if fits && DECL.contains(&prev_word) && !declared.iter().any(|d| d == w) {
                // Seen before as a plain word; it's a declaration after all.
                if let Some(p) = items.iter().position(|x| x == w) {
                    items.remove(p);
                    declared.push(w.to_string());
                }
            }
            prev_word = w;
            if seen.len() > 2000 {
                break;
            }
        }
    }
    declared.sort_by_key(|s| s.to_lowercase());
    items.sort_by_key(|s| s.to_lowercase());
    items.dedup();
    items.retain(|w| !declared.contains(w));
    declared.extend(items);
    let items = declared;
    if items.is_empty() {
        vs.autocomplete = None;
    } else {
        vs.autocomplete = Some(AutoComplete {
            start,
            items,
            selected: 0,
        });
    }
}

#[allow(clippy::too_many_lines)]
fn handle_keys(
    ui: &Ui,
    vs: &mut ViewState,
    doc: &mut Document,
    env: &mut EditorEnv,
    out: &mut EditorOutput,
) {
    let events = ui.input(|i| i.events.clone());
    let mac = cfg!(target_os = "macos");
    let settings = env.settings;
    for ev in events {
        match ev {
            Event::Text(t) | Event::Ime(egui::ImeEvent::Commit(t)) => {
                if t.is_empty() || t.chars().all(char::is_control) || doc.read_only {
                    continue;
                }
                ops::type_text(
                    doc,
                    vs,
                    &t,
                    *env.overtype,
                    settings.auto_close_brackets,
                    settings.auto_close_quotes,
                );
                out.changed = true;
                if settings.auto_complete && t.chars().all(ops::is_word_char) {
                    open_autocomplete(
                        vs,
                        doc,
                        env.keywords,
                        true,
                        true,
                        settings.auto_complete_min_chars,
                        false,
                    );
                } else if let Some(ac) = &vs.autocomplete
                    && !t.chars().all(ops::is_word_char)
                {
                    let _ = ac;
                    vs.autocomplete = None;
                }
                if settings.auto_complete && t.contains('(') {
                    open_call_tip(vs, doc);
                }
            }
            Event::Copy => {
                copy(vs, doc, env, false);
            }
            Event::Cut => {
                copy(vs, doc, env, true);
                out.changed = true;
            }
            Event::Paste(s) => {
                paste(vs, doc, env, &s);
                out.changed = true;
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                // Auto-completion navigation.
                if let Some(ac) = &mut vs.autocomplete {
                    match key {
                        Key::ArrowDown => {
                            ac.selected = (ac.selected + 1).min(ac.items.len() - 1);
                            continue;
                        }
                        Key::ArrowUp => {
                            ac.selected = ac.selected.saturating_sub(1);
                            continue;
                        }
                        Key::PageDown => {
                            ac.selected = (ac.selected + 9).min(ac.items.len() - 1);
                            continue;
                        }
                        Key::PageUp => {
                            ac.selected = ac.selected.saturating_sub(9);
                            continue;
                        }
                        Key::Enter | Key::Tab if !modifiers.any() => {
                            let i = ac.selected;
                            accept_autocomplete(vs, doc, i);
                            out.changed = true;
                            continue;
                        }
                        Key::Escape => {
                            vs.autocomplete = None;
                            continue;
                        }
                        Key::ArrowLeft | Key::ArrowRight | Key::Home | Key::End => {
                            vs.autocomplete = None
                        }
                        _ => {}
                    }
                }
                if let Some(action) = key_to_action(key, modifiers, mac) {
                    let is_edit = matches!(
                        action,
                        KeyAction::Delete(_)
                            | KeyAction::Newline
                            | KeyAction::Tab
                            | KeyAction::BackTab
                    );
                    if is_edit && doc.read_only {
                        continue;
                    }
                    // Begin/End select mode extends selections on movement.
                    let action = match action {
                        KeyAction::Move { m, extend: false } if vs.begin_select.is_some() => {
                            KeyAction::Move { m, extend: true }
                        }
                        a => a,
                    };
                    vs.exec(doc, action, settings, env.overtype);
                    if matches!(action, KeyAction::Delete(DeleteUnit::CharBack))
                        && vs.autocomplete.is_some()
                    {
                        open_autocomplete(vs, doc, env.keywords, true, true, 1, true);
                    }
                    if is_edit {
                        out.changed = true;
                    }
                }
            }
            _ => {}
        }
    }
}

fn key_to_action(key: Key, m: Modifiers, mac: bool) -> Option<KeyAction> {
    let word = if mac { m.alt } else { m.ctrl };
    let line_jump = mac && m.mac_cmd;
    let shift = m.shift;
    let mv = |mv: Move| {
        Some(KeyAction::Move {
            m: mv,
            extend: shift,
        })
    };
    // Column selection: Alt+Shift+arrows (Windows) / ⌥⇧ arrows on mac too.
    let rect = m.alt && m.shift && !m.command;
    Some(match key {
        Key::ArrowLeft if rect => KeyAction::RectMove(Move::Left),
        Key::ArrowRight if rect => KeyAction::RectMove(Move::Right),
        Key::ArrowUp if rect => KeyAction::RectMove(Move::Up),
        Key::ArrowDown if rect => KeyAction::RectMove(Move::Down),
        Key::PageUp if rect => KeyAction::RectMove(Move::PageUp),
        Key::PageDown if rect => KeyAction::RectMove(Move::PageDown),
        Key::Home if rect => KeyAction::RectMove(Move::Home),
        Key::End if rect => KeyAction::RectMove(Move::End),
        Key::ArrowLeft if line_jump => return mv(Move::Home),
        Key::ArrowRight if line_jump => return mv(Move::End),
        Key::ArrowUp if line_jump => return mv(Move::DocStart),
        Key::ArrowDown if line_jump => return mv(Move::DocEnd),
        Key::ArrowLeft if word => return mv(Move::WordLeft),
        Key::ArrowRight if word => return mv(Move::WordRight),
        Key::ArrowUp if m.ctrl && !shift => KeyAction::ScrollLines(-1),
        Key::ArrowDown if m.ctrl && !shift => KeyAction::ScrollLines(1),
        Key::ArrowLeft => return mv(Move::Left),
        Key::ArrowRight => return mv(Move::Right),
        Key::ArrowUp => return mv(Move::Up),
        Key::ArrowDown => return mv(Move::Down),
        Key::Home if m.command => return mv(Move::DocStart),
        Key::End if m.command => return mv(Move::DocEnd),
        Key::Home => return mv(Move::Home),
        Key::End => return mv(Move::End),
        Key::PageUp => return mv(Move::PageUp),
        Key::PageDown => return mv(Move::PageDown),
        Key::Backspace if m.command && shift => KeyAction::Delete(DeleteUnit::LineStart),
        Key::Backspace if word || (mac && m.ctrl) => KeyAction::Delete(DeleteUnit::WordBack),
        Key::Backspace if mac && m.mac_cmd => KeyAction::Delete(DeleteUnit::LineStart),
        Key::Backspace => KeyAction::Delete(DeleteUnit::CharBack),
        Key::Delete if m.command && shift => KeyAction::Delete(DeleteUnit::LineEnd),
        Key::Delete if word => KeyAction::Delete(DeleteUnit::WordForward),
        Key::Delete => KeyAction::Delete(DeleteUnit::CharForward),
        Key::Enter if !m.command && !m.alt => KeyAction::Newline,
        Key::Tab if !m.command && shift => KeyAction::BackTab,
        Key::Tab if !m.command && !m.alt => KeyAction::Tab,
        Key::Insert if !m.any() => KeyAction::ToggleOvertype,
        Key::Escape => KeyAction::Escape,
        _ => return None,
    })
}

pub fn copy(vs: &mut ViewState, doc: &mut Document, env: &mut EditorEnv, cut: bool) {
    let eol = doc.eol.as_str();
    let texts = vs.selected_texts(doc);
    if texts.is_empty() {
        if !env.settings.copy_line_without_selection {
            return;
        }
        // Copy/cut the whole current line.
        let (l, _) = doc.pos_to_line_col(vs.primary().head);
        let s = doc.line_start(l);
        let e = if l + 1 < doc.line_count() {
            doc.line_start(l + 1)
        } else {
            doc.len_chars()
        };
        let mut t = doc.slice(s, e);
        if l + 1 >= doc.line_count() {
            t.push_str(eol);
        }
        env.clip.record(&t, true, false);
        copy_to_os(&t);
        if cut && !doc.read_only {
            let before = vs.sels.clone();
            vs.sels = doc.edit(vec![(s, e, String::new())], &before, EditKind::Other, |f| {
                vec![Selection::caret(f[0].0)]
            });
            vs.after_edit(doc);
        }
        return;
    }
    let joined = texts.join(eol);
    let rect = vs.rect.is_some() || vs.sels.len() > 1;
    let joined = if rect && vs.rect.is_some() {
        format!("{joined}{eol}")
    } else {
        joined
    };
    env.clip.record(&joined, false, vs.rect.is_some());
    copy_to_os(&joined);
    if cut && !doc.read_only {
        ops::delete(doc, vs, DeleteUnit::CharBack);
    }
}

/// Puts text on the OS clipboard. Done via egui's platform output so it
/// works the same on macOS and Windows.
fn copy_to_os(text: &str) {
    COPY_REQUEST.with(|c| *c.borrow_mut() = Some(text.to_string()));
}

thread_local! {
    pub static COPY_REQUEST: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub fn take_copy_request() -> Option<String> {
    COPY_REQUEST.with(|c| c.borrow_mut().take())
}

pub fn paste(vs: &mut ViewState, doc: &mut Document, env: &mut EditorEnv, s: &str) {
    if doc.read_only || s.is_empty() {
        return;
    }
    let eol = doc.eol.as_str();
    let normalized = s.replace("\r\n", "\n").replace('\r', "\n");
    let converted = if eol == "\n" {
        normalized.clone()
    } else {
        normalized.replace('\n', eol)
    };
    let same_as_clip = env.clip.text.replace("\r\n", "\n").replace('\r', "\n") == normalized;
    if same_as_clip && env.clip.line_mode && vs.sels.iter().all(|x| x.is_empty()) {
        // Line-mode paste: insert above the caret line.
        let before = vs.sels.clone();
        let edits = vs
            .sels
            .iter()
            .map(|x| {
                let (l, _) = doc.pos_to_line_col(x.head);
                let ls = doc.line_start(l);
                (ls, ls, converted.clone())
            })
            .collect();
        let old = vs.sels.clone();
        let ver = doc.version;
        doc.edit(edits, &before, EditKind::Other, |_| old.clone());
        vs.sels = old
            .iter()
            .map(|x| Selection::caret(doc.map_pos(x.head, ver)))
            .collect();
        vs.after_edit(doc);
        return;
    }
    if same_as_clip && env.clip.rect_mode {
        // Rectangular paste at the caret column on successive lines.
        let lines: Vec<String> = normalized
            .trim_end_matches('\n')
            .split('\n')
            .map(str::to_string)
            .collect();
        let p = vs.primary();
        let (line, _) = doc.pos_to_line_col(p.start());
        let col = vs.display_col(doc, p.start());
        if !p.is_empty() {
            ops::delete(doc, vs, DeleteUnit::CharBack);
        }
        // Extend the document if needed.
        let missing = (line + lines.len()).saturating_sub(doc.line_count());
        if missing > 0 {
            let end = doc.len_chars();
            let before = vs.sels.clone();
            doc.edit(
                vec![(end, end, eol.repeat(missing))],
                &before,
                EditKind::Other,
                |_| before.clone(),
            );
        }
        ops::column_insert(doc, vs, line, col, &lines);
        return;
    }
    let parts: Vec<&str> = normalized.trim_end_matches('\n').split('\n').collect();
    if vs.sels.len() > 1 && parts.len() == vs.sels.len() {
        let texts: Vec<String> = parts.iter().map(|p| (*p).to_string()).collect();
        ops::insert_per_selection(doc, vs, &texts);
        return;
    }
    ops::insert_text(doc, vs, &converted, EditKind::Other);
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn handle_mouse(
    ui: &Ui,
    response: &Response,
    vs: &mut ViewState,
    doc: &mut Document,
    env: &mut EditorEnv,
    m: &Metrics,
    text_rect: Rect,
    gutter_rect: Rect,
    text_left: f32,
) {
    let wrap = vs.effective_wrap(doc, env.settings.word_wrap);
    let (pointer, mods, time, scroll) = ui.input(|i| {
        (
            i.pointer.clone(),
            i.modifiers,
            i.time,
            i.smooth_scroll_delta,
        )
    });
    let hovered = response.hovered() || response.contains_pointer();
    if hovered {
        if scroll != Vec2::ZERO {
            vs.top_row -= f64::from(scroll.y / m.lh);
            vs.top_row = vs.top_row.max(0.0);
            if !wrap {
                vs.scroll_x = (vs.scroll_x - scroll.x).max(0.0);
            }
        }
        if let Some(p) = pointer.hover_pos() {
            if text_rect.contains(p) {
                ui.ctx().set_cursor_icon(CursorIcon::Text);
            } else if gutter_rect.contains(p) {
                ui.ctx().set_cursor_icon(CursorIcon::Default);
            }
        }
    }
    let total_rows = vs.total_rows(doc).max(1);
    let hit = |vs: &ViewState, doc: &Document, p: Pos2| -> usize {
        let row_f = vs.top_row + f64::from((p.y - text_rect.min.y) / m.lh);
        let row = row_f.max(0.0).floor() as usize;
        if row >= total_rows {
            return doc.len_chars();
        }
        let col = ((p.x - text_left + vs.scroll_x) / m.cw).max(0.0).round() as usize;
        vs.row_col_to_pos(doc, row, col, wrap)
    };
    let hit_line = |vs: &ViewState, doc: &Document, p: Pos2| -> usize {
        let row = (vs.top_row + f64::from((p.y - text_rect.min.y) / m.lh))
            .max(0.0)
            .floor() as usize;
        vs.row_to_line(row.min(total_rows - 1), doc).0
    };
    let Some(pp) = pointer.interact_pos() else {
        return;
    };
    // `contains_pointer` is false when another window is in front.
    if pointer.primary_pressed()
        && response.contains_pointer()
        && !matches!(vs.drag, Some(Drag::VScroll { .. } | Drag::HScroll { .. }))
    {
        let in_scrollbar = pp.x > text_rect.max.x || pp.y > text_rect.max.y;
        if in_scrollbar {
            return;
        }
        vs.autocomplete = None;
        vs.begin_select = None;
        let count = match vs.last_press {
            Some((t, lp, c)) if time - t < 0.45 && lp.distance(pp) < 6.0 => (c % 3) + 1,
            _ => 1,
        };
        vs.last_press = Some((time, pp, count));
        if gutter_rect.contains(pp) {
            let line = hit_line(vs, doc, pp);
            let x = pp.x - gutter_rect.min.x;
            let bm_start = m.gutter_ln;
            let fold_start = m.gutter_ln + m.gutter_bm + m.gutter_ch;
            if m.gutter_fold > 0.0 && x >= fold_start {
                if doc.is_fold_header(line) {
                    let folded = doc.meta[line].folded;
                    doc.set_fold(line, !folded);
                }
            } else if m.gutter_bm > 0.0 && x >= bm_start && x < bm_start + m.gutter_bm {
                doc.toggle_bookmark(line);
            } else {
                // Line-number margin: select whole lines.
                let s = doc.line_start(line);
                let e = if line + 1 < doc.line_count() {
                    doc.line_start(line + 1)
                } else {
                    doc.len_chars()
                };
                if mods.shift {
                    let a = vs.primary().anchor;
                    vs.set_selection(a, if s < a { s } else { e });
                } else {
                    vs.set_selection(s, e);
                }
                vs.drag = Some(Drag::Gutter(line));
            }
            return;
        }
        let pos = hit(vs, doc, pp);
        if mods.alt && !mods.command {
            let line = doc.pos_to_line_col(pos).0;
            let col = vs.display_col(doc, pos);
            vs.set_rect(doc, (line, col), (line, col));
            vs.drag = Some(Drag::Rect);
            return;
        }
        match count {
            2 => {
                let (a, b) = ops::word_at(doc, pos);
                vs.set_selection(a, b);
                vs.drag = Some(Drag::Word(a, b));
            }
            3 => {
                let line = doc.pos_to_line_col(pos).0;
                let s = doc.line_start(line);
                let e = if line + 1 < doc.line_count() {
                    doc.line_start(line + 1)
                } else {
                    doc.len_chars()
                };
                vs.set_selection(s, e);
                vs.drag = Some(Drag::Line(line));
            }
            _ => {
                if mods.command && env.settings.multi_editing {
                    vs.sels.push(Selection::caret(pos));
                    vs.primary_idx = vs.sels.len() - 1;
                    vs.rect = None;
                } else if mods.shift {
                    let a = vs.primary().anchor;
                    vs.set_selection(a, pos);
                } else {
                    vs.set_single(pos);
                }
                vs.drag = Some(Drag::Text);
            }
        }
        vs.scroll_to_caret = false;
        return;
    }
    if !pointer.primary_down() {
        if vs.drag.is_some() {
            vs.drag = None;
        }
        return;
    }
    // Dragging: auto-scroll when outside the text area.
    if vs.drag.is_some() && !matches!(vs.drag, Some(Drag::VScroll { .. } | Drag::HScroll { .. })) {
        if pp.y < text_rect.min.y {
            vs.top_row = (vs.top_row - 1.0).max(0.0);
        } else if pp.y > text_rect.max.y {
            vs.top_row += 1.0;
        }
        if !wrap && pp.x > text_rect.max.x {
            vs.scroll_x += m.cw * 2.0;
        } else if !wrap && pp.x < text_rect.min.x && vs.scroll_x > 0.0 {
            vs.scroll_x = (vs.scroll_x - m.cw * 2.0).max(0.0);
        }
        ui.ctx().request_repaint();
    }
    let clamped = pos2(
        pp.x.clamp(text_rect.min.x, text_rect.max.x),
        pp.y.clamp(text_rect.min.y, text_rect.max.y),
    );
    match vs.drag {
        Some(Drag::Text) => {
            let pos = hit(vs, doc, clamped);
            if let Some(s) = vs.sels.get_mut(vs.primary_idx) {
                s.head = pos;
            }
        }
        Some(Drag::Word(a, b)) => {
            let pos = hit(vs, doc, clamped);
            let (wa, wb) = ops::word_at(doc, pos);
            if pos < a {
                vs.sels = vec![Selection::new(b, wa.min(pos))];
            } else {
                vs.sels = vec![Selection::new(a, wb.max(pos))];
            }
            vs.primary_idx = 0;
        }
        Some(Drag::Line(l0)) | Some(Drag::Gutter(l0)) => {
            let l = hit_line(vs, doc, clamped);
            let n = doc.line_count();
            let end_of = |l: usize| {
                if l + 1 < n {
                    doc.line_start(l + 1)
                } else {
                    doc.len_chars()
                }
            };
            let sel = if l >= l0 {
                Selection::new(doc.line_start(l0), end_of(l))
            } else {
                Selection::new(end_of(l0), doc.line_start(l))
            };
            vs.sels = vec![sel];
            vs.primary_idx = 0;
        }
        Some(Drag::Rect) => {
            if let Some((anchor, _)) = vs.rect {
                let pos = hit(vs, doc, clamped);
                let line = doc.pos_to_line_col(pos).0;
                let col = ((clamped.x - text_left + vs.scroll_x) / m.cw)
                    .max(0.0)
                    .round() as usize;
                let (row, _) = vs.pos_to_row_col(doc, doc.line_start(line), wrap);
                let _ = row;
                vs.set_rect(doc, anchor, (line, col));
                vs.scroll_to_caret = false;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_highlight_stops_at_line_boundaries() {
        // "abc\ndef\n": line 0 is 0..3 with its line break at 3; line 1
        // starts at 4.
        let row0 = |ss, se| selection_on_row(ss, se, 0, 3, 1);
        let row1 = |ss, se| selection_on_row(ss, se, 4, 7, 1);
        // Dragging right from the start of line 1 leaves line 0 alone.
        assert_eq!(row0(4, 6), None);
        assert_eq!(row1(4, 6), Some((4, 6, false)));
        // Starting on line 0's line break shows it selected.
        assert_eq!(row0(3, 6), Some((3, 3, true)));
        // Ending at the start of line 1 includes line 0's line break.
        assert_eq!(row0(1, 4), Some((1, 3, true)));
        assert_eq!(row1(1, 4), Some((4, 4, false)));
        // Within line 0.
        assert_eq!(row0(1, 2), Some((1, 2, false)));
        // CRLF: a selection starting between \r and \n still shows it.
        assert_eq!(selection_on_row(4, 6, 0, 3, 2), Some((3, 3, true)));
        assert_eq!(selection_on_row(5, 7, 0, 3, 2), None);
        // A wrapped row (no line break after it) shows a selection starting
        // at its end as an empty span, and nothing past it.
        assert_eq!(selection_on_row(3, 5, 0, 3, 0), Some((3, 3, false)));
        assert_eq!(selection_on_row(4, 5, 0, 3, 0), None);
    }

    fn completions(text: &str) -> Vec<String> {
        let langs = crate::langs::LangRegistry::load();
        let doc = Document::from_text(1, "t".into(), text, &langs, langs.normal());
        let mut vs = ViewState::default();
        vs.set_single(doc.len_chars());
        open_autocomplete(
            &mut vs,
            &doc,
            &["continue".to_string()],
            true,
            true,
            1,
            true,
        );
        vs.autocomplete.map(|a| a.items).unwrap_or_default()
    }

    #[test]
    fn completion_lists_declarations_first_and_members_after_dots() {
        let src = "obj.count_items();\ncountdown();\nlet counter = 1;\nco";
        assert_eq!(
            completions(src),
            ["counter", "continue", "count_items", "countdown"]
        );
        // After a dot: only names used after dots, no keywords.
        let src = "self.velocity = 1;\nlet value = 2;\nvalue_x();\nx.v";
        assert_eq!(completions(src), ["velocity"]);
    }

    #[test]
    fn column_math() {
        assert_eq!(columns("a\tb", 4), vec![0, 1, 4, 5]);
        assert_eq!(columns("世a", 4), vec![0, 2, 3]);
        let vs = ViewState::default();
        assert_eq!(vs.col_to_char_in("a\tb", 3), (2, 4));
        assert_eq!(vs.col_to_char_in("a\tb", 2), (1, 1));
        assert_eq!(vs.col_to_char_in("ab", 10), (2, 2));
    }

    #[test]
    fn wrapping_prefers_spaces() {
        assert_eq!(wrap_breaks("hello world foo", 4, 11), vec![0, 12]);
        assert_eq!(wrap_breaks("abcdefghij", 4, 4), vec![0, 4, 8]);
        assert_eq!(wrap_breaks("short", 4, 80), vec![0]);
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::document::{Document, EditKind};
    use crate::langs::LangRegistry;
    use std::time::Instant;

    /// Cost of one keystroke plus the word-completion popup on a big file.
    #[test]
    #[ignore = "benchmark: BENCH_FILE=path cargo test --release typing_bench -- --ignored --nocapture"]
    fn typing_bench() {
        let path = std::env::var("BENCH_FILE").expect("BENCH_FILE");
        let langs = LangRegistry::load();
        let mut doc = Document::open(1, std::path::Path::new(&path), &langs, None).unwrap();
        let mut vs = ViewState::default();
        let mid = doc.line_start(doc.line_count() / 2);
        vs.set_single(mid);
        let (mut edit_t, mut ac_t) = (Vec::new(), Vec::new());
        for c in "notifyX".chars() {
            let head = vs.primary().head;
            let t = Instant::now();
            doc.edit(
                vec![(head, head, c.to_string())],
                &vs.sels,
                EditKind::Typing,
                |f| vec![Selection::caret(f[0].1)],
            );
            vs.set_single(head + 1);
            edit_t.push(t.elapsed());
            let t = Instant::now();
            open_autocomplete(&mut vs, &doc, &[], true, false, 3, false);
            ac_t.push(t.elapsed());
        }
        println!("edit per key: {edit_t:.2?}\nautocomplete per key: {ac_t:.2?}");
    }
}
