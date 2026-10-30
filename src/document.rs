//! The text buffer: rope storage, undo/redo, per-line metadata, lexing cache.
//!
//! Text storage, undo history and per-line state for one open file.
//! All positions are *char* indices into the rope.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use ropey::Rope;

use crate::encoding::{Eol, TextEncoding};
use crate::langs::{FoldMode, LangRegistry};
use crate::lexer::{LexState, Lexer, Span};

pub type DocId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(pos: usize) -> Self {
        Self {
            anchor: pos,
            head: pos,
        }
    }

    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    pub fn start(self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }
}

/// Sorts selections and merges overlapping ones (keeps direction of the last).
pub fn normalize_selections(sels: &mut Vec<Selection>) {
    if sels.len() <= 1 {
        return;
    }
    sels.sort_by_key(|s| (s.start(), s.end()));
    let mut out: Vec<Selection> = Vec::with_capacity(sels.len());
    for s in sels.drain(..) {
        if let Some(last) = out.last_mut()
            && (s.start() < last.end()
                || (s.start() == last.end()
                    && (s.is_empty() || last.is_empty())
                    && s.start() == last.start()))
        {
            let start = last.start().min(s.start());
            let end = last.end().max(s.end());
            *last = if s.head < s.anchor {
                Selection::new(end, start)
            } else {
                Selection::new(start, end)
            };
            continue;
        }
        out.push(s);
    }
    *sels = out;
}

#[derive(Clone, Debug)]
struct Change {
    pos: usize,
    old: String,
    new: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Other,
}

#[derive(Clone, Debug)]
struct Transaction {
    changes: Vec<Change>,
    before: Vec<Selection>,
    after: Vec<Selection>,
    kind: EditKind,
    time: Instant,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LineMeta {
    pub bookmark: bool,
    /// 0 = unchanged, 1 = modified (unsaved), 2 = modified and saved.
    pub change: u8,
    /// Fold header collapsed.
    pub folded: bool,
}

/// A styled range: 0 = "Find Mark", 1..=5 = "Style token" styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub start: usize,
    pub end: usize,
    pub style: u8,
}

pub struct LexCache {
    lexer: Lexer,
    /// Syntax highlighting on. When off, no line is lexed and folding
    /// follows indentation.
    enabled: bool,
    /// State at the start of each line; `states.len() - 1` lines are valid.
    states: Vec<LexState>,
    /// Fold level at the start of each line (parallel to `states`).
    levels: Vec<i32>,
    /// States computed before the last edit for the lines below it. When
    /// re-lexing reaches a line whose new state matches, the rest is reused.
    tail: Option<LexTail>,
}

/// Lexer states of the unchanged lines after an edit, already shifted to
/// their new line numbers: `states[i]` belongs to line `first + i - skip`.
struct LexTail {
    first: usize,
    skip: usize,
    states: Vec<LexState>,
    levels: Vec<i32>,
}

/// Background colouring stops at this many lines; beyond it, lines are
/// still coloured on demand when they're shown.
const BACKGROUND_LEX_MAX_LINES: usize = 2_000_000;

pub struct Document {
    pub id: DocId,
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub untitled_name: String,
    pub encoding: TextEncoding,
    pub eol: Eol,
    /// Line endings in the text: [CR LF, LF, CR], kept up to date by edits.
    pub eol_counts: [usize; 3],
    pub lang: usize,
    pub read_only: bool,
    pub version: u64,
    pub meta: Vec<LineMeta>,
    pub marks: Vec<Mark>,
    pub mtime: Option<SystemTime>,
    pub tab_color: Option<u8>,
    pub pinned: bool,
    /// Set when the encoding was converted (no text change but still unsaved).
    pub force_dirty: bool,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    save_point: Option<usize>,
    change_log: VecDeque<(u64, usize, usize, usize)>,
    lex: LexCache,
    text_cache: RefCell<Option<(u64, Arc<String>)>>,
    pub tab_width: usize,
    /// Bumped whenever fold state changes (layout cache key).
    pub fold_gen: u64,
    /// Lines left visible by the line filter, if one is active.
    pub line_filter: Option<Arc<Vec<usize>>>,
    /// A temporary, generated view (compare snapshot, inline diff) that
    /// isn't saved with the session.
    pub transient: bool,
}

const CHANGE_LOG_MAX: usize = 4096;

fn count_line_breaks(s: &str) -> usize {
    let b = s.as_bytes();
    let mut n = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\n' => n += 1,
            b'\r' => {
                n += 1;
                if i + 1 < b.len() && b[i + 1] == b'\n' {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    n
}

impl Document {
    pub fn new(
        id: DocId,
        untitled_name: String,
        langs: &LangRegistry,
        lang: usize,
        eol: Eol,
        encoding: TextEncoding,
    ) -> Self {
        let lexer = Lexer::new(&langs.langs[lang], langs);
        let init = lexer.initial_state();
        Self {
            id,
            rope: Rope::new(),
            path: None,
            untitled_name,
            encoding,
            eol,
            eol_counts: [0; 3],
            lang,
            read_only: false,
            version: 0,
            meta: vec![LineMeta::default()],
            marks: Vec::new(),
            mtime: None,
            tab_color: None,
            pinned: false,
            force_dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            save_point: Some(0),
            change_log: VecDeque::new(),
            lex: LexCache {
                lexer,
                enabled: true,
                states: vec![init],
                levels: vec![0],
                tail: None,
            },
            text_cache: RefCell::new(None),
            tab_width: 4,
            fold_gen: 0,
            line_filter: None,
            transient: false,
        }
    }

    pub fn from_text(
        id: DocId,
        name: String,
        text: &str,
        langs: &LangRegistry,
        lang: usize,
    ) -> Self {
        let eol = Eol::detect(text).unwrap_or_else(Eol::platform_default);
        let mut d = Self::new(id, name, langs, lang, eol, TextEncoding::Utf8);
        d.rope = Rope::from_str(text);
        d.meta = vec![LineMeta::default(); d.rope.len_lines()];
        d.eol_counts = Eol::count(text);
        d
    }

    /// True when the document mixes different line endings.
    pub fn has_mixed_eol(&self) -> bool {
        self.eol_counts.iter().filter(|&&n| n > 0).count() > 1
    }

    /// Line-ending counts of the text around an edit, one char either side, so
    /// CR LF pairs split or joined by the edit are counted correctly.
    fn eol_counts_around(&self, pos: usize, len: usize) -> [usize; 3] {
        let a = pos.saturating_sub(1);
        let b = (pos + len + 1).min(self.rope.len_chars());
        Eol::count(&self.rope.slice(a..b).to_string())
    }

    /// Loads a file from disk, detecting its encoding, EOL style and language.
    pub fn open(
        id: DocId,
        path: &Path,
        langs: &LangRegistry,
        forced: Option<TextEncoding>,
    ) -> io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let (encoding, text) = match forced {
            Some(enc) => (enc, enc.decode(&bytes)),
            None => TextEncoding::detect_and_decode(bytes),
        };
        let first_line = text.lines().next().unwrap_or_default();
        let file_name = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let lang = langs.detect(&file_name, first_line);
        let mut d = Self::from_text(id, file_name, &text, langs, lang);
        d.encoding = encoding;
        d.path = Some(path.to_path_buf());
        d.mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        d.read_only = std::fs::metadata(path).is_ok_and(|m| m.permissions().readonly());
        Ok(d)
    }

    /// Replaces the whole text with freshly loaded file content (File > Reload).
    pub fn reload(&mut self, langs: &LangRegistry) -> io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        let bytes = std::fs::read(&path)?;
        let text = self.encoding.decode(&bytes);
        let len = self.len_chars();
        let before = vec![Selection::caret(0)];
        self.edit(vec![(0, len, text)], &before, EditKind::Other, |_| {
            vec![Selection::caret(0)]
        });
        self.undo.clear();
        self.redo.clear();
        self.save_point = Some(0);
        for m in &mut self.meta {
            m.change = 0;
        }
        self.force_dirty = false;
        self.mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let _ = langs;
        Ok(())
    }

    pub fn display_name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_name()).map_or_else(
            || self.untitled_name.clone(),
            |f| f.to_string_lossy().to_string(),
        )
    }

    pub fn full_path_or_name(&self) -> String {
        self.path
            .as_ref()
            .map_or_else(|| self.untitled_name.clone(), |p| p.display().to_string())
    }

    pub fn is_dirty(&self) -> bool {
        self.force_dirty || self.save_point != Some(self.undo.len())
    }

    pub fn is_untitled(&self) -> bool {
        self.path.is_none()
    }

    pub fn is_empty_untitled(&self) -> bool {
        self.path.is_none() && self.rope.len_chars() == 0 && !self.is_dirty()
    }

    pub fn set_language(&mut self, lang: usize, langs: &LangRegistry) {
        self.lang = lang;
        self.lex.lexer = Lexer::new(&langs.langs[lang], langs);
        self.lex.states = vec![self.lex.lexer.initial_state()];
        self.lex.levels = vec![0];
        self.lex.tail = None;
        for m in &mut self.meta {
            m.folded = false;
        }
        self.version += 1;
        self.fold_gen += 1;
    }

    // ---------------------------------------------------------------- text access

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line.min(self.line_count()))
    }

    /// Length of a line in chars, excluding the line terminator.
    pub fn line_len(&self, line: usize) -> usize {
        if line >= self.line_count() {
            return 0;
        }
        let sl = self.rope.line(line);
        let mut n = sl.len_chars();
        if n > 0 && sl.char(n - 1) == '\n' {
            n -= 1;
        }
        if n > 0 && sl.char(n - 1) == '\r' {
            n -= 1;
        }
        n
    }

    pub fn line_end(&self, line: usize) -> usize {
        self.line_start(line) + self.line_len(line)
    }

    /// Line text without the terminator.
    pub fn line_text(&self, line: usize) -> String {
        if line >= self.line_count() {
            return String::new();
        }
        let s = self.line_start(line);
        self.rope.slice(s..s + self.line_len(line)).to_string()
    }

    pub fn char_to_line(&self, pos: usize) -> usize {
        self.rope.char_to_line(pos.min(self.len_chars()))
    }

    pub fn pos_to_line_col(&self, pos: usize) -> (usize, usize) {
        let pos = pos.min(self.len_chars());
        let line = self.rope.char_to_line(pos);
        (line, pos - self.rope.line_to_char(line))
    }

    pub fn char_at(&self, pos: usize) -> Option<char> {
        (pos < self.len_chars()).then(|| self.rope.char(pos))
    }

    pub fn slice(&self, start: usize, end: usize) -> String {
        let len = self.len_chars();
        let (s, e) = (start.min(len), end.min(len));
        self.rope.slice(s.min(e)..e.max(s)).to_string()
    }

    /// Full text, cached per version (used by search, function list...).
    pub fn text(&self) -> Arc<String> {
        let mut c = self.text_cache.borrow_mut();
        if let Some((v, t)) = c.as_ref()
            && *v == self.version
        {
            return t.clone();
        }
        let t = Arc::new(self.rope.to_string());
        *c = Some((self.version, t.clone()));
        t
    }

    pub fn byte_to_char(&self, b: usize) -> usize {
        self.rope.byte_to_char(b.min(self.rope.len_bytes()))
    }

    pub fn char_to_byte(&self, c: usize) -> usize {
        self.rope.char_to_byte(c.min(self.len_chars()))
    }

    // ---------------------------------------------------------------- editing

    /// Applies non-overlapping `(start, end, replacement)` edits given in
    /// current coordinates (any order). `make_after` receives the final
    /// `(start, end)` of each inserted text, in the order of `edits` sorted
    /// ascending, and returns the selections after the edit.
    pub fn edit(
        &mut self,
        mut edits: Vec<(usize, usize, String)>,
        before: &[Selection],
        kind: EditKind,
        make_after: impl FnOnce(&[(usize, usize)]) -> Vec<Selection>,
    ) -> Vec<Selection> {
        edits.sort_by_key(|e| (e.0, e.1));
        edits.dedup_by(|b, a| a.0 == b.0 && a.1 == b.1 && b.2.is_empty() && a.2.is_empty());
        let len = self.len_chars();
        let mut finals = Vec::with_capacity(edits.len());
        let mut delta: isize = 0;
        for (s, e, t) in &edits {
            let s = (*s).min(len);
            let e = (*e).min(len).max(s);
            let fs = s.cast_signed() + delta;
            let ins = t.chars().count();
            finals.push((fs.cast_unsigned(), fs.cast_unsigned() + ins));
            delta += ins.cast_signed() - (e - s).cast_signed();
        }
        let after = make_after(&finals);
        let mut changes = Vec::with_capacity(edits.len());
        for (s, e, t) in edits.into_iter().rev() {
            let s = s.min(len);
            let e = e.min(len).max(s);
            if s == e && t.is_empty() {
                continue;
            }
            let old = self.rope.slice(s..e).to_string();
            if old == t {
                continue;
            }
            self.raw_replace(s, e - s, &t);
            changes.push(Change {
                pos: s,
                old,
                new: t,
            });
        }
        if changes.is_empty() {
            return after;
        }
        self.redo.clear();
        let now = Instant::now();
        let mergeable = kind == EditKind::Typing
            && self.save_point != Some(self.undo.len())
            && self.undo.last().is_some_and(|t| {
                t.kind == EditKind::Typing
                    && t.after == before
                    && now.duration_since(t.time).as_secs() < 5
            });
        if mergeable {
            let last = self.undo.last_mut().unwrap();
            last.changes.extend(changes);
            last.after.clone_from(&after);
            last.time = now;
        } else {
            if let Some(sp) = self.save_point
                && sp > self.undo.len()
            {
                self.save_point = None;
            }
            self.undo.push(Transaction {
                changes,
                before: before.to_vec(),
                after: after.clone(),
                kind,
                time: now,
            });
        }
        after
    }

    fn raw_replace(&mut self, pos: usize, del: usize, new: &str) {
        let lines_before = self.rope.len_lines();
        let start_line = self.rope.char_to_line(pos);
        let end_line_old = self.rope.char_to_line(pos + del);
        let eol_before = self.eol_counts_around(pos, del);
        // A deletion that starts right after "\r" and inserts "\n" can merge lines;
        // the line-count delta below handles that uniformly.
        if del > 0 {
            self.rope.remove(pos..pos + del);
        }
        if !new.is_empty() {
            self.rope.insert(pos, new);
        }
        let lines_after = self.rope.len_lines();
        let delta = lines_after.cast_signed() - lines_before.cast_signed();
        let eol_after = self.eol_counts_around(pos, new.chars().count());
        for i in 0..3 {
            self.eol_counts[i] = self.eol_counts[i] + eol_after[i] - eol_before[i];
        }
        if delta > 0 {
            let at = (start_line + 1).min(self.meta.len());
            self.meta.splice(
                at..at,
                std::iter::repeat_n(LineMeta::default(), delta.cast_unsigned()),
            );
        } else if delta < 0 {
            let at = (start_line + 1).min(self.meta.len());
            let end = (at + (-delta).cast_unsigned()).min(self.meta.len());
            let any_bm = self.meta[at..end].iter().any(|m| m.bookmark);
            self.meta.drain(at..end);
            if any_bm && let Some(m) = self.meta.get_mut(start_line) {
                m.bookmark = true;
            }
        }
        self.meta.resize(lines_after, LineMeta::default());
        let touched = 1 + count_line_breaks(new).min(lines_after);
        for l in start_line..(start_line + touched).min(self.meta.len()) {
            self.meta[l].change = 1;
        }
        // Shift style marks.
        let ins = new.chars().count();
        self.marks.retain_mut(|m| {
            if m.start >= pos + del {
                m.start = m.start + ins - del;
                m.end = m.end + ins - del;
                true
            } else {
                m.end <= pos
            }
        });
        // Re-lex from the edited line on, but keep the old states of the
        // untouched lines below so re-lexing can stop once it catches up.
        let keep = (start_line + 1).min(self.lex.states.len());
        let old_first = end_line_old + 1;
        self.lex.tail =
            (old_first < self.lex.states.len() && self.lex.tail.is_none()).then(|| LexTail {
                first: (old_first.cast_signed() + delta).cast_unsigned(),
                skip: 0,
                states: self.lex.states.split_off(old_first),
                levels: self.lex.levels.split_off(old_first),
            });
        self.lex.states.truncate(keep);
        self.lex.levels.truncate(keep);
        self.version += 1;
        self.change_log.push_back((self.version, pos, del, ins));
        if self.change_log.len() > CHANGE_LOG_MAX {
            self.change_log.pop_front();
        }
    }

    pub fn undo(&mut self) -> Option<Vec<Selection>> {
        let t = self.undo.pop()?;
        for c in t.changes.iter().rev() {
            self.raw_replace(c.pos, c.new.chars().count(), &c.old);
        }
        let sel = t.before.clone();
        self.redo.push(t);
        Some(sel)
    }

    pub fn redo(&mut self) -> Option<Vec<Selection>> {
        let t = self.redo.pop()?;
        for c in &t.changes {
            self.raw_replace(c.pos, c.old.chars().count(), &c.new);
        }
        let sel = t.after.clone();
        self.undo.push(t);
        Some(sel)
    }

    /// Maps a position from `since_version` to the current version.
    pub fn map_pos(&self, mut pos: usize, since_version: u64) -> usize {
        if since_version >= self.version {
            return pos.min(self.len_chars());
        }
        let oldest = self.change_log.front().map_or(u64::MAX, |c| c.0);
        if oldest > since_version + 1 {
            return pos.min(self.len_chars());
        }
        for &(v, p, del, ins) in &self.change_log {
            if v <= since_version {
                continue;
            }
            if pos >= p + del {
                pos = pos + ins - del;
            } else if pos > p {
                pos = p + ins;
            }
        }
        pos.min(self.len_chars())
    }

    pub fn mark_saved(&mut self) {
        self.save_point = Some(self.undo.len());
        self.force_dirty = false;
        for m in &mut self.meta {
            if m.change == 1 {
                m.change = 2;
            }
        }
    }

    pub fn clear_change_history(&mut self) {
        for m in &mut self.meta {
            m.change = 0;
        }
    }

    /// Writes the buffer to `path` with the document's encoding.
    pub fn save_to(&mut self, path: &Path, backup: bool) -> io::Result<()> {
        if backup && path.exists() {
            let mut bak = path.as_os_str().to_owned();
            bak.push(".bak");
            let _ = std::fs::copy(path, PathBuf::from(bak));
        }
        let bytes = self.encoding.encode(&self.rope.to_string());
        std::fs::write(path, bytes)?;
        self.mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        Ok(())
    }

    // ---------------------------------------------------------------- lexing / folding

    fn ensure_lexed(&mut self, upto_line: usize) {
        let target = upto_line.min(self.line_count());
        let mut spans: Vec<Span> = Vec::new();
        while self.lex.states.len() <= target {
            self.lex_next_line(&mut spans);
        }
    }

    /// Lexes the first line without a known end state. If its end state
    /// matches the pre-edit state of the next line, the remaining pre-edit
    /// states are reused (shifting fold levels by the difference).
    fn lex_next_line(&mut self, spans: &mut Vec<Span>) {
        let l = self.lex.states.len() - 1;
        let st = self.lex.states[l];
        let text = self.line_text(l);
        let (next, fold) = self.lex.lexer.lex_line(&text, st, spans);
        self.lex.states.push(next);
        let level = self.lex.levels[l] + fold;
        self.lex.levels.push(level);
        let line = l + 1;
        let Some(t) = &mut self.lex.tail else {
            return;
        };
        if line < t.first {
            return;
        }
        let i = t.skip + (line - t.first);
        if i >= t.states.len() {
            self.lex.tail = None;
            return;
        }
        if t.states[i] == next {
            let offset = level - t.levels[i];
            let t = self.lex.tail.take().expect("tail");
            let n = self.line_count() + 1;
            let room = n.saturating_sub(self.lex.states.len());
            self.lex
                .states
                .extend(t.states[i + 1..].iter().copied().take(room));
            self.lex
                .levels
                .extend(t.levels[i + 1..].iter().map(|v| v + offset).take(room));
        }
    }

    /// Colours more of the document in the background, for up to `budget`.
    /// Returns true while there is more to do.
    pub fn lex_ahead(&mut self, budget: std::time::Duration) -> bool {
        if !self.is_lexing_pending() {
            return false;
        }
        let target = self.line_count().min(BACKGROUND_LEX_MAX_LINES);
        let start = std::time::Instant::now();
        let mut spans: Vec<Span> = Vec::new();
        while self.lex.states.len() <= target {
            for _ in 0..256 {
                if self.lex.states.len() > target {
                    return false;
                }
                self.lex_next_line(&mut spans);
            }
            if start.elapsed() >= budget {
                break;
            }
        }
        self.lex.states.len() <= target
    }

    /// True while background colouring still has lines to do.
    pub fn is_lexing_pending(&self) -> bool {
        self.lex.enabled
            && !self.is_plain_lexer()
            && self.lex.states.len() <= self.line_count().min(BACKGROUND_LEX_MAX_LINES)
    }

    fn is_plain_lexer(&self) -> bool {
        self.lex.lexer.is_plain()
    }

    /// Turns syntax highlighting on or off. Collapsed folds are opened,
    /// because the fold structure changes with it.
    pub fn set_highlighting(&mut self, on: bool) {
        if self.lex.enabled == on {
            return;
        }
        self.lex.enabled = on;
        for m in &mut self.meta {
            m.folded = false;
        }
        self.version += 1;
        self.fold_gen += 1;
    }

    /// Syntax spans for one line (byte offsets into `line_text`).
    pub fn line_spans(&mut self, line: usize, text: &str) -> Vec<Span> {
        // Plain text has no colours, so there is nothing to lex up to `line`.
        if !self.lex.enabled || self.is_plain_lexer() {
            return Vec::new();
        }
        self.ensure_lexed(line);
        let mut spans = Vec::new();
        if let Some(&st) = self.lex.states.get(line) {
            self.lex.lexer.lex_line(text, st, &mut spans);
        }
        spans
    }

    pub fn fold_mode(&self) -> FoldMode {
        match self.lex.lexer.fold_mode() {
            FoldMode::None => FoldMode::None,
            _ if !self.lex.enabled => FoldMode::Indent,
            m => m,
        }
    }

    fn indent_of(&self, line: usize) -> Option<usize> {
        let s = self.line_start(line);
        let len = self.line_len(line);
        let mut w = 0;
        for c in self.rope.slice(s..s + len).chars() {
            match c {
                ' ' => w += 1,
                '\t' => w += self.tab_width - (w % self.tab_width),
                _ => return Some(w),
            }
        }
        None // blank line
    }

    /// If `line` is a fold header, returns the last line of its fold body.
    pub fn fold_range(&mut self, line: usize) -> Option<usize> {
        let n = self.line_count();
        if line + 1 >= n {
            return None;
        }
        match self.fold_mode() {
            FoldMode::None => None,
            FoldMode::Indent => {
                let base = self.indent_of(line)?;
                let mut next = line + 1;
                while next < n && self.indent_of(next).is_none() {
                    next += 1;
                }
                if next >= n || self.indent_of(next)? <= base {
                    return None;
                }
                let mut last = next;
                let mut l = next + 1;
                while l < n {
                    match self.indent_of(l) {
                        Some(ind) if ind <= base => break,
                        Some(_) => last = l,
                        None => {}
                    }
                    l += 1;
                }
                Some(last)
            }
            FoldMode::Braces | FoldMode::Markup | FoldMode::Keywords => {
                self.ensure_lexed(line + 1);
                let base = self.lex.levels[line];
                if self.lex.levels[line + 1] <= base {
                    return None;
                }
                let mut m = line + 1;
                loop {
                    if m + 1 >= n {
                        return Some(n - 1);
                    }
                    self.ensure_lexed(m + 1);
                    if self.lex.levels[m + 1] <= base {
                        return Some(m);
                    }
                    m += 1;
                }
            }
        }
    }

    pub fn is_fold_header(&mut self, line: usize) -> bool {
        let n = self.line_count();
        if line + 1 >= n {
            return false;
        }
        match self.fold_mode() {
            FoldMode::None => false,
            FoldMode::Indent => {
                let Some(base) = self.indent_of(line) else {
                    return false;
                };
                let mut next = line + 1;
                while next < n && self.indent_of(next).is_none() {
                    next += 1;
                }
                next < n && self.indent_of(next).is_some_and(|i| i > base)
            }
            _ => {
                self.ensure_lexed(line + 1);
                self.lex.levels[line + 1] > self.lex.levels[line]
            }
        }
    }

    /// Fold nesting depth of a line (used for "fold level N").
    pub fn fold_depth(&mut self, line: usize) -> usize {
        match self.fold_mode() {
            FoldMode::Indent => {
                let ind = self.indent_of(line).unwrap_or(0);
                ind / self.tab_width.max(1)
            }
            FoldMode::None => 0,
            _ => {
                self.ensure_lexed(line);
                usize::try_from(self.lex.levels[line].max(0)).unwrap_or(0)
            }
        }
    }

    /// Lines hidden by collapsed folds, as sorted inclusive ranges.
    pub fn hidden_ranges(&mut self) -> Vec<(usize, usize)> {
        let headers: Vec<usize> = self
            .meta
            .iter()
            .enumerate()
            .filter(|(_, m)| m.folded)
            .map(|(i, _)| i)
            .collect();
        let mut out: Vec<(usize, usize)> = Vec::new();
        for h in headers {
            if out.last().is_some_and(|&(_, e)| h <= e) {
                continue; // nested inside an already-hidden range
            }
            match self.fold_range(h) {
                Some(end) => out.push((h + 1, end)),
                None => {
                    self.meta[h].folded = false;
                    self.fold_gen += 1;
                }
            }
        }
        out
    }

    /// Shows only `lines` (sorted line numbers), or every line with `None`.
    pub fn set_line_filter(&mut self, lines: Option<Arc<Vec<usize>>>) {
        self.line_filter = lines;
        self.fold_gen += 1;
    }

    pub fn has_folds(&self) -> bool {
        self.meta.iter().any(|m| m.folded)
    }

    pub fn set_fold(&mut self, line: usize, folded: bool) {
        if folded && !self.is_fold_header(line) {
            return;
        }
        if let Some(m) = self.meta.get_mut(line) {
            m.folded = folded;
            self.fold_gen += 1;
        }
    }

    /// Fold/unfold every header, optionally only at a given depth (1-based).
    pub fn fold_all(&mut self, folded: bool, depth: Option<usize>) {
        self.fold_gen += 1;
        let n = self.line_count();
        if folded {
            self.ensure_lexed(n);
        }
        for l in 0..n {
            if depth.is_some_and(|d| self.fold_depth(l) + 1 != d) {
                continue;
            }
            if folded {
                if self.is_fold_header(l) {
                    self.meta[l].folded = true;
                }
            } else {
                self.meta[l].folded = false;
            }
        }
    }

    /// Fold header that contains `line` (the line itself if it's a header).
    pub fn enclosing_header(&mut self, line: usize) -> Option<usize> {
        let mut l = line as isize;
        while l >= 0 {
            let lu = l.cast_unsigned();
            if self.is_fold_header(lu)
                && let Some(end) = self.fold_range(lu)
                && end >= line
            {
                return Some(lu);
            }
            l -= 1;
        }
        None
    }

    /// Unfolds any collapsed header hiding `line`.
    pub fn ensure_line_visible(&mut self, line: usize) {
        if !self.has_folds() {
            return;
        }
        loop {
            let ranges = self.hidden_ranges();
            let Some(&(s, _)) = ranges.iter().find(|(s, e)| *s <= line && line <= *e) else {
                break;
            };
            self.meta[s - 1].folded = false;
            self.fold_gen += 1;
        }
    }

    // ---------------------------------------------------------------- bookmarks

    pub fn toggle_bookmark(&mut self, line: usize) {
        if let Some(m) = self.meta.get_mut(line) {
            m.bookmark = !m.bookmark;
        }
    }

    pub fn bookmarked_lines(&self) -> Vec<usize> {
        self.meta
            .iter()
            .enumerate()
            .filter(|(_, m)| m.bookmark)
            .map(|(i, _)| i)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        let langs = LangRegistry::load();
        let cpp = langs.by_name("cpp").unwrap();
        Document::from_text(1, "t".into(), text, &langs, cpp)
    }

    /// Lexes everything from scratch and returns (states, levels).
    fn fresh_lex(d: &Document) -> (Vec<LexState>, Vec<i32>) {
        let langs = LangRegistry::load();
        let mut f = Document::from_text(2, "f".into(), &d.rope.to_string(), &langs, d.lang);
        f.ensure_lexed(f.line_count());
        (f.lex.states, f.lex.levels)
    }

    #[test]
    fn incremental_lexing_matches_a_fresh_lex() {
        let base = "int a = 1; // c\n/* block\n comment */\nvoid f() {\n  if (x) {\n    s = \"str\";\n  }\n}\n";
        let mut d = doc(&base.repeat(40));
        let pieces = ["/*", "*/", "\"", "{", "}", "\n", "x", "// ", "\n\n", "'"];
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = |m: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            usize::try_from(seed >> 33).unwrap() % m
        };
        for step in 0..400 {
            // Lex part of the document, like a frame showing some lines.
            let upto = next(d.line_count() + 1);
            d.ensure_lexed(upto);
            let len = d.len_chars();
            let pos = next(len + 1);
            let del = if next(3) == 0 {
                next(12).min(len - pos)
            } else {
                0
            };
            let ins = pieces[next(pieces.len())];
            let sels = vec![Selection::caret(pos)];
            d.edit(
                vec![(pos, pos + del, ins.into())],
                &sels,
                EditKind::Other,
                |_| sels.clone(),
            );
            if step % 7 == 0 {
                d.ensure_lexed(d.line_count());
                let (states, levels) = fresh_lex(&d);
                assert_eq!(d.lex.states, states, "states differ after step {step}");
                assert_eq!(d.lex.levels, levels, "fold levels differ after step {step}");
            }
        }
    }

    #[test]
    fn background_lexing_fills_the_whole_document() {
        let mut d = doc(&"void f() { /* x */ }\n".repeat(5000));
        let mut rounds = 0;
        while d.lex_ahead(std::time::Duration::from_millis(1)) {
            rounds += 1;
            assert!(rounds < 10_000);
        }
        assert_eq!(d.lex.states.len(), d.line_count() + 1);
        assert_eq!((d.lex.states.clone(), d.lex.levels.clone()), fresh_lex(&d));
        // Turned off, it does nothing.
        let mut d = doc("int x;\n");
        d.set_highlighting(false);
        assert!(!d.lex_ahead(std::time::Duration::from_millis(1)));
        assert_eq!(d.lex.states.len(), 1);
    }

    #[test]
    fn eol_counts_follow_edits() {
        let mut d = doc("a\r\nb\nc");
        assert_eq!(d.eol_counts, [1, 1, 0]);
        assert!(d.has_mixed_eol());
        let mut seed = 7u64;
        let pieces = ["\r", "\n", "\r\n", "x", ""];
        for _ in 0..500 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let r = usize::try_from(seed >> 33).unwrap();
            let len = d.len_chars();
            let pos = r % (len + 1);
            let del = if r % 3 == 0 { (r / 7) % 3 } else { 0 }.min(len - pos);
            let ins = pieces[(r / 11) % pieces.len()];
            let sels = vec![Selection::caret(pos)];
            d.edit(
                vec![(pos, pos + del, ins.into())],
                &sels,
                EditKind::Other,
                |_| sels.clone(),
            );
            assert_eq!(d.eol_counts, Eol::count(&d.rope.to_string()));
        }
    }

    #[test]
    fn highlighting_off_skips_lexing_and_folds_by_indent() {
        let mut d = doc("int f() {\n    return 1;\n}\n");
        assert!(!d.line_spans(0, "int f() {").is_empty());
        assert_eq!(d.fold_mode(), FoldMode::Braces);
        d.set_highlighting(false);
        assert!(d.line_spans(1, "    return 1;").is_empty());
        assert_eq!(d.fold_mode(), FoldMode::Indent);
        assert!(d.is_fold_header(0));
        assert_eq!(d.fold_range(0), Some(1));
        // Nothing was lexed after highlighting was turned off.
        assert_eq!(d.lex.states.len(), 1);
    }

    #[test]
    fn edit_undo_redo_and_dirty() {
        let mut d = doc("hello\nworld");
        assert!(!d.is_dirty());
        let s = vec![Selection::caret(5)];
        d.edit(vec![(5, 5, "!".into())], &s, EditKind::Typing, |f| {
            vec![Selection::caret(f[0].1)]
        });
        assert_eq!(d.rope.to_string(), "hello!\nworld");
        assert!(d.is_dirty());
        d.undo();
        assert_eq!(d.rope.to_string(), "hello\nworld");
        assert!(!d.is_dirty());
        d.redo();
        assert_eq!(d.rope.to_string(), "hello!\nworld");
    }

    #[test]
    fn multi_edit_positions() {
        let mut d = doc("aa\nbb\ncc");
        let before = vec![
            Selection::caret(0),
            Selection::caret(3),
            Selection::caret(6),
        ];
        let after = d.edit(
            vec![(0, 0, "X".into()), (3, 3, "Y".into()), (6, 6, "Z".into())],
            &before,
            EditKind::Typing,
            |f| f.iter().map(|&(_, e)| Selection::caret(e)).collect(),
        );
        assert_eq!(d.rope.to_string(), "Xaa\nYbb\nZcc");
        assert_eq!(
            after,
            vec![
                Selection::caret(1),
                Selection::caret(5),
                Selection::caret(9)
            ]
        );
        d.undo();
        assert_eq!(d.rope.to_string(), "aa\nbb\ncc");
    }

    #[test]
    fn bookmarks_follow_line_edits() {
        let mut d = doc("a\nb\nc");
        d.toggle_bookmark(2);
        let s = vec![Selection::caret(0)];
        d.edit(vec![(0, 0, "new\n".into())], &s, EditKind::Other, |_| {
            s.clone()
        });
        assert_eq!(d.bookmarked_lines(), vec![3]);
        d.edit(vec![(0, 4, String::new())], &s, EditKind::Other, |_| {
            s.clone()
        });
        assert_eq!(d.bookmarked_lines(), vec![2]);
    }

    #[test]
    fn brace_fold_ranges() {
        let mut d = doc("int f() {\n  a;\n  if (x) {\n    b;\n  }\n}\nint g;");
        assert!(d.is_fold_header(0));
        assert_eq!(d.fold_range(0), Some(5));
        assert_eq!(d.fold_range(2), Some(4));
        assert_eq!(d.fold_range(1), None);
        d.set_fold(0, true);
        assert_eq!(d.hidden_ranges(), vec![(1, 5)]);
        d.ensure_line_visible(3);
        assert!(d.hidden_ranges().is_empty());
    }

    #[test]
    fn indent_fold_ranges() {
        let langs = LangRegistry::load();
        let py = langs.by_name("python").unwrap();
        let mut d =
            Document::from_text(1, "t".into(), "def f():\n    a\n\n    b\nx = 1", &langs, py);
        assert_eq!(d.fold_range(0), Some(3));
        assert_eq!(d.fold_range(4), None);
    }

    #[test]
    fn map_pos_through_changes() {
        let mut d = doc("abcdef");
        let v = d.version;
        let s = vec![Selection::caret(0)];
        d.edit(vec![(1, 1, "XX".into())], &s, EditKind::Other, |_| {
            s.clone()
        });
        assert_eq!(d.map_pos(4, v), 6);
        assert_eq!(d.map_pos(0, v), 0);
    }

    #[test]
    fn normalize_merges_overlaps() {
        let mut v = vec![
            Selection::new(5, 8),
            Selection::new(0, 2),
            Selection::new(6, 10),
        ];
        normalize_selections(&mut v);
        assert_eq!(v, vec![Selection::new(0, 2), Selection::new(5, 10)]);
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    #[test]
    #[ignore = "benchmark: cargo test --release open_bench -- --ignored --nocapture"]
    fn open_bench() {
        let path = std::env::var("BENCH_FILE").expect("BENCH_FILE");
        let langs = LangRegistry::load();
        let p = std::path::Path::new(&path);
        let t = Instant::now();
        let bytes = std::fs::read(p).unwrap();
        let t_read = t.elapsed();
        let t_detect = t.elapsed();
        let (_, text) = TextEncoding::detect_and_decode(bytes);
        let t_decode = t.elapsed();
        let rope = Rope::from_str(&text);
        let t_rope = t.elapsed();
        drop((text, rope));
        let t2 = Instant::now();
        let mut d = Document::open(1, p, &langs, None).unwrap();
        let t_open = t2.elapsed();
        let t3 = Instant::now();
        for l in 0..60 {
            let txt = d.line_text(l);
            let _ = d.line_spans(l, &txt);
        }
        let t_first = t3.elapsed();
        let t4 = Instant::now();
        let last = d.line_count() - 1;
        let txt = d.line_text(last);
        let _ = d.line_spans(last, &txt);
        let t_end = t4.elapsed();
        println!(
            "read {t_read:?} | +detect {:?} | +decode {:?} | +rope {:?} || Document::open {t_open:?} | first screen {t_first:?} | jump to end (lex) {t_end:?}",
            t_detect - t_read,
            t_decode - t_detect,
            t_rope - t_decode
        );
    }
}

#[cfg(test)]
mod startup_bench {
    #[test]
    #[ignore = "benchmark"]
    fn startup_bench() {
        let t = std::time::Instant::now();
        let _l = crate::langs::LangRegistry::load();
        let a = t.elapsed();
        let _t = crate::theme::ThemeRegistry::load();
        let b = t.elapsed();
        let fl = crate::function_list::FunctionLists::new();
        let _ = fl.parse("cpp", "int main() {}");
        let c = t.elapsed();
        let f1 = std::fs::read("/System/Library/Fonts/Menlo.ttc")
            .map(|v| v.len())
            .unwrap_or(0);
        let f2 = std::fs::read("/System/Library/Fonts/Supplemental/Arial Unicode.ttf")
            .map(|v| v.len())
            .unwrap_or(0);
        let d = t.elapsed();
        println!(
            "langs {a:?} | themes {:?} | cpp function parser {:?} | font files {:?} ({} MB)",
            b - a,
            c - b,
            d - c,
            (f1 + f2) / 1_000_000
        );
    }
}
