//! File compare session: wiring the diff engine into panes and commands.

use std::sync::Arc;

use crate::app::App;
use crate::compare::{self, Hunk};
use crate::document::{DocId, Document, EditKind};

pub struct CompareSession {
    pub left: DocId,
    pub right: DocId,
    /// Read-only snapshot documents (clipboard, saved version, selections)
    /// to close afterwards.
    pub snapshots: Vec<DocId>,
    pub hunks: Vec<Hunk>,
    pub versions: (u64, u64),
    pub stale_since: Option<f64>,
    /// (added, removed, changed, moved) lines.
    pub stats: (usize, usize, usize, usize),
    /// Side panels hidden for the compare, shown again when it ends.
    pub hidden_panels: Option<[bool; 7]>,
}

impl App {
    /// Compares the two views, or the active tab with the next one.
    pub fn compare_default(&mut self) {
        if let (Some(a), Some(b)) = (self.panes[0].active_doc(), self.panes[1].active_doc())
            && a != b
        {
            self.start_compare(a, b, Vec::new());
            return;
        }
        let pane = self.focused;
        let p = &self.panes[pane];
        if p.tabs.len() < 2 {
            self.message("Compare", "Open a second document to compare with, or use Compare > Compare with Clipboard / Last Save.");
            return;
        }
        let a = p.tabs[p.active];
        let b = p.tabs[(p.active + 1) % p.tabs.len()];
        self.start_compare(a, b, Vec::new());
    }

    /// Compares the selected lines of the two views (each selection is
    /// extended to whole lines), shown as read-only copies.
    pub fn compare_selections(&mut self) {
        let mut parts = Vec::new();
        for pane in 0..2 {
            let Some(id) = self.panes[pane].active_doc() else {
                break;
            };
            let (Some(d), Some(v)) = (self.docs.get(&id), self.views.get(&(pane, id))) else {
                break;
            };
            let s = v.primary();
            if s.is_empty() {
                break;
            }
            let (a, b) = (
                d.char_to_line(s.start()),
                d.char_to_line(s.end().saturating_sub(1).max(s.start())),
            );
            let text: Vec<String> = (a..=b).map(|l| d.line_text(l)).collect();
            parts.push((
                format!("{} ({}-{})", d.display_name(), a + 1, b + 1),
                text.join("\n"),
                d.lang,
            ));
        }
        if parts.len() < 2 {
            self.message(
                "Compare",
                "Select some lines in both views first (use Move to Other View to show two documents).",
            );
            return;
        }
        let mut ids = Vec::new();
        for (name, text, lang) in parts {
            let id = self.alloc_id();
            let mut d = Document::from_text(id, name, &text, &self.langs, lang);
            d.read_only = true;
            d.transient = true;
            d.transient = true;
            self.docs.insert(id, d);
            ids.push(id);
        }
        self.start_compare(ids[0], ids[1], ids.clone());
    }

    /// Opens two empty documents, "Original" and "Changed", side by side to
    /// paste texts into; they're compared as you type.
    pub fn compare_blank(&mut self) {
        let mut ids = Vec::new();
        for (pane, name) in [(0, "Original"), (1, "Changed")] {
            let id = self.new_document(pane);
            if let Some(d) = self.docs.get_mut(&id) {
                d.untitled_name = name.into();
            }
            ids.push(id);
        }
        self.start_compare(ids[0], ids[1], Vec::new());
    }

    /// Shows the differences in one read-only document: unchanged lines once,
    /// removed lines (red) followed by the lines that replaced them (green).
    pub fn compare_inline(&mut self) {
        let pair = match &self.compare {
            Some(s) => Some((s.left, s.right)),
            None => match (self.panes[0].active_doc(), self.panes[1].active_doc()) {
                (Some(a), Some(b)) if a != b => Some((a, b)),
                _ => None,
            },
        };
        let Some((l, r)) = pair else {
            self.message(
                "Compare",
                "Show the two documents in the two views first, or start a compare.",
            );
            return;
        };
        let (Some(dl), Some(dr)) = (self.docs.get(&l), self.docs.get(&r)) else {
            return;
        };
        let lines = |d: &Document| {
            (0..d.line_count())
                .map(|i| d.line_text(i))
                .collect::<Vec<_>>()
        };
        let (left, right) = (lines(dl), lines(dr));
        let title = format!("Diff: {} ↔ {}", dl.display_name(), dr.display_name());
        let lang = dl.lang;
        let res = match compare::compare(&left, &right, 0, 0, &self.settings.compare) {
            Ok(r) => r,
            Err(e) => {
                self.message("Compare", &e);
                return;
            }
        };
        use compare::LineKind;
        let mut text: Vec<&str> = Vec::new();
        let mut kinds = Vec::new();
        let mut inline = std::collections::HashMap::new();
        let mut li = 0;
        for h in &res.hunks {
            for line in &left[li..h.left.start] {
                text.push(line);
                kinds.push(LineKind::Same);
            }
            for i in h.left.clone() {
                if let Some(rg) = res.left.inline.get(&i) {
                    inline.insert(text.len(), rg.clone());
                }
                text.push(&left[i]);
                kinds.push(match res.left.kinds[i] {
                    LineKind::Moved => LineKind::Moved,
                    _ => LineKind::Removed,
                });
            }
            for i in h.right.clone() {
                if let Some(rg) = res.right.inline.get(&i) {
                    inline.insert(text.len(), rg.clone());
                }
                text.push(&right[i]);
                kinds.push(match res.right.kinds[i] {
                    LineKind::Moved => LineKind::Moved,
                    _ => LineKind::Added,
                });
            }
            li = h.left.end;
        }
        for line in &left[li..] {
            text.push(line);
            kinds.push(LineKind::Same);
        }
        let id = self.alloc_id();
        let mut d = Document::from_text(id, title, &text.join("\n"), &self.langs, lang);
        d.read_only = true;
        d.transient = true;
        let decor = compare::SideDecor {
            generation: compare::next_generation(),
            version: d.version,
            runs: compare::runs_of(&kinds),
            spacers_before: vec![0; kinds.len() + 1],
            kinds,
            inline,
        };
        self.docs.insert(id, d);
        self.clear_compare();
        let pane = self.focused;
        self.open_in_pane(pane, id);
        self.activate(pane, id);
        self.views.entry((pane, id)).or_default().compare = Some(Arc::new(decor));
    }

    /// Swaps which document is on the left.
    pub fn compare_swap(&mut self) {
        let Some(s) = self.compare.take() else {
            self.flash("No compare is active");
            return;
        };
        if let Some(h) = s.hidden_panels {
            self.set_panels(h);
        }
        let (l, r, snaps) = (s.left, s.right, s.snapshots);
        for (pane, id) in [(0, l), (1, r)] {
            if let Some(v) = self.views.get_mut(&(pane, id)) {
                v.compare = None;
            }
            self.close_in_pane(pane, id);
        }
        self.start_compare(r, l, snaps);
    }

    /// Makes one side identical to the other, as one undo step.
    pub fn compare_copy_all(&mut self, to_right: bool) {
        let Some(s) = &self.compare else {
            self.flash("No compare is active");
            return;
        };
        let (src, dst, pane) = if to_right {
            (s.left, s.right, 1)
        } else {
            (s.right, s.left, 0)
        };
        let Some(text) = self.docs.get(&src).map(|d| d.rope.to_string()) else {
            return;
        };
        let Some(d) = self.docs.get_mut(&dst) else {
            return;
        };
        if d.read_only {
            self.message("Compare", "The target document is read-only.");
            return;
        }
        let vs = self.views.entry((pane, dst)).or_default();
        vs.sync(d);
        let keep = vs.sels.clone();
        let len = d.len_chars();
        d.edit(vec![(0, len, text)], &keep, EditKind::Other, |_| {
            vec![crate::document::Selection::caret(0)]
        });
        vs.after_edit(d);
        self.recompute_compare();
    }

    /// Visibility of the side panels, in a fixed order.
    fn panels(&self) -> [bool; 7] {
        let s = &self.settings;
        [
            s.show_doc_map,
            s.show_doc_list,
            s.show_function_list,
            s.show_folder_tree,
            s.show_json_tree,
            s.show_char_panel,
            s.show_clipboard_history,
        ]
    }

    fn set_panels(&mut self, p: [bool; 7]) {
        let s = &mut self.settings;
        [
            s.show_doc_map,
            s.show_doc_list,
            s.show_function_list,
            s.show_folder_tree,
            s.show_json_tree,
            s.show_char_panel,
            s.show_clipboard_history,
        ] = p;
    }

    pub fn compare_with_clipboard(&mut self) {
        let text = arboard::Clipboard::new().and_then(|mut c| c.get_text());
        let Ok(text) = text else {
            self.message("Compare", "The clipboard doesn't contain text.");
            return;
        };
        let Some(left) = self.active_id() else { return };
        self.compare_with_snapshot(left, "Clipboard".into(), &text);
    }

    pub fn compare_with_saved(&mut self) {
        let Some(left) = self.active_id() else { return };
        let Some(doc) = self.docs.get(&left) else {
            return;
        };
        let Some(path) = doc.path.clone() else {
            self.message("Compare", "This document has never been saved.");
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                let text = doc.encoding.decode(&bytes);
                let name = format!("{} (on disk)", doc.display_name());
                self.compare_with_snapshot(left, name, &text);
            }
            Err(e) => self.message(
                "Compare",
                &format!("Cannot read \"{}\": {e}", path.display()),
            ),
        }
    }

    fn compare_with_snapshot(&mut self, left: DocId, name: String, text: &str) {
        let lang = self
            .docs
            .get(&left)
            .map_or_else(|| self.langs.normal(), |d| d.lang);
        let id = self.alloc_id();
        let mut d = Document::from_text(id, name, text, &self.langs, lang);
        d.read_only = true;
        d.transient = true;
        self.docs.insert(id, d);
        self.start_compare(left, id, vec![id]);
    }

    pub fn start_compare(&mut self, left: DocId, right: DocId, snapshots: Vec<DocId>) {
        if left == right {
            self.message("Compare", "A document can't be compared with itself.");
            return;
        }
        self.clear_compare();
        // Left document in view 1, right document in view 2.
        if !self.panes[0].tabs.contains(&left) {
            self.open_in_pane(0, left);
            if self.panes[1].tabs.contains(&left) {
                self.close_in_pane(1, left);
            }
        }
        if !self.panes[1].tabs.contains(&right) {
            self.open_in_pane(1, right);
            if self.panes[0].tabs.contains(&right) {
                self.close_in_pane(0, right);
            }
        }
        self.activate(1, right);
        self.activate(0, left);
        for id in [left, right] {
            if let Some(d) = self.docs.get_mut(&id) {
                d.fold_all(false, None);
            }
        }
        let hidden_panels = if self.settings.compare.hide_panels {
            let before = self.panels();
            self.set_panels([false; 7]);
            Some(before)
        } else {
            None
        };
        self.compare = Some(CompareSession {
            left,
            right,
            snapshots,
            hunks: Vec::new(),
            versions: (u64::MAX, u64::MAX),
            stale_since: None,
            stats: (0, 0, 0, 0),
            hidden_panels,
        });
        self.recompute_compare();
        if let Some(v) = self.views.get_mut(&(0, left)) {
            v.set_single(0);
            v.top_row = 0.0;
        }
        if let Some(v) = self.views.get_mut(&(1, right)) {
            v.set_single(0);
            v.top_row = 0.0;
        }
    }

    pub fn recompute_compare(&mut self) {
        let Some(s) = &self.compare else { return };
        let (l, r) = (s.left, s.right);
        let (Some(dl), Some(dr)) = (self.docs.get(&l), self.docs.get(&r)) else {
            self.clear_compare();
            return;
        };
        let lines = |d: &Document| {
            (0..d.line_count())
                .map(|i| d.line_text(i))
                .collect::<Vec<_>>()
        };
        let (vl, vr) = (dl.version, dr.version);
        let res = compare::compare(&lines(dl), &lines(dr), vl, vr, &self.settings.compare);
        match res {
            Ok(res) => {
                self.views.entry((0, l)).or_default().compare = Some(Arc::new(res.left));
                self.views.entry((1, r)).or_default().compare = Some(Arc::new(res.right));
                let identical = res.hunks.is_empty();
                if let Some(s) = &mut self.compare {
                    s.hunks = res.hunks;
                    s.versions = (vl, vr);
                    s.stale_since = None;
                    s.stats = (res.added, res.removed, res.changed, res.moved);
                }
                if identical {
                    self.flash("Compare: the files match");
                }
            }
            Err(e) => {
                self.clear_compare();
                self.message("Compare", &e);
            }
        }
    }

    pub fn clear_compare(&mut self) {
        let Some(s) = self.compare.take() else { return };
        for v in self.views.values_mut() {
            v.compare = None;
        }
        if let Some(p) = s.hidden_panels {
            self.set_panels(p);
        }
        for snap in s.snapshots {
            for pane in 0..2 {
                if self.panes[pane].tabs.contains(&snap) {
                    self.close_in_pane(pane, snap);
                }
            }
            self.docs.remove(&snap);
        }
    }

    /// Per frame: end the compare if a side was closed; re-compare after edits.
    pub fn poll_compare(&mut self, now: f64) {
        let Some(s) = &self.compare else { return };
        let placed = self.panes[0].active_doc() == Some(s.left)
            && self.panes[1].active_doc() == Some(s.right);
        let exists = self.docs.contains_key(&s.left) && self.docs.contains_key(&s.right);
        let still_open =
            self.panes[0].tabs.contains(&s.left) && self.panes[1].tabs.contains(&s.right);
        if !exists || !still_open {
            self.clear_compare();
            return;
        }
        if !placed {
            return; // the user switched tabs; keep the session for when they come back
        }
        let cur = (self.docs[&s.left].version, self.docs[&s.right].version);
        if cur != s.versions && self.settings.compare.auto {
            let since = *self
                .compare
                .as_mut()
                .unwrap()
                .stale_since
                .get_or_insert(now);
            if now - since > 0.3 {
                self.recompute_compare();
            } else if let Some(c) = &self.egui_ctx {
                c.request_repaint_after(std::time::Duration::from_millis(320));
            }
        }
    }

    /// Keeps both compared views scrolled to the same row.
    pub fn sync_compare_scroll(&mut self) {
        let Some(s) = &self.compare else { return };
        let keys = [(0, s.left), (1, s.right)];
        let src = keys[self.focused.min(1)];
        let dst = keys[1 - self.focused.min(1)];
        if let Some((top, x)) = self.views.get(&src).map(|v| (v.top_row, v.scroll_x))
            && let Some(d) = self.views.get_mut(&dst)
        {
            d.top_row = top;
            d.scroll_x = x;
        }
    }

    fn side_ranges(&self, side: usize) -> Vec<std::ops::Range<usize>> {
        self.compare
            .as_ref()
            .map(|s| {
                s.hunks
                    .iter()
                    .map(|h| {
                        if side == 0 {
                            h.left.clone()
                        } else {
                            h.right.clone()
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Next / previous difference.
    pub fn compare_nav(&mut self, forward: bool) {
        let Some(s) = &self.compare else {
            self.flash("No compare is active");
            return;
        };
        if s.hunks.is_empty() {
            self.flash("Compare: no differences");
            return;
        }
        let side = self.focused.min(1);
        let id = if side == 0 { s.left } else { s.right };
        let line = self
            .views
            .get(&(side, id))
            .and_then(|v| self.docs.get(&id).map(|d| d.char_to_line(v.primary().head)))
            .unwrap_or(0);
        let starts: Vec<usize> = self.side_ranges(side).iter().map(|r| r.start).collect();
        let idx = if forward {
            starts.iter().position(|&st| st > line).unwrap_or(0)
        } else {
            starts
                .iter()
                .rposition(|&st| st < line)
                .unwrap_or(starts.len() - 1)
        };
        let hunk = s.hunks[idx].clone();
        for (pane, doc, r) in [(0, s.left, hunk.left), (1, s.right, hunk.right)] {
            if let (Some(d), Some(v)) = (self.docs.get(&doc), self.views.get_mut(&(pane, doc))) {
                let l = r.start.min(d.line_count().saturating_sub(1));
                v.set_single(d.line_start(l));
                v.center_caret = true;
            }
        }
        self.flash(format!("Difference {} of {}", idx + 1, starts.len()));
    }

    /// Copies the difference under the caret to the other side.
    pub fn compare_copy(&mut self, to_right: bool) {
        let Some(s) = &self.compare else {
            self.flash("No compare is active");
            return;
        };
        let side = self.focused.min(1);
        let id = if side == 0 { s.left } else { s.right };
        let Some(line) = self
            .views
            .get(&(side, id))
            .and_then(|v| self.docs.get(&id).map(|d| d.char_to_line(v.primary().head)))
        else {
            return;
        };
        let hunk = s.hunks.iter().find(|h| {
            let r = if side == 0 { &h.left } else { &h.right };
            r.contains(&line) || (r.is_empty() && r.start == line)
        });
        let Some(h) = hunk.cloned() else {
            self.flash("Place the caret on a difference first");
            return;
        };
        let (src, src_r, dst, dst_r, dst_pane) = if to_right {
            (s.left, h.left, s.right, h.right, 1)
        } else {
            (s.right, h.right, s.left, h.left, 0)
        };
        let src_lines: Vec<String> = match self.docs.get(&src) {
            Some(d) => src_r.clone().map(|l| d.line_text(l)).collect(),
            None => return,
        };
        let Some(d) = self.docs.get_mut(&dst) else {
            return;
        };
        if d.read_only {
            self.message("Compare", "The target document is read-only.");
            return;
        }
        let eol = d.eol.as_str().to_string();
        let n = d.line_count();
        let len = d.len_chars();
        let (mut start, end) = (
            if dst_r.start < n {
                d.line_start(dst_r.start)
            } else {
                len
            },
            if dst_r.end < n {
                d.line_start(dst_r.end)
            } else {
                len
            },
        );
        let mut ins: String = src_lines.iter().map(|l| format!("{l}{eol}")).collect();
        if dst_r.start >= n {
            // Appending after the last line, which has no line ending.
            ins = if src_lines.is_empty() {
                String::new()
            } else {
                format!("{eol}{}", src_lines.join(&eol))
            };
        } else if dst_r.end >= n {
            ins.truncate(ins.len() - if src_lines.is_empty() { 0 } else { eol.len() });
            if src_lines.is_empty() && dst_r.start > 0 {
                start = d.line_end(dst_r.start - 1);
            }
        }
        let vs = self.views.entry((dst_pane, dst)).or_default();
        vs.sync(d);
        let before = vs.sels.clone();
        let keep = vs.sels.clone();
        d.edit(vec![(start, end, ins)], &before, EditKind::Other, |_| {
            keep.clone()
        });
        vs.after_edit(d);
        vs.scroll_to_caret = false;
        self.recompute_compare();
    }

    pub fn compare_status(&self) -> Option<String> {
        let s = self.compare.as_ref()?;
        let (a, r, c, m) = s.stats;
        let stale = self.docs.get(&s.left).map(|d| d.version) != Some(s.versions.0)
            || self.docs.get(&s.right).map(|d| d.version) != Some(s.versions.1);
        Some(if s.hunks.is_empty() && !stale {
            "Compare: files match".into()
        } else {
            format!(
                "Compare: {} difference{}  (+{a} −{r} ~{c}{}){}",
                s.hunks.len(),
                if s.hunks.len() == 1 { "" } else { "s" },
                if m > 0 {
                    format!(" ↕{m}")
                } else {
                    String::new()
                },
                if stale { "  · outdated" } else { "" }
            )
        })
    }

    fn open_in_pane(&mut self, pane: usize, id: DocId) {
        let p = &mut self.panes[pane];
        p.tabs.push(id);
        p.active = p.tabs.len() - 1;
        self.views.entry((pane, id)).or_default();
    }

    fn close_in_pane(&mut self, pane: usize, id: DocId) {
        let p = &mut self.panes[pane];
        if let Some(i) = p.tabs.iter().position(|&t| t == id) {
            p.tabs.remove(i);
            if p.active >= p.tabs.len() {
                p.active = p.tabs.len().saturating_sub(1);
            }
        }
        self.views.remove(&(pane, id));
    }
}
