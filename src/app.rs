//! Application state and command dispatch.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, channel};

use crate::commands::{
    Command, FormatOp, HashSource, MimeOp, Shortcut, TabSort, default_shortcuts,
};
use crate::config::{self, Persisted, Session, SessionFile, Settings};
use crate::document::{DocId, Document, EditKind, Mark, Selection};
use crate::editor::{ClipState, ViewState};
use crate::encoding::{Eol, TextEncoding};
use crate::function_list::{FoundItem, FunctionLists};
use crate::langs::LangRegistry;
use crate::ops;
use crate::search::{self, FifMessage, SearchResults};
use crate::textops::{self, CaseMode, HashAlgo, SortMode};
use crate::theme::{Globals, LangStyles, ThemeRegistry};

#[derive(Default)]
pub struct Pane {
    pub tabs: Vec<DocId>,
    pub active: usize,
}

impl Pane {
    pub fn active_doc(&self) -> Option<DocId> {
        self.tabs.get(self.active).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FindTab {
    #[default]
    Find,
    Replace,
    FindInFiles,
    Mark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    NotFound,
    Wrapped,
}

pub struct FindState {
    pub open: bool,
    pub tab: FindTab,
    pub what: String,
    pub with: String,
    pub backward: bool,
    pub in_selection: bool,
    pub fif_dir: String,
    pub fif_filters: String,
    pub fif_recursive: bool,
    pub fif_hidden: bool,
    pub fif_follow_current: bool,
    pub mark_bookmark: bool,
    pub mark_purge: bool,
    pub status: Option<(String, StatusKind)>,
    pub focus_what: bool,
}

impl Default for FindState {
    fn default() -> Self {
        Self {
            open: false,
            tab: FindTab::Find,
            what: String::new(),
            with: String::new(),
            backward: false,
            in_selection: false,
            fif_dir: String::new(),
            fif_filters: "*.*".into(),
            fif_recursive: true,
            fif_hidden: false,
            fif_follow_current: false,
            mark_bookmark: false,
            mark_purge: true,
            status: None,
            focus_what: false,
        }
    }
}

pub struct FifRunning {
    pub rx: Receiver<FifMessage>,
    pub cancel: Arc<AtomicBool>,
    pub progress: usize,
    pub replace: bool,
    pub replaced: usize,
}

#[derive(Default)]
pub struct IncrementalState {
    pub open: bool,
    pub text: String,
    pub match_case: bool,
    pub highlight_all: bool,
    pub not_found: bool,
    pub focus: bool,
    pub origin: Option<usize>,
}

/// A queued confirmation that needs the user's answer.
pub enum Prompt {
    SaveBeforeClose { doc: DocId, then_exit: bool },
    Reload(DocId),
    Deleted(DocId),
    Message { title: String, text: String },
}

/// Documents larger than this (chars) are backed up at most every
/// `BIG_BACKUP_INTERVAL` while editing.
const BIG_BACKUP_CHARS: usize = 4 * 1024 * 1024;
const BIG_BACKUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Writes `rope` to `path` chunk by chunk (no full-text copy) through a
/// temporary file, so a crash mid-write never leaves a half-written backup.
fn write_backup_file(path: &Path, rope: &ropey::Rope) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    {
        let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        for chunk in rope.chunks() {
            w.write_all(chunk.as_bytes())?;
        }
        w.flush()?;
    }
    std::fs::rename(&tmp, path)
}

/// Font sizes the editor allows (points).
pub const MIN_FONT_SIZE: u8 = 6;
pub const MAX_FONT_SIZE: u8 = 48;
/// Default editor font size (points).
pub const DEFAULT_FONT_SIZE: u8 = 13;

#[derive(Default)]
pub struct Dialogs {
    pub goto: Option<(bool, String)>,
    pub about: bool,
    pub preferences: bool,
    pub style_config: bool,
    pub shortcut_mapper: bool,
    pub column_editor: Option<crate::dialogs::ColumnEditorState>,
    pub summary: bool,
    pub run: Option<String>,
    pub hash: Option<crate::dialogs::HashDialog>,
    pub windows: bool,
    pub windows_selected: Option<DocId>,
    pub char_range: Option<crate::dialogs::CharRangeDialog>,
    pub rename: Option<(DocId, String)>,
    pub command_line: bool,
    pub debug_info: bool,
    pub pref_tab: usize,
    pub datetime: Option<crate::dialogs::DateTimeDialog>,
}

pub struct App {
    /// Frame timing, printed to stderr when `SUPERFASTNOTEPAD_PROFILE` is set.
    pub profile: Option<FrameProfile>,
    /// Last title sent to the window.
    pub window_title: String,
    /// The OS is in dark mode (as last reported).
    pub system_dark: bool,
    /// Last normal (not maximized or full screen) window geometry.
    pub window_geometry: Option<config::WindowGeometry>,
    /// Unsaved-content backups on disk: document version, when, and where.
    pub backups_written: HashMap<DocId, (u64, std::time::Instant, PathBuf)>,
    /// When the last periodic backup ran, and a fingerprint of what it saved.
    pub last_autosave: f64,
    pub autosave_sig: u64,
    pub langs: LangRegistry,
    pub themes: ThemeRegistry,
    pub theme_idx: usize,
    pub globals: Globals,
    styles_cache: HashMap<usize, LangStyles>,
    keyword_cache: HashMap<usize, Vec<String>>,
    pub docs: HashMap<DocId, Document>,
    next_id: DocId,
    untitled_counter: usize,
    pub panes: [Pane; 2],
    pub focused: usize,
    pub split_ratio: f32,
    pub views: HashMap<(usize, DocId), ViewState>,
    pub settings: Settings,
    pub persisted: Persisted,
    pub shortcuts: Vec<(Command, Shortcut)>,
    pub clip: ClipState,
    pub overtype: bool,
    closed_stack: Vec<String>,
    pub find: FindState,
    pub results: Option<SearchResults>,
    pub results_visible: bool,
    pub results_cursor: Option<(usize, usize)>,
    pub fif: Option<FifRunning>,
    pub dialogs: Dialogs,
    pub prompts: Vec<Prompt>,
    pub function_lists: Arc<FunctionLists>,
    pub func_cache: Option<(DocId, u64, Vec<FoundItem>)>,
    /// Function List parse running on a background thread: (doc, version, result).
    pub func_job: Option<(DocId, u64, std::sync::mpsc::Receiver<Vec<FoundItem>>)>,
    /// Document version the Function List last saw, and when it changed.
    pub func_seen: (DocId, u64, f64),
    /// JSON tree panel (parsed only while the panel is open).
    pub json_tree: crate::panels::JsonTreeState,
    /// Classes collapsed in the Function List.
    pub func_collapsed: HashSet<String>,
    pub func_filter: String,
    pub incremental: IncrementalState,
    /// Line filter bar (View > Filter Lines).
    pub filter: crate::filter::FilterState,
    pub status_msg: Option<(String, f64)>,
    pub always_on_top: bool,
    pub fullscreen: bool,
    pub postit: bool,
    pub ipc_rx: Option<Receiver<Vec<String>>>,
    last_monitor_check: f64,
    prompted_mtime: HashSet<DocId>,
    pub exit_requested: bool,
    pub allow_close: bool,
    pub tree_expanded: HashSet<PathBuf>,
    pub pending_goto: Option<(DocId, usize, usize)>,
    /// Contents of the toolbar's "Go to line" box.
    pub goto_line_text: String,
    pub egui_ctx: Option<egui::Context>,
    pub request_focus_editor: bool,
    pub tab_drag: Option<(usize, usize)>,
    pub no_session: bool,
    pub compare: Option<crate::compare_ui::CompareSession>,
}

pub struct StartupArgs {
    pub files: Vec<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub lang: Option<String>,
    pub no_session: bool,
    pub read_only: bool,
    pub always_on_top: bool,
}

/// Collects how long building each frame takes and prints a summary every
/// two seconds: frame count, average, 95th percentile and slowest frame.
#[derive(Default)]
pub struct FrameProfile {
    since: Option<std::time::Instant>,
    times: Vec<std::time::Duration>,
    causes: HashMap<String, usize>,
}

impl FrameProfile {
    pub fn record(&mut self, t: std::time::Duration, causes: &[egui::RepaintCause]) {
        let since = *self.since.get_or_insert_with(std::time::Instant::now);
        self.times.push(t);
        for c in causes {
            *self.causes.entry(format!("{c:?}")).or_default() += 1;
        }
        if since.elapsed().as_secs_f32() >= 2.0 {
            self.times.sort_unstable();
            let n = self.times.len();
            let avg = self.times.iter().sum::<std::time::Duration>() / n as u32;
            eprintln!(
                "[profile] {n} frames | avg {avg:.2?} | p95 {:.2?} | max {:.2?}",
                self.times[n * 95 / 100],
                self.times[n - 1]
            );
            let mut causes: Vec<_> = self.causes.drain().collect();
            causes.sort_by_key(|c| std::cmp::Reverse(c.1));
            for (c, k) in causes.iter().take(3) {
                eprintln!("[profile]   repaint requested {k}x by {c}");
            }
            self.times.clear();
            self.since = None;
        }
    }
}

impl App {
    pub fn new(args: StartupArgs) -> Self {
        let langs = LangRegistry::load();
        let themes = ThemeRegistry::load();
        let settings = config::load_settings();
        let theme_idx = themes.index_of(&settings.theme);
        let globals = themes.themes[theme_idx].globals();
        let mut app = Self {
            profile: std::env::var_os("SUPERFASTNOTEPAD_PROFILE").map(|_| FrameProfile::default()),
            window_title: String::new(),
            system_dark: false,
            window_geometry: None,
            backups_written: HashMap::new(),
            last_autosave: 0.0,
            autosave_sig: 0,
            langs,
            themes,
            theme_idx,
            globals,
            styles_cache: HashMap::new(),
            keyword_cache: HashMap::new(),
            docs: HashMap::new(),
            next_id: 1,
            untitled_counter: 0,
            panes: [Pane::default(), Pane::default()],
            focused: 0,
            split_ratio: 0.5,
            views: HashMap::new(),
            persisted: config::load_persisted(),
            shortcuts: default_shortcuts(),
            clip: ClipState::default(),
            overtype: false,
            closed_stack: Vec::new(),
            find: FindState::default(),
            results: None,
            results_visible: false,
            results_cursor: None,
            fif: None,
            dialogs: Dialogs::default(),
            prompts: Vec::new(),
            function_lists: Arc::new(FunctionLists::new()),
            func_cache: None,
            func_job: None,
            func_seen: (0, 0, 0.0),
            func_collapsed: HashSet::new(),
            json_tree: crate::panels::JsonTreeState::default(),
            filter: crate::filter::FilterState::default(),
            func_filter: String::new(),
            incremental: IncrementalState::default(),
            status_msg: None,
            always_on_top: args.always_on_top,
            fullscreen: false,
            postit: false,
            ipc_rx: None,
            last_monitor_check: 0.0,
            prompted_mtime: HashSet::new(),
            exit_requested: false,
            allow_close: false,
            tree_expanded: HashSet::new(),
            pending_goto: None,
            goto_line_text: String::new(),
            egui_ctx: None,
            request_focus_editor: true,
            tab_drag: None,
            no_session: args.no_session,
            compare: None,
            settings,
        };
        app.find.fif_filters = app
            .settings
            .fif_filters
            .first()
            .cloned()
            .unwrap_or_else(|| "*.*".into());
        app.find.fif_dir = app.settings.fif_dirs.first().cloned().unwrap_or_default();
        if app.settings.remember_session && !args.no_session {
            app.restore_session();
        }
        let mut opened = None;
        for f in &args.files {
            let p = PathBuf::from(f);
            if p.is_dir() {
                app.add_workspace_folder(p);
                continue;
            }
            if !p.exists() {
                // Missing files open as an empty buffer bound to the path.
                let id = app.new_document(0);
                if let Some(d) = app.docs.get_mut(&id) {
                    d.path = Some(p.clone());
                    d.untitled_name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let lang = app.langs.detect(&d.untitled_name, "");
                    d.set_language(lang, &app.langs);
                }
                opened = Some(id);
                continue;
            }
            opened = app.open_path(&p, app.focused).or(opened);
        }
        if let Some(id) = opened {
            if let Some(lang) = &args.lang
                && let Some(l) = app.langs.by_name(lang)
                && let Some(d) = app.docs.get_mut(&id)
            {
                d.set_language(l, &app.langs);
            }
            if args.read_only
                && let Some(d) = app.docs.get_mut(&id)
            {
                d.read_only = true;
            }
            if let Some(line) = args.line {
                app.pending_goto = Some((
                    id,
                    line.saturating_sub(1),
                    args.column.unwrap_or(1).saturating_sub(1),
                ));
            }
        }
        if app.panes[0].tabs.is_empty() {
            app.new_document(0);
        }
        app
    }

    // ---------------------------------------------------------------- styles

    /// Chooses a theme in the settings (the dark one when following a dark
    /// system) and shows it.
    pub fn set_theme(&mut self, idx: usize) {
        let idx = idx.min(self.themes.themes.len() - 1);
        let name = self.themes.themes[idx].name.clone();
        if self.settings.follow_system_theme && self.system_dark {
            self.settings.dark_theme = name;
        } else {
            self.settings.theme = name;
        }
        self.show_theme(idx);
    }

    /// Shows a theme without changing the settings.
    pub fn show_theme(&mut self, idx: usize) {
        self.theme_idx = idx.min(self.themes.themes.len() - 1);
        self.globals = self.themes.themes[self.theme_idx].globals();
        self.styles_cache.clear();
    }

    /// Follows the OS light/dark setting when that option is on.
    pub fn follow_system_theme(&mut self, dark: bool) {
        self.system_dark = dark;
        if !self.settings.follow_system_theme {
            return;
        }
        let want = if dark {
            &self.settings.dark_theme
        } else {
            &self.settings.theme
        };
        let idx = self.themes.index_of(want);
        if idx != self.theme_idx {
            self.show_theme(idx);
        }
    }

    pub fn styles_for(&mut self, lang: usize) -> &LangStyles {
        let theme = &self.themes.themes[self.theme_idx];
        let langs = &self.langs;
        self.styles_cache
            .entry(lang)
            .or_insert_with(|| theme.resolve(&langs.langs[lang]))
    }

    pub fn keywords_for(&mut self, lang: usize) -> Vec<String> {
        let langs = &self.langs;
        self.keyword_cache
            .entry(lang)
            .or_insert_with(|| langs.langs[lang].all_keywords())
            .clone()
    }

    // ---------------------------------------------------------------- documents

    pub(crate) fn alloc_id(&mut self) -> DocId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn default_encoding(&self) -> TextEncoding {
        TextEncoding::from_key(&self.settings.default_encoding).unwrap_or(TextEncoding::Utf8)
    }

    pub fn new_document(&mut self, pane: usize) -> DocId {
        let id = self.alloc_id();
        // Reuse the lowest free "new N" number.
        let used: HashSet<String> = self
            .docs
            .values()
            .filter(|d| d.path.is_none())
            .map(|d| d.untitled_name.clone())
            .collect();
        let mut n = 1;
        while used.contains(&format!("new {n}")) {
            n += 1;
        }
        self.untitled_counter = self.untitled_counter.max(n);
        let lang = self
            .langs
            .by_name(&self.settings.default_lang)
            .unwrap_or_else(|| self.langs.normal());
        let doc = Document::new(
            id,
            format!("new {n}"),
            &self.langs,
            lang,
            self.settings.default_eol,
            self.default_encoding(),
        );
        self.docs.insert(id, doc);
        self.add_tab(pane, id);
        id
    }

    fn add_tab(&mut self, pane: usize, id: DocId) {
        let p = &mut self.panes[pane];
        let at = if p.tabs.is_empty() { 0 } else { p.active + 1 };
        p.tabs.insert(at.min(p.tabs.len()), id);
        p.active = at.min(p.tabs.len() - 1);
        self.focused = pane;
        self.views.entry((pane, id)).or_default();
        self.request_focus_editor = true;
    }

    pub fn find_open(&self, path: &Path) -> Option<(usize, DocId)> {
        let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        for (pi, pane) in self.panes.iter().enumerate() {
            for &id in &pane.tabs {
                if let Some(p) = self.docs.get(&id).and_then(|d| d.path.as_ref())
                    && (std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()) == canon)
                {
                    return Some((pi, id));
                }
            }
        }
        None
    }

    pub fn activate(&mut self, pane: usize, id: DocId) {
        if let Some(i) = self.panes[pane].tabs.iter().position(|&t| t == id) {
            self.panes[pane].active = i;
            self.focused = pane;
            self.views.entry((pane, id)).or_default();
            self.request_focus_editor = true;
        }
    }

    pub fn open_path(&mut self, path: &Path, pane: usize) -> Option<DocId> {
        if let Some((p, id)) = self.find_open(path) {
            self.activate(p, id);
            return Some(id);
        }
        let id = self.alloc_id();
        match Document::open(id, path, &self.langs, None) {
            Ok(doc) => {
                // Replace a pristine "new 1" tab.
                let replace = self.panes[pane].active_doc().filter(|a| {
                    self.panes[pane].tabs.len() == 1
                        && self.docs.get(a).is_some_and(Document::is_empty_untitled)
                });
                self.docs.insert(id, doc);
                self.add_tab(pane, id);
                if let Some(old) = replace {
                    self.remove_tab(pane, old);
                }
                self.add_recent(path);
                Some(id)
            }
            Err(e) => {
                self.prompts.push(Prompt::Message {
                    title: "Open".into(),
                    text: format!("Cannot open file \"{}\":\n{e}", path.display()),
                });
                None
            }
        }
    }

    pub fn add_recent(&mut self, path: &Path) {
        let s = path.display().to_string();
        self.persisted.recent_files.retain(|r| *r != s);
        self.persisted.recent_files.insert(0, s);
        self.persisted
            .recent_files
            .truncate(self.settings.recent_max.max(1));
    }

    pub fn add_workspace_folder(&mut self, p: PathBuf) {
        let s = p.display().to_string();
        if !self.settings.workspace_folders.contains(&s) {
            self.settings.workspace_folders.push(s);
        }
        self.tree_expanded.insert(p);
        self.settings.show_folder_tree = true;
    }

    pub fn active_id(&self) -> Option<DocId> {
        self.panes[self.focused].active_doc()
    }

    pub fn active_doc(&self) -> Option<&Document> {
        self.active_id().and_then(|id| self.docs.get(&id))
    }

    /// Runs `f` with the active document and its view state.
    pub fn with_active<R>(
        &mut self,
        f: impl FnOnce(&mut Document, &mut ViewState, &Settings) -> R,
    ) -> Option<R> {
        let pane = self.focused;
        let id = self.active_id()?;
        let doc = self.docs.get_mut(&id)?;
        let vs = self.views.entry((pane, id)).or_default();
        vs.sync(doc);
        Some(f(doc, vs, &self.settings))
    }

    fn remove_tab(&mut self, pane: usize, id: DocId) {
        let p = &mut self.panes[pane];
        if let Some(i) = p.tabs.iter().position(|&t| t == id) {
            p.tabs.remove(i);
            if p.active > i || p.active >= p.tabs.len() {
                p.active = p.active.saturating_sub(1);
            }
        }
        self.views.remove(&(pane, id));
        let still_open = self.panes.iter().any(|p| p.tabs.contains(&id));
        if !still_open {
            if let Some(d) = self.docs.remove(&id)
                && let Some(path) = &d.path
            {
                self.closed_stack.push(path.display().to_string());
                self.add_recent(&path.clone());
            }
            self.prompted_mtime.remove(&id);
        }
        if self.panes[1].tabs.is_empty() {
            self.focused = 0;
        }
        if self.panes[0].tabs.is_empty() && !self.panes[1].tabs.is_empty() {
            self.panes.swap(0, 1);
            let moved: Vec<_> = self
                .views
                .drain()
                .map(|((p, d), v)| ((1 - p, d), v))
                .collect();
            self.views.extend(moved);
            self.focused = 0;
        }
    }

    /// Closes a tab, prompting to save if it's dirty and only open here.
    pub fn close_tab(&mut self, pane: usize, id: DocId, then_exit: bool) {
        let shared = self.panes.iter().filter(|p| p.tabs.contains(&id)).count() > 1;
        let dirty = self.docs.get(&id).is_some_and(Document::is_dirty);
        if dirty && !shared {
            self.activate(pane, id);
            self.prompts
                .push(Prompt::SaveBeforeClose { doc: id, then_exit });
            return;
        }
        self.remove_tab(pane, id);
        if self.panes[0].tabs.is_empty() && !then_exit {
            self.new_document(0);
        }
    }

    /// Called by the save prompt.
    pub fn finish_close(&mut self, id: DocId, save: Option<bool>, then_exit: bool) {
        match save {
            None => {
                self.exit_requested = false;
            }
            Some(true) => {
                if self.save_doc(id, false) {
                    self.force_remove(id);
                    if then_exit {
                        self.request_exit();
                    }
                } else {
                    self.exit_requested = false;
                }
            }
            Some(false) => {
                self.force_remove(id);
                if then_exit {
                    self.request_exit();
                }
            }
        }
    }

    fn force_remove(&mut self, id: DocId) {
        for pane in 0..2 {
            if self.panes[pane].tabs.contains(&id) {
                self.remove_tab(pane, id);
            }
        }
        if self.panes[0].tabs.is_empty() && !self.exit_requested {
            self.new_document(0);
        }
    }

    fn all_doc_ids(&self) -> Vec<(usize, DocId)> {
        let mut v = Vec::new();
        for (pi, p) in self.panes.iter().enumerate() {
            for &id in &p.tabs {
                v.push((pi, id));
            }
        }
        v
    }

    pub fn close_many(&mut self, which: &[(usize, DocId)]) {
        for &(pane, id) in which.iter().rev() {
            if self.docs.get(&id).is_some_and(Document::is_dirty) {
                self.activate(pane, id);
                self.prompts.push(Prompt::SaveBeforeClose {
                    doc: id,
                    then_exit: false,
                });
            } else {
                self.remove_tab(pane, id);
            }
        }
        if self.panes[0].tabs.is_empty() && self.prompts.is_empty() {
            self.new_document(0);
        }
    }

    /// Exit: snapshot the session or prompt per dirty doc.
    pub fn request_exit(&mut self) {
        self.exit_requested = true;
        let snapshot =
            self.settings.remember_session && self.settings.backup_unsaved && !self.no_session;
        if !snapshot {
            let dirty: Vec<(usize, DocId)> = self
                .all_doc_ids()
                .into_iter()
                .filter(|(_, id)| self.docs.get(id).is_some_and(Document::is_dirty))
                .collect();
            if let Some(&(pane, id)) = dirty.first() {
                self.activate(pane, id);
                if !self
                    .prompts
                    .iter()
                    .any(|p| matches!(p, Prompt::SaveBeforeClose { .. }))
                {
                    self.prompts.push(Prompt::SaveBeforeClose {
                        doc: id,
                        then_exit: true,
                    });
                }
                return;
            }
        }
        self.save_all_state();
        self.allow_close = true;
        if let Some(ctx) = &self.egui_ctx {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub fn save_doc(&mut self, id: DocId, save_as: bool) -> bool {
        let Some(doc) = self.docs.get(&id) else {
            return false;
        };
        let path = if save_as
            || doc.path.is_none()
            || (doc.path.as_ref().is_some_and(|p| !p.exists()) && doc.is_untitled())
        {
            let mut dlg = rfd::FileDialog::new().set_file_name(doc.display_name());
            if let Some(dir) = doc.path.as_ref().and_then(|p| p.parent()) {
                dlg = dlg.set_directory(dir);
            }
            match dlg.save_file() {
                Some(p) => p,
                None => return false,
            }
        } else {
            doc.path.clone().unwrap_or_default()
        };
        let backup = self.settings.backup_on_save;
        let doc = self.docs.get_mut(&id).unwrap();
        match doc.save_to(&path, backup) {
            Ok(()) => {
                let renamed = doc.path.as_ref() != Some(&path);
                doc.path = Some(path.clone());
                doc.mark_saved();
                doc.read_only = false;
                if renamed && doc.lang == self.langs.normal() {
                    let lang = self.langs.detect(&doc.display_name(), &doc.line_text(0));
                    doc.set_language(lang, &self.langs);
                }
                self.add_recent(&path);
                self.prompted_mtime.remove(&id);
                true
            }
            Err(e) => {
                self.prompts.push(Prompt::Message {
                    title: "Save failed".into(),
                    text: format!(
                        "Please check whether if this file is opened in another program.\n{}: {e}",
                        path.display()
                    ),
                });
                false
            }
        }
    }

    fn save_copy_as(&mut self, id: DocId) {
        let Some(doc) = self.docs.get(&id) else {
            return;
        };
        if let Some(p) = rfd::FileDialog::new()
            .set_file_name(doc.display_name())
            .save_file()
        {
            let bytes = doc.encoding.encode(&doc.rope.to_string());
            if let Err(e) = std::fs::write(&p, bytes) {
                self.message("Save a Copy As", &e.to_string());
            }
        }
    }

    pub fn message(&mut self, title: &str, text: &str) {
        self.prompts.push(Prompt::Message {
            title: title.into(),
            text: text.into(),
        });
    }

    pub fn flash(&mut self, msg: impl Into<String>) {
        let t = self.egui_ctx.as_ref().map_or(0.0, |c| c.input(|i| i.time));
        self.status_msg = Some((msg.into(), t));
    }

    // ---------------------------------------------------------------- session

    pub fn build_session(&mut self, write_backups: bool) -> Session {
        self.backups_written
            .retain(|id, _| self.docs.get(id).is_some_and(Document::is_dirty));
        let mut session = Session {
            window: self.window_geometry,
            compare: Some(self.settings.compare.clone()),
            focused_view: u8::try_from(self.focused).unwrap_or(0),
            ..Default::default()
        };
        let backup_dir = config::backup_dir();
        if write_backups {
            let _ = std::fs::create_dir_all(&backup_dir);
        }
        let mut live_backups = HashSet::new();
        for pane in 0..2 {
            let tabs = self.panes[pane].tabs.clone();
            for (ti, id) in tabs.iter().enumerate() {
                let Some(doc) = self.docs.get(id) else {
                    continue;
                };
                if self.panes[pane].active == ti {
                    session.active[pane] = Some(session.files.len());
                }
                let vs = self.views.get(&(pane, *id));
                let mut backup = None;
                if write_backups && doc.is_dirty() && !(doc.is_untitled() && doc.len_chars() == 0) {
                    let name = format!(
                        "{}@{}.txt",
                        doc.display_name().replace(['/', '\\', ':'], "_"),
                        doc.id
                    );
                    let p = backup_dir.join(name);
                    // Rewrite a backup only when its document changed, and
                    // big documents at most every 30 s (always on exit).
                    let now = std::time::Instant::now();
                    let fresh = self.backups_written.get(id).is_some_and(|(v, at, path)| {
                        path == &p
                            && p.exists()
                            && (*v == doc.version
                                || (doc.len_chars() > BIG_BACKUP_CHARS
                                    && !self.exit_requested
                                    && now.duration_since(*at) < BIG_BACKUP_INTERVAL))
                    });
                    if fresh || write_backup_file(&p, &doc.rope).is_ok() {
                        if !fresh {
                            self.backups_written
                                .insert(*id, (doc.version, now, p.clone()));
                        }
                        live_backups.insert(p.clone());
                        backup = Some(p.display().to_string());
                    }
                }
                if doc.transient || (doc.is_untitled() && backup.is_none() && doc.len_chars() == 0)
                {
                    // Empty tabs and temporary views (compare snapshots,
                    // inline diffs) aren't part of the session.
                    continue;
                }
                session.files.push(SessionFile {
                    path: doc.path.as_ref().map(|p| p.display().to_string()),
                    backup,
                    untitled_name: doc.untitled_name.clone(),
                    lang: self.langs.langs[doc.lang].name.clone(),
                    encoding: doc.encoding.key(),
                    eol: Some(doc.eol),
                    anchor: vs.map_or(0, |v| v.primary().anchor),
                    caret: vs.map_or(0, |v| v.primary().head),
                    top_row: vs.map_or(0.0, |v| v.top_row),
                    bookmarks: doc.bookmarked_lines(),
                    folds: doc
                        .meta
                        .iter()
                        .enumerate()
                        .filter(|(_, m)| m.folded)
                        .map(|(i, _)| i)
                        .collect(),
                    read_only: doc.read_only,
                    view: u8::try_from(pane).unwrap_or(0),
                    tab_color: doc.tab_color,
                    pinned: doc.pinned,
                    dirty: doc.is_dirty(),
                });
            }
        }
        if write_backups && let Ok(rd) = std::fs::read_dir(&backup_dir) {
            for e in rd.flatten() {
                if !live_backups.contains(&e.path()) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        session
    }

    pub fn save_all_state(&mut self) {
        if self.settings.remember_session && !self.no_session {
            let backups = self.settings.backup_unsaved;
            let s = self.build_session(backups);
            config::save_session(&s);
        }
        self.settings.fif_filters.retain(|f| !f.is_empty());
        config::save_settings(&self.settings);
        config::save_persisted(&self.persisted);
    }

    fn apply_session(&mut self, session: Session) {
        let mut active_ids: [Option<DocId>; 2] = [None, None];
        for (fi, f) in session.files.iter().enumerate() {
            let pane = usize::from(f.view.min(1));
            let id = self.alloc_id();
            let mut doc = if let Some(b) = f.backup.as_ref().filter(|b| Path::new(b).exists()) {
                let text = std::fs::read_to_string(b).unwrap_or_default();
                let lang = self
                    .langs
                    .by_name(&f.lang)
                    .unwrap_or_else(|| self.langs.normal());
                let mut d =
                    Document::from_text(id, f.untitled_name.clone(), &text, &self.langs, lang);
                d.path = f.path.as_ref().map(PathBuf::from);
                d.force_dirty = true;
                if let Some(p) = &d.path {
                    d.mtime = std::fs::metadata(p).and_then(|m| m.modified()).ok();
                }
                d
            } else if let Some(p) = &f.path {
                let enc = TextEncoding::from_key(&f.encoding);
                match Document::open(id, Path::new(p), &self.langs, enc) {
                    Ok(d) => d,
                    Err(_) => continue,
                }
            } else {
                continue;
            };
            if let Some(enc) = TextEncoding::from_key(&f.encoding) {
                doc.encoding = enc;
            }
            if let Some(eol) = f.eol {
                doc.eol = eol;
            }
            if let Some(l) = self.langs.by_name(&f.lang)
                && l != doc.lang
            {
                doc.set_language(l, &self.langs);
            }
            for &b in &f.bookmarks {
                if let Some(m) = doc.meta.get_mut(b) {
                    m.bookmark = true;
                }
            }
            for &fl in &f.folds {
                doc.set_fold(fl, true);
            }
            doc.read_only |= f.read_only;
            doc.tab_color = f.tab_color;
            doc.pinned = f.pinned;
            let len = doc.len_chars();
            self.docs.insert(id, doc);
            self.panes[pane].tabs.push(id);
            let vs = self.views.entry((pane, id)).or_default();
            vs.sels = vec![Selection::new(f.anchor.min(len), f.caret.min(len))];
            vs.top_row = f.top_row;
            if session.active[pane] == Some(fi) {
                active_ids[pane] = Some(id);
            }
        }
        for pane in 0..2 {
            if let Some(id) = active_ids[pane]
                && let Some(i) = self.panes[pane].tabs.iter().position(|&t| t == id)
            {
                self.panes[pane].active = i;
            }
        }
        if self.panes[0].tabs.is_empty() && !self.panes[1].tabs.is_empty() {
            self.panes.swap(0, 1);
        }
        self.focused = usize::from(session.focused_view.min(1));
        if let Some(c) = session.compare {
            self.settings.compare = c;
        }
        if self.panes[self.focused].tabs.is_empty() {
            self.focused = 0;
        }
    }

    fn restore_session(&mut self) {
        let session = config::load_session();
        self.apply_session(session);
    }

    // ---------------------------------------------------------------- per-frame housekeeping

    /// Polls background work: find-in-files, single-instance IPC, file monitoring.
    pub fn poll(&mut self, now: f64) {
        #[cfg(target_os = "macos")]
        {
            let files = crate::platform::finder::take();
            if !files.is_empty() {
                self.open_args(&files);
                if let Some(ctx) = &self.egui_ctx {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
        }
        if let Some(fif) = &mut self.fif {
            let mut done = false;
            while let Ok(msg) = fif.rx.try_recv() {
                match msg {
                    FifMessage::Progress(n) => fif.progress = n,
                    FifMessage::File(fh) => {
                        if let Some(r) = &mut self.results {
                            r.files.push(fh);
                        }
                    }
                    FifMessage::Replaced(n) => fif.replaced += n,
                    FifMessage::Done { searched, error } => {
                        if let Some(r) = &mut self.results {
                            r.searched_files = searched;
                        }
                        if let Some(e) = error {
                            self.find.status = Some((
                                format!("Find in Files: invalid regular expression ({e})"),
                                StatusKind::NotFound,
                            ));
                        } else if fif.replace {
                            self.find.status = Some((
                                format!(
                                    "Replace in Files: {} occurrences were replaced in {searched} searched files",
                                    fif.replaced
                                ),
                                StatusKind::Info,
                            ));
                        } else if let Some(r) = &self.results {
                            self.find.status = Some((
                                format!(
                                    "Find in Files: {} hits in {} files ({searched} searched)",
                                    r.total_hits(),
                                    r.files.len()
                                ),
                                StatusKind::Info,
                            ));
                        }
                        done = true;
                    }
                }
            }
            if done {
                let replace = self.fif.as_ref().is_some_and(|f| f.replace);
                self.fif = None;
                if replace {
                    self.reload_unmodified_from_disk();
                }
            }
        }
        if let Some(rx) = &self.ipc_rx {
            let mut batches = Vec::new();
            while let Ok(args) = rx.try_recv() {
                batches.push(args);
            }
            for args in batches {
                self.open_args(&args);
                if let Some(ctx) = &self.egui_ctx {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
        }
        self.poll_compare(now);
        if self.settings.file_monitoring && now - self.last_monitor_check > 1.5 {
            self.last_monitor_check = now;
            self.check_external_changes();
        }
    }

    pub fn open_args(&mut self, args: &[String]) {
        let mut line = None;
        let mut last = None;
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if a == "--session-dir" {
                it.next(); // its folder argument isn't a file to open
            } else if let Some(n) = a.strip_prefix("-n") {
                line = n.parse::<usize>().ok();
            } else if a.starts_with('-') {
            } else {
                last = self.open_path(Path::new(a), self.focused).or(last);
            }
        }
        if let (Some(id), Some(l)) = (last, line) {
            self.pending_goto = Some((id, l.saturating_sub(1), 0));
        }
    }

    fn reload_unmodified_from_disk(&mut self) {
        let ids: Vec<DocId> = self.docs.keys().copied().collect();
        for id in ids {
            let langs = &self.langs;
            if let Some(d) = self.docs.get_mut(&id)
                && !d.is_dirty()
                && d.path.is_some()
            {
                let changed = d
                    .path
                    .as_ref()
                    .and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
                    != d.mtime;
                if changed {
                    let _ = d.reload(langs);
                }
            }
        }
    }

    fn check_external_changes(&mut self) {
        let ids: Vec<DocId> = self.docs.keys().copied().collect();
        for id in ids {
            if self.prompted_mtime.contains(&id) {
                continue;
            }
            let Some(d) = self.docs.get(&id) else {
                continue;
            };
            let Some(path) = d.path.clone() else { continue };
            if d.mtime.is_none() && !path.exists() {
                continue; // never saved yet
            }
            match std::fs::metadata(&path).and_then(|m| m.modified()) {
                Ok(mt) if Some(mt) != d.mtime => {
                    if self.settings.update_silently && !d.is_dirty() {
                        let langs = &self.langs;
                        if let Some(d) = self.docs.get_mut(&id) {
                            let _ = d.reload(langs);
                        }
                        if self.settings.scroll_to_end_on_update {
                            for pane in 0..2 {
                                if let (Some(vs), Some(d)) =
                                    (self.views.get_mut(&(pane, id)), self.docs.get(&id))
                                {
                                    vs.set_single(d.len_chars());
                                }
                            }
                        }
                    } else {
                        self.prompted_mtime.insert(id);
                        self.prompts.push(Prompt::Reload(id));
                    }
                }
                Err(_) if d.mtime.is_some() => {
                    self.prompted_mtime.insert(id);
                    self.prompts.push(Prompt::Deleted(id));
                }
                _ => {}
            }
        }
    }

    pub fn answer_reload(&mut self, id: DocId, yes: bool) {
        self.prompted_mtime.remove(&id);
        let langs = &self.langs;
        if let Some(d) = self.docs.get_mut(&id) {
            if yes {
                if let Err(e) = d.reload(langs) {
                    let msg = e.to_string();
                    self.message("Reload", &msg);
                }
            } else {
                d.mtime = d
                    .path
                    .as_ref()
                    .and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
            }
        }
    }

    pub fn answer_deleted(&mut self, id: DocId, keep: bool) {
        self.prompted_mtime.remove(&id);
        if keep {
            if let Some(d) = self.docs.get_mut(&id) {
                d.mtime = None;
                d.force_dirty = true;
            }
        } else {
            self.force_remove(id);
        }
    }

    pub fn apply_pending_goto(&mut self) {
        if let Some((id, line, col)) = self.pending_goto.take() {
            let pane = self
                .panes
                .iter()
                .position(|p| p.tabs.contains(&id))
                .unwrap_or(0);
            if let (Some(d), Some(vs)) = (self.docs.get_mut(&id), self.views.get_mut(&(pane, id))) {
                let l = line.min(d.line_count().saturating_sub(1));
                d.ensure_line_visible(l);
                let p = d.line_start(l) + col.min(d.line_len(l));
                vs.set_single(p);
                vs.center_caret = true;
            }
        }
    }

    // ---------------------------------------------------------------- search helpers

    pub fn push_history(&mut self) {
        let w = self.find.what.clone();
        if !w.is_empty() {
            self.settings.find_history.retain(|h| *h != w);
            self.settings.find_history.insert(0, w);
            self.settings.find_history.truncate(20);
        }
        let r = self.find.with.clone();
        if !r.is_empty() {
            self.settings.replace_history.retain(|h| *h != r);
            self.settings.replace_history.insert(0, r);
            self.settings.replace_history.truncate(20);
        }
    }

    /// Find Next / Previous on the active document. Returns whether found.
    pub fn find_in_active(
        &mut self,
        pattern: &str,
        backward: bool,
        opts: &search::SearchOptions,
        record_status: bool,
    ) -> bool {
        let re = match search::build_regex(pattern, opts) {
            Ok(r) => r,
            Err(e) => {
                self.find.status = Some((
                    format!("Find: Invalid regular expression: {e}"),
                    StatusKind::NotFound,
                ));
                return false;
            }
        };
        let wrap = opts.wrap;
        let res = self.with_active(|doc, vs, _| {
            let text = doc.text();
            let sel = vs.primary();
            let found = if backward {
                search::find_prev(&re, &text, doc.char_to_byte(sel.start()), wrap)
            } else {
                let from = doc.char_to_byte(sel.end());
                // Avoid re-finding an empty match at the caret.
                search::find_next(&re, &text, from, wrap).and_then(|(s, e, w)| {
                    if s == e && s == from {
                        search::find_next(&re, &text, from + 1, wrap)
                    } else {
                        Some((s, e, w))
                    }
                })
            };
            if let Some((s, e, wrapped)) = found {
                let (cs, ce) = (doc.byte_to_char(s), doc.byte_to_char(e));
                doc.ensure_line_visible(doc.char_to_line(cs));
                if backward {
                    vs.set_selection(ce, cs);
                } else {
                    vs.set_selection(cs, ce);
                }
                vs.center_caret = true;
                Some(wrapped)
            } else {
                None
            }
        });
        match res.flatten() {
            Some(wrapped) => {
                if record_status {
                    self.find.status = if wrapped {
                        Some((
                            format!(
                                "Find: Found the 1st occurrence from the {}. The {} of the document has been reached.",
                                if backward { "bottom" } else { "top" },
                                if backward { "beginning" } else { "end" }
                            ),
                            StatusKind::Wrapped,
                        ))
                    } else {
                        None
                    };
                }
                true
            }
            None => {
                if record_status {
                    self.find.status = Some((
                        format!("Find: Can't find the text \"{pattern}\""),
                        StatusKind::NotFound,
                    ));
                }
                false
            }
        }
    }

    fn selection_or_word(&mut self) -> Option<String> {
        self.with_active(|doc, vs, _| {
            let p = vs.primary();
            if p.is_empty() {
                let (a, b) = ops::word_at(doc, p.head);
                (a != b).then(|| doc.slice(a, b))
            } else {
                Some(doc.slice(p.start(), p.end()))
            }
        })
        .flatten()
    }

    fn style_token(&mut self, style: u8, all: bool) {
        self.with_active(|doc, vs, _| {
            let p = vs.primary();
            let (a, b, whole) = if p.is_empty() {
                let (a, b) = ops::word_at(doc, p.head);
                (a, b, true)
            } else {
                (p.start(), p.end(), false)
            };
            if a == b {
                return;
            }
            if !all {
                doc.marks.push(Mark {
                    start: a,
                    end: b,
                    style,
                });
            } else {
                let needle = doc.slice(a, b);
                let opts = search::SearchOptions {
                    match_case: true,
                    whole_word: whole,
                    ..Default::default()
                };
                if let Ok(re) = search::build_regex(&needle, &opts) {
                    let text = doc.text();
                    for (s, e) in search::find_all(&re, &text) {
                        let (cs, ce) = (doc.byte_to_char(s), doc.byte_to_char(e));
                        if !doc.marks.iter().any(|m| m.style == style && m.start == cs) {
                            doc.marks.push(Mark {
                                start: cs,
                                end: ce,
                                style,
                            });
                        }
                    }
                }
            }
            doc.marks.sort_by_key(|m| (m.start, m.end));
        });
    }

    fn jump_style(&mut self, style: u8, next: bool) {
        self.with_active(|doc, vs, _| {
            let p = vs.primary();
            let target = if next {
                doc.marks
                    .iter()
                    .filter(|m| m.style == style)
                    .find(|m| m.start > p.start())
                    .or_else(|| doc.marks.iter().find(|m| m.style == style))
            } else {
                doc.marks
                    .iter()
                    .filter(|m| m.style == style)
                    .rev()
                    .find(|m| m.start < p.start())
                    .or_else(|| doc.marks.iter().rev().find(|m| m.style == style))
            };
            if let Some(m) = target.copied() {
                doc.ensure_line_visible(doc.char_to_line(m.start));
                vs.set_selection(m.start, m.end);
            }
        });
    }

    fn bookmark_nav(&mut self, next: bool) {
        self.with_active(|doc, vs, _| {
            let line = doc.char_to_line(vs.primary().head);
            let bms = doc.bookmarked_lines();
            let t = if next {
                bms.iter().find(|&&l| l > line).or_else(|| bms.first())
            } else {
                bms.iter().rev().find(|&&l| l < line).or_else(|| bms.last())
            };
            if let Some(&l) = t {
                doc.ensure_line_visible(l);
                vs.set_single(doc.line_start(l));
                vs.center_caret = true;
            }
        });
    }

    fn change_nav(&mut self, next: bool) {
        self.with_active(|doc, vs, _| {
            let line = doc.char_to_line(vs.primary().head);
            let n = doc.line_count();
            let changed = |l: usize| doc.meta.get(l).is_some_and(|m| m.change != 0);
            let mut found = None;
            if next {
                let mut l = line;
                while l < n && changed(l) {
                    l += 1;
                }
                found = (l..n)
                    .find(|&x| changed(x))
                    .or_else(|| (0..line).find(|&x| changed(x)));
            } else {
                let mut l = line;
                while l > 0 && changed(l) {
                    l -= 1;
                }
                let mut x = l;
                while x > 0 {
                    x -= 1;
                    if changed(x) {
                        while x > 0 && changed(x - 1) {
                            x -= 1;
                        }
                        found = Some(x);
                        break;
                    }
                }
            }
            if let Some(l) = found {
                doc.ensure_line_visible(l);
                vs.set_single(doc.line_start(l));
                vs.center_caret = true;
            }
        });
    }

    fn bookmark_lines_op(&mut self, op: &str) {
        let clip_text = if op == "paste" {
            Some(self.clip.text.clone())
        } else {
            None
        };
        let copied = self.with_active(|doc, vs, _| {
            let bms = doc.bookmarked_lines();
            if bms.is_empty() && op != "inverse" {
                return None;
            }
            let eol = doc.eol.as_str().to_string();
            match op {
                "copy" | "cut" => {
                    let text: Vec<String> = bms.iter().map(|&l| doc.line_text(l)).collect();
                    let joined = text.join(&eol) + &eol;
                    if op == "cut" {
                        delete_line_set(doc, vs, &bms);
                    }
                    Some(joined)
                }
                "remove" => {
                    delete_line_set(doc, vs, &bms);
                    None
                }
                "remove_unmarked" => {
                    let set: HashSet<usize> = bms.iter().copied().collect();
                    let unmarked: Vec<usize> =
                        (0..doc.line_count()).filter(|l| !set.contains(l)).collect();
                    delete_line_set(doc, vs, &unmarked);
                    None
                }
                "inverse" => {
                    for m in &mut doc.meta {
                        m.bookmark = !m.bookmark;
                    }
                    None
                }
                "paste" => {
                    let text = clip_text.clone().unwrap_or_default().replace("\r\n", "\n");
                    let lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
                    let before = vs.sels.clone();
                    let edits = bms
                        .iter()
                        .zip(lines.iter().cycle())
                        .map(|(&l, t)| (doc.line_start(l), doc.line_end(l), (*t).to_string()))
                        .collect();
                    let old = vs.sels.clone();
                    doc.edit(edits, &before, EditKind::Other, |_| old.clone());
                    vs.after_edit(doc);
                    None
                }
                _ => None,
            }
        });
        if let Some(Some(t)) = copied {
            self.clip_set(&t);
        }
    }

    pub fn clip_set(&mut self, t: &str) {
        self.clip.text = t.to_string();
        self.clip.line_mode = false;
        self.clip.rect_mode = false;
        self.clip.history.retain(|h| h != t);
        self.clip.history.insert(0, t.to_string());
        if let Some(ctx) = &self.egui_ctx {
            ctx.copy_text(t.to_string());
        }
    }

    pub fn expand_vars(&self, s: &str) -> String {
        let doc = self.active_doc();
        let path = doc.and_then(|d| d.path.clone()).unwrap_or_default();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let (stem, ext) = match name.rsplit_once('.') {
            Some((a, b)) => (a.to_string(), format!(".{b}")),
            None => (name.clone(), String::new()),
        };
        let dir = path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let (word, line, col) = self
            .active_id()
            .and_then(|id| {
                let d = self.docs.get(&id)?;
                let v = self.views.get(&(self.focused, id))?;
                let p = v.primary();
                let w = if p.is_empty() {
                    let (a, b) = ops::word_at(d, p.head);
                    d.slice(a, b)
                } else {
                    d.slice(p.start(), p.end())
                };
                let (l, c) = d.pos_to_line_col(p.head);
                Some((w, l + 1, c + 1))
            })
            .unwrap_or_default();
        let exe = std::env::current_exe().unwrap_or_default();
        let mut out = s
            .replace("$(FULL_CURRENT_PATH)", &path.display().to_string())
            .replace("$(CURRENT_DIRECTORY)", &dir)
            .replace("$(FILE_NAME)", &name)
            .replace("$(NAME_PART)", &stem)
            .replace("$(EXT_PART)", &ext)
            .replace("$(CURRENT_WORD)", &word)
            .replace("$(CURRENT_LINENUMBER)", &line.to_string())
            .replace("$(CURRENT_LINE)", &line.to_string())
            .replace("$(CURRENT_COLUMN)", &col.to_string())
            .replace(
                "$(APP_DIRECTORY)",
                &exe.parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            )
            .replace("$(APP_FULL_FILE_PATH)", &exe.display().to_string());
        while let Some(start) = out.find("$(SYS.") {
            let Some(end) = out[start..].find(')') else {
                break;
            };
            let var = &out[start + 6..start + end];
            let val = std::env::var(var).unwrap_or_default();
            out.replace_range(start..start + end + 1, &val);
        }
        out
    }

    // ---------------------------------------------------------------- command dispatch

    #[allow(clippy::too_many_lines)]
    pub fn run(&mut self, cmd: Command) {
        let pane = self.focused;
        let s = self.settings.clone();
        match cmd {
            // ------------------------------------------------ File
            Command::New => {
                self.new_document(pane);
            }
            Command::Open => {
                let mut dlg = rfd::FileDialog::new();
                if let Some(dir) = self
                    .active_doc()
                    .and_then(|d| d.path.as_ref())
                    .and_then(|p| p.parent())
                {
                    dlg = dlg.set_directory(dir);
                }
                if let Some(files) = dlg.pick_files() {
                    for f in files {
                        self.open_path(&f, pane);
                    }
                }
            }
            Command::OpenFolderAsWorkspace => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.add_workspace_folder(dir);
                }
            }
            Command::Reload => {
                if let Some(id) = self.active_id() {
                    let dirty = self.docs.get(&id).is_some_and(Document::is_dirty);
                    if dirty {
                        self.prompted_mtime.insert(id);
                        self.prompts.push(Prompt::Reload(id));
                    } else {
                        self.answer_reload(id, true);
                    }
                }
            }
            Command::Save => {
                if let Some(id) = self.active_id() {
                    self.save_doc(id, false);
                }
            }
            Command::SaveAs => {
                if let Some(id) = self.active_id() {
                    self.save_doc(id, true);
                }
            }
            Command::SaveCopyAs => {
                if let Some(id) = self.active_id() {
                    self.save_copy_as(id);
                }
            }
            Command::SaveAll => {
                for (_, id) in self.all_doc_ids() {
                    if self.docs.get(&id).is_some_and(Document::is_dirty) {
                        self.save_doc(id, false);
                    }
                }
            }
            Command::Rename => {
                if let Some(d) = self.active_doc() {
                    self.dialogs.rename = Some((d.id, d.display_name()));
                }
            }
            Command::Close => {
                if let Some(id) = self.active_id() {
                    self.close_tab(pane, id, false);
                }
            }
            Command::CloseAll => {
                let all = self.all_doc_ids();
                self.close_many(&all);
            }
            Command::CloseAllButCurrent => {
                let cur = self.active_id();
                let v: Vec<_> = self
                    .all_doc_ids()
                    .into_iter()
                    .filter(|(_, id)| Some(*id) != cur)
                    .collect();
                self.close_many(&v);
            }
            Command::CloseAllButPinned => {
                let v: Vec<_> = self
                    .all_doc_ids()
                    .into_iter()
                    .filter(|(_, id)| !self.docs.get(id).is_some_and(|d| d.pinned))
                    .collect();
                self.close_many(&v);
            }
            Command::CloseAllToLeft | Command::CloseAllToRight => {
                let p = &self.panes[pane];
                let a = p.active;
                let v: Vec<_> = p
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| {
                        if cmd == Command::CloseAllToLeft {
                            *i < a
                        } else {
                            *i > a
                        }
                    })
                    .map(|(_, id)| (pane, *id))
                    .collect();
                self.close_many(&v);
            }
            Command::CloseAllUnchanged => {
                let v: Vec<_> = self
                    .all_doc_ids()
                    .into_iter()
                    .filter(|(_, id)| !self.docs.get(id).is_some_and(Document::is_dirty))
                    .collect();
                self.close_many(&v);
            }
            Command::RestoreLastClosed => {
                while let Some(p) = self.closed_stack.pop() {
                    if Path::new(&p).exists() {
                        self.open_path(Path::new(&p), pane);
                        break;
                    }
                }
            }
            Command::OpenAllRecent => {
                for f in self.persisted.recent_files.clone() {
                    if Path::new(&f).exists() {
                        self.open_path(Path::new(&f), pane);
                    }
                }
            }
            Command::ClearRecent => self.persisted.recent_files.clear(),
            Command::OpenRecent(p) => {
                if Path::new(&p).exists() {
                    self.open_path(Path::new(&p), pane);
                } else {
                    self.persisted.recent_files.retain(|r| *r != p);
                    self.message("Open", &format!("\"{p}\" doesn't exist anymore."));
                }
            }
            Command::LoadSession => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Session", &["json"])
                    .pick_file()
                    && let Some(sess) = std::fs::read_to_string(p)
                        .ok()
                        .and_then(|t| serde_json::from_str::<Session>(&t).ok())
                {
                    self.apply_session(sess);
                }
            }
            Command::SaveSession => {
                if let Some(p) = rfd::FileDialog::new()
                    .set_file_name("session.json")
                    .save_file()
                {
                    let sess = self.build_session(false);
                    if let Ok(t) = serde_json::to_string_pretty(&sess) {
                        let _ = std::fs::write(p, t);
                    }
                }
            }
            Command::OpenContainingFolder => {
                if let Some(p) = self.active_doc().and_then(|d| d.path.clone()) {
                    crate::platform::reveal_in_file_manager(&p);
                }
            }
            Command::OpenTerminalHere => {
                if let Some(dir) = self
                    .active_doc()
                    .and_then(|d| d.path.clone())
                    .and_then(|p| p.parent().map(Path::to_path_buf))
                {
                    crate::platform::open_terminal(&dir);
                }
            }
            Command::OpenInDefaultViewer | Command::ViewInBrowser => {
                if let Some(p) = self.active_doc().and_then(|d| d.path.clone()) {
                    crate::platform::open_with_default_app(&p.display().to_string());
                }
            }
            Command::Exit => self.request_exit(),
            // ------------------------------------------------ Edit
            Command::Undo => {
                self.with_active(|d, v, _| {
                    if let Some(sels) = d.undo() {
                        v.sels = sels;
                        v.after_edit(d);
                    }
                });
            }
            Command::Redo => {
                self.with_active(|d, v, _| {
                    if let Some(sels) = d.redo() {
                        v.sels = sels;
                        v.after_edit(d);
                    }
                });
            }
            Command::Cut | Command::Copy => {
                let cut = cmd == Command::Cut;
                let mut clip = std::mem::take(&mut self.clip);
                let mut over = self.overtype;
                let globals = self.globals.clone();
                let styles = LangStyles {
                    kinds: Vec::new(),
                    keywords: Vec::new(),
                };
                self.with_active(|d, v, st| {
                    let mut env = crate::editor::EditorEnv {
                        settings: st,
                        globals: &globals,
                        styles: &styles,
                        keywords: &[],
                        clip: &mut clip,
                        overtype: &mut over,
                        font_size: 12.0,
                    };
                    crate::editor::copy(v, d, &mut env, cut);
                });
                self.clip = clip;
                if let Some(t) = crate::editor::take_copy_request()
                    && let Some(ctx) = &self.egui_ctx
                {
                    ctx.copy_text(t);
                }
            }
            Command::Paste => {
                // Request the OS clipboard; egui delivers it as a Paste event to the editor.
                if let Some(ctx) = &self.egui_ctx {
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                }
                self.request_focus_editor = true;
            }
            Command::DeleteSelection => {
                self.with_active(|d, v, _| {
                    if v.sels.iter().any(|s| !s.is_empty()) {
                        ops::delete(d, v, ops::DeleteUnit::CharBack);
                    } else {
                        ops::delete(d, v, ops::DeleteUnit::CharForward);
                    }
                });
            }
            Command::SelectAll => {
                self.with_active(|d, v, _| v.set_selection(0, d.len_chars()));
            }
            Command::BeginEndSelect | Command::BeginEndSelectColumn => {
                let column = cmd == Command::BeginEndSelectColumn;
                self.with_active(|d, v, _| {
                    if let Some(start) = v.begin_select.take() {
                        let head = v.primary().head;
                        if column {
                            let a = (d.char_to_line(start), v.display_col(d, start));
                            let h = (d.char_to_line(head), v.display_col(d, head));
                            v.set_rect(d, a, h);
                        } else {
                            v.set_selection(start, head);
                        }
                    } else {
                        v.begin_select = Some(v.primary().head);
                    }
                });
            }
            Command::InsertDateTimeShort
            | Command::InsertDateTimeLong
            | Command::InsertDateTimeCustom => {
                let now = chrono::Local::now();
                let text = match cmd {
                    Command::InsertDateTimeShort => now.format("%H:%M %x").to_string(),
                    Command::InsertDateTimeLong => now.format("%H:%M %A, %B %-d, %Y").to_string(),
                    _ => config::format_date(&s.date_time_format, now),
                };
                self.with_active(|d, v, _| ops::insert_text(d, v, &text, EditKind::Other));
            }
            Command::CopyFullPath | Command::CopyFileName | Command::CopyDirPath => {
                if let Some(d) = self.active_doc() {
                    let t = match cmd {
                        Command::CopyFullPath => d.full_path_or_name(),
                        Command::CopyFileName => d.display_name(),
                        _ => d
                            .path
                            .as_ref()
                            .and_then(|p| p.parent())
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                    };
                    self.clip_set(&t);
                }
            }
            Command::CopyAllNames | Command::CopyAllPaths => {
                let eol = Eol::platform_default().as_str();
                let names: Vec<String> = self
                    .all_doc_ids()
                    .iter()
                    .filter_map(|(_, id)| self.docs.get(id))
                    .map(|d| {
                        if cmd == Command::CopyAllNames {
                            d.display_name()
                        } else {
                            d.full_path_or_name()
                        }
                    })
                    .collect();
                self.clip_set(&names.join(eol));
            }
            Command::IndentIncrease | Command::IndentDecrease => {
                let inc = cmd == Command::IndentIncrease;
                self.with_active(|d, v, st| {
                    ops::indent_lines(d, v, st.tab_size, st.use_spaces, inc)
                });
            }
            Command::ConvertCase(mode) => {
                self.with_active(|d, v, _| {
                    ops::transform_selections(d, v, |t| textops::convert_case(t, mode))
                });
            }
            Command::DuplicateLine => {
                self.with_active(|d, v, _| ops::duplicate(d, v));
            }
            Command::RemoveDuplicateLines => {
                self.with_active(|d, v, _| {
                    ops::transform_lines(d, v, textops::remove_duplicate_lines)
                });
            }
            Command::RemoveConsecutiveDuplicates => {
                self.with_active(|d, v, _| {
                    ops::transform_lines(d, v, textops::remove_consecutive_duplicates)
                });
            }
            Command::SplitLines => {
                let width = if s.edge_column > 0 { s.edge_column } else { 80 };
                self.with_active(|d, v, _| {
                    ops::transform_lines(d, v, |lines| {
                        lines
                            .iter()
                            .flat_map(|l| textops::split_line(l, width))
                            .collect()
                    })
                });
            }
            Command::JoinLines => {
                self.with_active(|d, v, _| ops::join_lines(d, v));
            }
            Command::MoveLineUp | Command::MoveLineDown => {
                let up = cmd == Command::MoveLineUp;
                self.with_active(|d, v, _| ops::move_lines(d, v, up));
            }
            Command::RemoveEmptyLines | Command::RemoveEmptyLinesWithBlank => {
                let blank = cmd == Command::RemoveEmptyLinesWithBlank;
                self.with_active(|d, v, _| {
                    ops::transform_lines(d, v, |l| textops::remove_empty_lines(l, blank))
                });
            }
            Command::InsertBlankAbove | Command::InsertBlankBelow => {
                let above = cmd == Command::InsertBlankAbove;
                self.with_active(|d, v, _| ops::insert_blank_line(d, v, above));
            }
            Command::Sort(mode) => {
                self.with_active(|d, v, _| {
                    // Column (rectangular) selection sorts by the selected columns.
                    if let Some((a, h)) = v.rect
                        && a.1 != h.1
                    {
                        let (l1, l2) = (a.0.min(h.0), a.0.max(h.0));
                        let (c1, c2) = (a.1.min(h.1), a.1.max(h.1));
                        let mut lines: Vec<String> = (l1..=l2).map(|l| d.line_text(l)).collect();
                        let key = |s: &String| -> String {
                            let cols = crate::editor::columns(s, v.tab_width);
                            let a = cols.iter().position(|&c| c >= c1).unwrap_or(cols.len() - 1);
                            let b = cols.iter().position(|&c| c >= c2).unwrap_or(cols.len() - 1);
                            s.chars().skip(a).take(b.saturating_sub(a)).collect()
                        };
                        let mut keyed: Vec<(String, String)> =
                            lines.drain(..).map(|l| (key(&l), l)).collect();
                        let mut keys: Vec<String> = keyed.iter().map(|k| k.0.clone()).collect();
                        textops::sort_lines(&mut keys, mode);
                        let mut out = Vec::new();
                        for k in keys {
                            if let Some(i) = keyed.iter().position(|x| x.0 == k) {
                                out.push(keyed.remove(i).1);
                            }
                        }
                        ops::replace_lines(d, v, l1, l2, &out);
                    } else {
                        ops::transform_lines(d, v, |mut l| {
                            textops::sort_lines(&mut l, mode);
                            l
                        });
                    }
                });
            }
            Command::DeleteLine => {
                self.with_active(|d, v, _| ops::delete_lines(d, v));
            }
            Command::TransposeLine => {
                self.with_active(|d, v, _| ops::transpose_line(d, v));
            }
            Command::ToggleComment | Command::BlockComment | Command::BlockUncomment => {
                let mode = match cmd {
                    Command::ToggleComment => ops::CommentMode::Toggle,
                    Command::BlockComment => ops::CommentMode::Comment,
                    _ => ops::CommentMode::Uncomment,
                };
                let lang = self.active_doc().map(|d| self.langs.langs[d.lang].clone());
                if let Some(lang) = lang {
                    if let Some(prefix) = lang.comment_line.clone() {
                        self.with_active(|d, v, _| ops::line_comment(d, v, &prefix, mode));
                    } else if let (Some(a), Some(b)) =
                        (lang.comment_start.clone(), lang.comment_end.clone())
                    {
                        let add = mode != ops::CommentMode::Uncomment;
                        self.with_active(|d, v, _| ops::stream_comment(d, v, &a, &b, add));
                    }
                }
            }
            Command::StreamComment | Command::StreamUncomment => {
                let lang = self.active_doc().map(|d| self.langs.langs[d.lang].clone());
                if let Some(lang) = lang
                    && let (Some(a), Some(b)) =
                        (lang.comment_start.clone(), lang.comment_end.clone())
                {
                    let add = cmd == Command::StreamComment;
                    self.with_active(|d, v, _| ops::stream_comment(d, v, &a, &b, add));
                }
            }
            Command::WordCompletion | Command::FunctionCompletion => {
                let lang = self.active_doc().map(|d| d.lang);
                let kws = lang.map(|l| self.keywords_for(l)).unwrap_or_default();
                let words = cmd == Command::WordCompletion;
                self.with_active(|d, v, _| {
                    crate::editor::open_autocomplete(v, d, &kws, words, !words, 0, true)
                });
            }
            Command::EolWindows | Command::EolUnix | Command::EolMac => {
                let eol = match cmd {
                    Command::EolWindows => Eol::Crlf,
                    Command::EolUnix => Eol::Lf,
                    _ => Eol::Cr,
                };
                self.with_active(|d, v, _| ops::convert_eol(d, v, eol));
            }
            Command::TrimTrailing => {
                self.with_active(|d, v, _| ops::transform_each_line(d, v, textops::trim_trailing));
            }
            Command::TrimLeading => {
                self.with_active(|d, v, _| ops::transform_each_line(d, v, textops::trim_leading));
            }
            Command::TrimBoth => {
                self.with_active(|d, v, _| ops::transform_each_line(d, v, textops::trim_both));
            }
            Command::EolToSpace | Command::TrimAndEolToSpace => {
                let trim = cmd == Command::TrimAndEolToSpace;
                self.with_active(|d, v, _| {
                    ops::transform_lines(d, v, |lines| {
                        let parts: Vec<String> = lines
                            .iter()
                            .map(|l| {
                                if trim {
                                    textops::trim_both(l)
                                } else {
                                    l.clone()
                                }
                            })
                            .collect();
                        vec![parts.join(" ")]
                    });
                });
            }
            Command::TabToSpace => {
                let t = s.tab_size;
                self.with_active(|d, v, _| {
                    ops::transform_each_line(d, v, |l| textops::tabs_to_spaces(l, t))
                });
            }
            Command::SpaceToTabAll | Command::SpaceToTabLeading => {
                let t = s.tab_size;
                let lead = cmd == Command::SpaceToTabLeading;
                self.with_active(|d, v, _| {
                    ops::transform_each_line(d, v, |l| textops::spaces_to_tabs(l, t, lead))
                });
            }
            Command::ColumnEditor => {
                self.dialogs.column_editor = Some(crate::dialogs::ColumnEditorState::default())
            }
            Command::CharacterPanel => {
                self.settings.show_char_panel = !self.settings.show_char_panel
            }
            Command::ClipboardHistory => {
                self.settings.show_clipboard_history = !self.settings.show_clipboard_history
            }
            Command::ToggleReadOnly => {
                if let Some(id) = self.active_id()
                    && let Some(d) = self.docs.get_mut(&id)
                {
                    d.read_only = !d.read_only;
                }
            }
            Command::MultiSelectAll {
                match_case,
                whole_word,
            }
            | Command::MultiSelectNext {
                match_case,
                whole_word,
            } => {
                let all = matches!(cmd, Command::MultiSelectAll { .. });
                self.with_active(|d, v, _| {
                    ops::select_next_occurrence(d, v, match_case, whole_word, all)
                });
            }
            Command::MultiSelectUndo => {
                self.with_active(|_, v, _| {
                    if v.sels.len() > 1 {
                        v.sels.remove(v.primary_idx);
                        v.primary_idx = v.sels.len() - 1;
                    }
                });
            }
            Command::MultiSelectSkip => {
                self.with_active(|d, v, _| {
                    if v.sels.len() > 1 {
                        v.sels.remove(v.primary_idx);
                        v.primary_idx = v.sels.len() - 1;
                    }
                    ops::select_next_occurrence(d, v, false, false, false);
                });
            }
            Command::Redact => {
                self.with_active(|d, v, _| {
                    ops::transform_selections(d, v, |t| {
                        t.chars()
                            .map(|c| if c == '\n' || c == '\r' { c } else { '█' })
                            .collect()
                    });
                });
            }
            Command::SearchOnInternet => {
                if let Some(w) = self.selection_or_word() {
                    let url = s
                        .search_engine
                        .replace("$(CURRENT_WORD)", &crate::platform::percent_encode(&w));
                    crate::platform::open_with_default_app(&url);
                }
            }
            Command::OpenSelectedFile => {
                if let Some(w) = self.selection_or_word().or_else(|| {
                    self.with_active(|d, v, _| {
                        d.line_text(d.char_to_line(v.primary().head))
                            .trim()
                            .to_string()
                    })
                }) {
                    let base = self
                        .active_doc()
                        .and_then(|d| d.path.as_ref())
                        .and_then(|p| p.parent().map(Path::to_path_buf));
                    let p = PathBuf::from(w.trim().trim_matches('"'));
                    let full = if p.is_absolute() {
                        p
                    } else {
                        base.unwrap_or_default().join(p)
                    };
                    if full.is_file() {
                        self.open_path(&full, pane);
                    } else {
                        self.message(
                            "Open selected file",
                            &format!("\"{}\" doesn't exist.", full.display()),
                        );
                    }
                }
            }
            // ------------------------------------------------ Search
            Command::Find | Command::Replace | Command::Mark | Command::FindInFiles => {
                self.find.open = true;
                self.find.tab = match cmd {
                    Command::Find => FindTab::Find,
                    Command::Replace => FindTab::Replace,
                    Command::Mark => FindTab::Mark,
                    _ => FindTab::FindInFiles,
                };
                if let Some(t) = self.with_active(|d, v, _| {
                    let p = v.primary();
                    (!p.is_empty() && !d.slice(p.start(), p.end()).contains('\n'))
                        .then(|| d.slice(p.start(), p.end()))
                }) && let Some(t) = t
                {
                    self.find.what = t;
                }
                if cmd == Command::FindInFiles
                    && (self.find.fif_dir.is_empty() || self.find.fif_follow_current)
                    && let Some(dir) = self
                        .active_doc()
                        .and_then(|d| d.path.as_ref())
                        .and_then(|p| p.parent())
                {
                    self.find.fif_dir = dir.display().to_string();
                }
                self.find.focus_what = true;
            }
            Command::FindNext | Command::FindPrev => {
                if self.find.what.is_empty() {
                    self.run(Command::Find);
                    return;
                }
                let what = self.find.what.clone();
                let opts = s.search_opts.clone();
                self.find_in_active(&what, cmd == Command::FindPrev, &opts, true);
            }
            Command::SelectAndFindNext | Command::SelectAndFindPrev => {
                if let Some(w) = self.selection_or_word() {
                    self.find.what = w.clone();
                    self.push_history();
                    let mut opts = s.search_opts.clone();
                    opts.mode = search::SearchMode::Normal;
                    self.find_in_active(&w, cmd == Command::SelectAndFindPrev, &opts, true);
                }
            }
            Command::VolatileFindNext | Command::VolatileFindPrev => {
                if let Some(w) = self.selection_or_word() {
                    let opts = search::SearchOptions {
                        match_case: true,
                        whole_word: true,
                        wrap: true,
                        ..Default::default()
                    };
                    self.find_in_active(&w, cmd == Command::VolatileFindPrev, &opts, true);
                }
            }
            Command::IncrementalSearch => {
                self.incremental.open = true;
                self.incremental.focus = true;
                self.incremental.origin = self.with_active(|_, v, _| v.primary().start());
            }
            Command::SearchResultsWindow => self.results_visible = !self.results_visible,
            Command::NextSearchResult | Command::PrevSearchResult => {
                self.step_search_result(cmd == Command::NextSearchResult);
            }
            Command::GoTo => {
                let cur = self
                    .with_active(|d, v, _| d.char_to_line(v.primary().head) + 1)
                    .unwrap_or(1);
                self.dialogs.goto = Some((true, cur.to_string()));
            }
            Command::GoToMatchingBrace | Command::SelectToMatchingBrace => {
                let select = cmd == Command::SelectToMatchingBrace;
                self.with_active(|d, v, _| {
                    let p = v.primary().head;
                    if let Some((a, b)) = ops::matching_brace(d, p)
                        && b != usize::MAX
                    {
                        if select {
                            let (lo, hi) = (a.min(b), a.max(b) + 1);
                            v.set_selection(lo, hi);
                        } else {
                            let target = if b > a { b + 1 } else { b };
                            v.set_single(if p == a + 1 && b > a { b + 1 } else { target });
                        }
                    }
                });
            }
            Command::ChangeHistoryNext => self.change_nav(true),
            Command::ChangeHistoryPrev => self.change_nav(false),
            Command::ChangeHistoryClear => {
                if let Some(id) = self.active_id()
                    && let Some(d) = self.docs.get_mut(&id)
                {
                    d.clear_change_history();
                }
            }
            Command::StyleAll(n) => self.style_token(n, true),
            Command::StyleOne(n) => self.style_token(n, false),
            Command::ClearStyle(n) => {
                if let Some(id) = self.active_id()
                    && let Some(d) = self.docs.get_mut(&id)
                {
                    d.marks.retain(|m| m.style != n);
                }
            }
            Command::ClearAllStyles => {
                if let Some(id) = self.active_id()
                    && let Some(d) = self.docs.get_mut(&id)
                {
                    d.marks.retain(|m| m.style == 0 || m.style > 5);
                }
            }
            Command::JumpNextStyle(n) => self.jump_style(n, true),
            Command::JumpPrevStyle(n) => self.jump_style(n, false),
            Command::CopyStyledText(n) => {
                if let Some(d) = self.active_doc() {
                    let eol = d.eol.as_str();
                    let t: Vec<String> = d
                        .marks
                        .iter()
                        .filter(|m| n == 0 || m.style == n)
                        .map(|m| d.slice(m.start, m.end))
                        .collect();
                    let joined = t.join(eol);
                    self.clip_set(&joined);
                }
            }
            Command::ToggleBookmark => {
                self.with_active(|d, v, _| {
                    let lines = ops::all_sel_lines(d, v);
                    let l = d.char_to_line(v.primary().head);
                    if lines.len() > 1 {
                        for x in lines {
                            d.toggle_bookmark(x);
                        }
                    } else {
                        d.toggle_bookmark(l);
                    }
                });
            }
            Command::NextBookmark => self.bookmark_nav(true),
            Command::PrevBookmark => self.bookmark_nav(false),
            Command::ClearBookmarks => {
                if let Some(id) = self.active_id()
                    && let Some(d) = self.docs.get_mut(&id)
                {
                    for m in &mut d.meta {
                        m.bookmark = false;
                    }
                }
            }
            Command::CutBookmarkedLines => self.bookmark_lines_op("cut"),
            Command::CopyBookmarkedLines => self.bookmark_lines_op("copy"),
            Command::PasteToBookmarkedLines => self.bookmark_lines_op("paste"),
            Command::RemoveBookmarkedLines => self.bookmark_lines_op("remove"),
            Command::RemoveUnbookmarkedLines => self.bookmark_lines_op("remove_unmarked"),
            Command::InverseBookmarks => self.bookmark_lines_op("inverse"),
            Command::FindCharsInRange => {
                self.dialogs.char_range = Some(crate::dialogs::CharRangeDialog::default())
            }
            // ------------------------------------------------ View
            Command::AlwaysOnTop => {
                self.always_on_top = !self.always_on_top;
                if let Some(ctx) = &self.egui_ctx {
                    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                        if self.always_on_top {
                            egui::WindowLevel::AlwaysOnTop
                        } else {
                            egui::WindowLevel::Normal
                        },
                    ));
                }
            }
            Command::FullScreen => {
                self.fullscreen = !self.fullscreen;
                if let Some(ctx) = &self.egui_ctx {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                }
            }
            Command::PostIt | Command::DistractionFree => {
                self.postit = !self.postit;
                if cmd == Command::DistractionFree {
                    self.fullscreen = self.postit;
                    if let Some(ctx) = &self.egui_ctx {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                    }
                }
            }
            Command::ShowWhitespace => {
                self.settings.show_whitespace = !self.settings.show_whitespace
            }
            Command::ShowEol => self.settings.show_eol = !self.settings.show_eol,
            Command::ShowAllChars => {
                let on = !(self.settings.show_whitespace && self.settings.show_eol);
                self.settings.show_whitespace = on;
                self.settings.show_eol = on;
            }
            Command::ShowIndentGuides => {
                self.settings.show_indent_guides = !self.settings.show_indent_guides
            }
            Command::ShowWrapSymbol => {
                self.settings.show_wrap_symbol = !self.settings.show_wrap_symbol
            }
            Command::SetFontSize(size) => self.settings.font_size = f32::from(size),
            Command::FontBigger | Command::FontSmaller => {
                let step = if cmd == Command::FontBigger {
                    1.0
                } else {
                    -1.0
                };
                self.settings.font_size = (self.settings.font_size.round() + step)
                    .clamp(f32::from(MIN_FONT_SIZE), f32::from(MAX_FONT_SIZE));
            }
            Command::DateTimeTool => {
                self.dialogs.datetime = Some(crate::dialogs::DateTimeDialog::now());
            }
            Command::NewWindow => {
                if let Err(e) = crate::platform::open_new_window(&[]) {
                    self.flash(format!("Couldn't open a new window: {e}"));
                }
            }
            Command::MoveToNewWindow => {
                if let Some(id) = self.active_id() {
                    let doc = &self.docs[&id];
                    match doc.path.clone() {
                        Some(path) if !doc.is_dirty() => {
                            match crate::platform::open_new_window(&[path]) {
                                Ok(()) => self.close_tab(pane, id, false),
                                Err(e) => self.flash(format!("Couldn't open a new window: {e}")),
                            }
                        }
                        _ => self.flash("Save the document first to move it to a new window"),
                    }
                }
            }
            Command::MoveToOtherView | Command::CloneToOtherView => {
                if let Some(id) = self.active_id() {
                    let other = 1 - pane;
                    let vs_copy = self
                        .views
                        .get(&(pane, id))
                        .map(|v| (v.sels.clone(), v.top_row));
                    if self.panes[other].tabs.contains(&id) {
                        self.activate(other, id);
                    } else {
                        self.add_tab(other, id);
                    }
                    if let Some((sels, top)) = vs_copy {
                        let v = self.views.entry((other, id)).or_default();
                        v.sels = sels;
                        v.top_row = top;
                        if let Some(d) = self.docs.get(&id) {
                            v.seen_version = d.version;
                        }
                    }
                    if cmd == Command::MoveToOtherView {
                        self.remove_tab(pane, id);
                        if self.panes[pane].tabs.is_empty() && pane == 0 {
                            // remove_tab already swapped panes
                        }
                        let target = if self.panes[1].tabs.contains(&id) {
                            1
                        } else {
                            0
                        };
                        self.activate(target, id);
                    }
                }
            }
            Command::GoToTab(i) => {
                if i < self.panes[pane].tabs.len() {
                    self.panes[pane].active = i;
                    self.request_focus_editor = true;
                }
            }
            Command::NextTab | Command::PrevTab => {
                let p = &mut self.panes[pane];
                if !p.tabs.is_empty() {
                    let n = p.tabs.len();
                    p.active = if cmd == Command::NextTab {
                        (p.active + 1) % n
                    } else {
                        (p.active + n - 1) % n
                    };
                    self.request_focus_editor = true;
                }
            }
            Command::MoveTabForward | Command::MoveTabBackward => {
                let p = &mut self.panes[pane];
                let a = p.active;
                let b = if cmd == Command::MoveTabForward {
                    a + 1
                } else {
                    a.wrapping_sub(1)
                };
                if b < p.tabs.len() {
                    p.tabs.swap(a, b);
                    p.active = b;
                }
            }
            Command::WordWrap => self.settings.word_wrap = !self.settings.word_wrap,
            Command::SyntaxHighlighting => {
                self.settings.syntax_highlighting = !self.settings.syntax_highlighting;
            }
            Command::FocusOtherView => {
                if !self.panes[1 - pane].tabs.is_empty() {
                    self.focused = 1 - pane;
                    self.request_focus_editor = true;
                }
            }
            Command::FoldAll | Command::UnfoldAll => {
                let fold = cmd == Command::FoldAll;
                self.with_active(|d, _, _| d.fold_all(fold, None));
            }
            Command::FoldLevel(n) | Command::UnfoldLevel(n) => {
                let fold = matches!(cmd, Command::FoldLevel(_));
                self.with_active(|d, _, _| d.fold_all(fold, Some(n)));
            }
            Command::FoldCurrent | Command::UnfoldCurrent => {
                let fold = cmd == Command::FoldCurrent;
                self.with_active(|d, v, _| {
                    let l = d.char_to_line(v.primary().head);
                    if let Some(h) = d.enclosing_header(l) {
                        d.set_fold(h, fold);
                        if fold {
                            v.set_single(d.line_start(h));
                        }
                    }
                });
            }
            Command::Summary => self.dialogs.summary = true,
            Command::ToggleDocMap => self.settings.show_doc_map = !self.settings.show_doc_map,
            Command::ToggleDocList => self.settings.show_doc_list = !self.settings.show_doc_list,
            Command::ToggleFunctionList => {
                self.settings.show_function_list = !self.settings.show_function_list
            }
            Command::FilterLines => {
                self.filter.open = true;
                self.filter.focus = true;
            }
            Command::ExportFilteredLines => self.export_filtered_lines(),
            Command::ToggleJsonTree => {
                self.settings.show_json_tree = !self.settings.show_json_tree;
                if !self.settings.show_json_tree {
                    // Free the tree's memory while the panel is closed.
                    self.json_tree = crate::panels::JsonTreeState::default();
                }
            }
            Command::ToggleFolderTree => {
                self.settings.show_folder_tree = !self.settings.show_folder_tree
            }
            Command::SyncScrollV => self.settings.sync_scroll_v = !self.settings.sync_scroll_v,
            Command::SyncScrollH => self.settings.sync_scroll_h = !self.settings.sync_scroll_h,
            Command::ToggleToolbar => self.settings.show_toolbar = !self.settings.show_toolbar,
            Command::ToggleStatusbar => {
                self.settings.show_statusbar = !self.settings.show_statusbar
            }
            Command::TabColor(c) => {
                if let Some(id) = self.active_id()
                    && let Some(d) = self.docs.get_mut(&id)
                {
                    d.tab_color = c;
                }
            }
            Command::TogglePin => {
                if let Some(id) = self.active_id()
                    && let Some(d) = self.docs.get_mut(&id)
                {
                    d.pinned = !d.pinned;
                }
                // Pinned tabs go first.
                let docs = &self.docs;
                let p = &mut self.panes[pane];
                let active = p.active_doc();
                p.tabs
                    .sort_by_key(|id| !docs.get(id).is_some_and(|d| d.pinned));
                if let Some(a) = active {
                    p.active = p.tabs.iter().position(|&t| t == a).unwrap_or(0);
                }
            }
            // ------------------------------------------------ Encoding / Language
            Command::SetEncoding(key) => {
                if let (Some(enc), Some(id)) = (TextEncoding::from_key(&key), self.active_id()) {
                    let langs = &self.langs;
                    let d = self.docs.get_mut(&id).unwrap();
                    if d.is_dirty() && d.path.is_some() {
                        self.message("Encoding", "Your file has been modified. Save it before reinterpreting it with another encoding.");
                    } else if let Some(p) = d.path.clone() {
                        match Document::open(id, &p, langs, Some(enc)) {
                            Ok(mut nd) => {
                                nd.lang = d.lang;
                                nd.set_language(d.lang, langs);
                                nd.untitled_name = d.untitled_name.clone();
                                *d = nd;
                            }
                            Err(e) => {
                                let msg = e.to_string();
                                self.message("Encoding", &msg);
                            }
                        }
                    } else {
                        d.encoding = enc;
                    }
                }
            }
            Command::ConvertEncoding(key) => {
                if let (Some(enc), Some(id)) = (TextEncoding::from_key(&key), self.active_id())
                    && let Some(d) = self.docs.get_mut(&id)
                    && d.encoding != enc
                {
                    d.encoding = enc;
                    d.force_dirty = true;
                }
            }
            Command::SetLanguage(name) => {
                if let Some(l) = self.langs.by_name(&name)
                    && let Some(id) = self.active_id()
                    && let Some(d) = self.docs.get_mut(&id)
                {
                    d.set_language(l, &self.langs);
                }
            }
            // ------------------------------------------------ Settings / tools
            Command::Preferences => self.dialogs.preferences = true,
            Command::StyleConfigurator => self.dialogs.style_config = true,
            Command::ShortcutMapper => self.dialogs.shortcut_mapper = true,
            Command::Hash(algo, source) => self.hash_command(algo, source),
            Command::Mime(op) => self.mime_command(op),
            Command::Data(op) => {
                use crate::commands::DataOp;
                use crate::datatools as dt;
                type Convert = fn(&str) -> Result<String, String>;
                let (title, f): (&str, Convert) = match op {
                    DataOp::CsvToJson => ("CSV to JSON", |s| dt::csv_to_json(s, ',')),
                    DataOp::JsonToCsv => ("JSON to CSV", |s| dt::json_to_csv(s, ',')),
                    DataOp::CsvToTsv => ("CSV to TSV", dt::csv_to_tsv),
                    DataOp::TsvToCsv => ("TSV to CSV", dt::tsv_to_csv),
                };
                self.transform_text(title, f);
            }
            Command::InsertUuid => {
                let id = crate::datatools::uuid_v4();
                self.with_active(|d, v, _| ops::insert_text(d, v, &id, EditKind::Other));
            }
            Command::Compare => self.compare_default(),
            Command::CompareWith(a, b) => self.start_compare(a, b, Vec::new()),
            Command::CompareSelections => self.compare_selections(),
            Command::CompareBlank => self.compare_blank(),
            Command::CompareSwap => self.compare_swap(),
            Command::CompareInline => self.compare_inline(),
            Command::CompareCopyAll(to_right) => self.compare_copy_all(to_right),
            Command::CompareIgnoreFindText => {
                let what = self.find.what.clone();
                if what.is_empty() {
                    self.flash("Type a regex in the Find box first");
                } else {
                    self.settings.compare.ignore_pattern = Some(what);
                    self.recompute_compare();
                }
            }
            Command::CompareClearIgnore => {
                self.settings.compare.ignore_pattern = None;
                self.recompute_compare();
            }
            Command::CompareClipboard => self.compare_with_clipboard(),
            Command::CompareSaved => self.compare_with_saved(),
            Command::CompareClear => self.clear_compare(),
            Command::CompareRefresh => self.recompute_compare(),
            Command::CompareNext => self.compare_nav(true),
            Command::ComparePrev => self.compare_nav(false),
            Command::CompareCopyToRight => self.compare_copy(true),
            Command::CompareCopyToLeft => self.compare_copy(false),
            Command::CompareOption(i) => {
                let o = &mut self.settings.compare;
                let flag = match i {
                    0 => &mut o.ignore_whitespace,
                    1 => &mut o.ignore_case,
                    2 => &mut o.ignore_empty_lines,
                    3 => &mut o.word_diff,
                    5 => &mut o.detect_moves,
                    6 => &mut o.hide_panels,
                    _ => &mut o.auto,
                };
                *flag = !*flag;
                self.recompute_compare();
            }
            Command::Format(op) => self.format_command(op),
            Command::RunDialog => self.dialogs.run = Some(String::new()),
            Command::RunSaved(i) => {
                if let Some((_, c)) = self.settings.run_commands.get(i).cloned() {
                    let line = self.expand_vars(&c);
                    if let Err(e) = crate::platform::run_shell(&line) {
                        self.message("Run", &e.to_string());
                    }
                }
            }
            // ------------------------------------------------ Window / help
            Command::WindowsDialog => self.dialogs.windows = true,
            Command::SortTabs(by) => {
                let docs = &self.docs;
                let p = &mut self.panes[pane];
                let active = p.active_doc();
                let key = |id: &DocId| -> String {
                    let d = docs.get(id);
                    match by {
                        TabSort::NameAsc | TabSort::NameDesc => d
                            .map(Document::display_name)
                            .unwrap_or_default()
                            .to_lowercase(),
                        TabSort::PathAsc | TabSort::PathDesc => d
                            .map(Document::full_path_or_name)
                            .unwrap_or_default()
                            .to_lowercase(),
                        TabSort::TypeAsc | TabSort::TypeDesc => d
                            .map(|d| {
                                d.display_name()
                                    .rsplit_once('.')
                                    .map(|x| x.1.to_lowercase())
                                    .unwrap_or_default()
                            })
                            .unwrap_or_default(),
                    }
                };
                p.tabs.sort_by_key(key);
                if matches!(
                    by,
                    TabSort::NameDesc | TabSort::PathDesc | TabSort::TypeDesc
                ) {
                    p.tabs.reverse();
                }
                if let Some(a) = active {
                    p.active = p.tabs.iter().position(|&t| t == a).unwrap_or(0);
                }
            }
            Command::About => self.dialogs.about = true,
            Command::CommandLineHelp => self.dialogs.command_line = true,
            Command::DebugInfo => self.dialogs.debug_info = true,
        }
    }

    /// Replaces every non-empty selection with `f(text)`, or the whole
    /// document when nothing is selected. Errors are reported in a message box.
    fn transform_text(&mut self, title: &str, f: impl Fn(&str) -> Result<String, String>) -> bool {
        let res = self.with_active(|d, v, _| {
            if d.read_only {
                return Err("The document is read-only.".to_string());
            }
            let ranges: Vec<(usize, usize)> = if v.sels.iter().any(|s| !s.is_empty()) {
                v.sels
                    .iter()
                    .filter(|s| !s.is_empty())
                    .map(|s| (s.start(), s.end()))
                    .collect()
            } else {
                vec![(0, d.len_chars())]
            };
            let whole = ranges == [(0, d.len_chars())];
            let mut edits = Vec::new();
            for (a, b) in ranges {
                edits.push((a, b, f(&d.slice(a, b))?));
            }
            let before = v.sels.clone();
            let caret = v.primary().head;
            v.sels = d.edit(edits, &before, EditKind::Other, |fin| {
                if whole {
                    vec![Selection::caret(caret.min(fin[0].1))]
                } else {
                    fin.iter().map(|&(s, e)| Selection::new(s, e)).collect()
                }
            });
            v.after_edit(d);
            Ok(())
        });
        match res {
            Some(Err(e)) => {
                self.message(title, &e);
                false
            }
            Some(Ok(())) => true,
            None => false,
        }
    }

    fn mime_command(&mut self, op: MimeOp) {
        use crate::formatters as f;
        let by_line =
            |s: &str, g: &dyn Fn(&str) -> Result<String, String>| -> Result<String, String> {
                let eol = if s.contains("\r\n") { "\r\n" } else { "\n" };
                s.split(eol)
                    .map(g)
                    .collect::<Result<Vec<_>, _>>()
                    .map(|v| v.join(eol))
            };
        let decode = |s: &str| -> Result<String, String> {
            let bytes = f::base64_decode(s)?;
            String::from_utf8(bytes)
                .map_err(|_| "The decoded data is binary, not UTF-8 text.".to_string())
        };
        let (title, ok) = match op {
            MimeOp::Base64Encode => (
                "Base64 Encode",
                self.transform_text("Base64 Encode", |s| {
                    Ok(f::base64_encode(s.as_bytes(), false))
                }),
            ),
            MimeOp::Base64EncodeByLine => (
                "Base64 Encode",
                self.transform_text("Base64 Encode", |s| {
                    by_line(s, &|l| Ok(f::base64_encode(l.as_bytes(), false)))
                }),
            ),
            MimeOp::Base64UrlEncode => (
                "Base64 URL-safe Encode",
                self.transform_text("Base64 URL-safe Encode", |s| {
                    Ok(f::base64_encode(s.as_bytes(), true))
                }),
            ),
            MimeOp::Base64Decode => (
                "Base64 Decode",
                self.transform_text("Base64 Decode", decode),
            ),
            MimeOp::Base64DecodeByLine => (
                "Base64 Decode",
                self.transform_text("Base64 Decode", |s| by_line(s, &decode)),
            ),
            MimeOp::UrlEncode => (
                "URL Encode",
                self.transform_text("URL Encode", |s| Ok(f::url_encode(s))),
            ),
            MimeOp::UrlDecode => (
                "URL Decode",
                self.transform_text("URL Decode", |s| f::url_decode(s, false)),
            ),
            MimeOp::UrlDecodeForm => (
                "URL Decode",
                self.transform_text("URL Decode", |s| f::url_decode(s, true)),
            ),
            MimeOp::QpEncode => (
                "Quoted-printable Encode",
                self.transform_text("Quoted-printable Encode", |s| Ok(f::qp_encode(s))),
            ),
            MimeOp::QpDecode => (
                "Quoted-printable Decode",
                self.transform_text("Quoted-printable Decode", f::qp_decode),
            ),
        };
        if ok {
            self.flash(format!("{title}: done"));
        }
    }

    fn format_command(&mut self, op: FormatOp) {
        use crate::formatters as f;
        let opts = self.settings.format.clone();
        let op = if op == FormatOp::Auto {
            let lang = self
                .active_doc()
                .map(|d| self.langs.langs[d.lang].name.clone())
                .unwrap_or_default();
            let first = self
                .active_doc()
                .map(|d| d.slice(0, 200))
                .unwrap_or_default();
            let t = first.trim_start();
            match lang.as_str() {
                "json" => FormatOp::JsonPretty,
                "xml" => FormatOp::XmlPretty,
                "html" | "php" => FormatOp::HtmlPretty,
                "css" => FormatOp::CssPretty,
                "javascript" | "typescript" => FormatOp::JsPretty,
                "markdown" => FormatOp::MarkdownPretty,
                _ if t.starts_with('{') || t.starts_with('[') => FormatOp::JsonPretty,
                _ if t.to_ascii_lowercase().starts_with("<!doctype html")
                    || t.starts_with("<html") =>
                {
                    FormatOp::HtmlPretty
                }
                _ if t.starts_with('<') => FormatOp::XmlPretty,
                _ => {
                    self.message(
                        "Format Document",
                        "There's no formatter for this language.\nUse Tools > Format to pick one.",
                    );
                    return;
                }
            }
        } else {
            op
        };
        let (title, ok) = match op {
            FormatOp::JsonPretty => (
                "JSON Format",
                self.transform_text("JSON Format", |s| f::json_format(s, &opts)),
            ),
            FormatOp::JsonMinify => (
                "JSON Minify",
                self.transform_text("JSON Minify", f::json_minify),
            ),
            FormatOp::XmlPretty => (
                "XML Pretty Print",
                self.transform_text("XML Pretty Print", |s| f::xml_format(s, &opts)),
            ),
            FormatOp::XmlMinify => (
                "XML Minify",
                self.transform_text("XML Minify", |s| f::xml_minify(s, &opts)),
            ),
            FormatOp::CssPretty => (
                "CSS Format",
                self.transform_text("CSS Format", |s| {
                    crate::formatters_web::css_format(s, &opts)
                }),
            ),
            FormatOp::CssMinify => (
                "CSS Minify",
                self.transform_text("CSS Minify", crate::formatters_web::css_minify),
            ),
            FormatOp::HtmlPretty => (
                "HTML Format",
                self.transform_text("HTML Format", |s| {
                    crate::formatters_web::html_format(s, &opts)
                }),
            ),
            FormatOp::JsPretty => (
                "JavaScript Format",
                self.transform_text("JavaScript Format", |s| {
                    crate::formatters_web::js_format(s, &opts)
                }),
            ),
            FormatOp::MarkdownPretty => (
                "Markdown Format",
                self.transform_text("Markdown Format", |s| {
                    crate::formatters_web::markdown_format(s, &opts)
                }),
            ),
            FormatOp::JsonValidate | FormatOp::XmlValidate => {
                let json = op == FormatOp::JsonValidate;
                let text = self.with_active(|d, v, _| {
                    let p = v.primary();
                    if p.is_empty() {
                        d.text().to_string()
                    } else {
                        d.slice(p.start(), p.end())
                    }
                });
                if let Some(t) = text {
                    let r = if json {
                        f::json_validate(&t)
                    } else {
                        f::xml_validate(&t)
                    };
                    let title = if json {
                        "JSON Validation"
                    } else {
                        "XML Syntax Check"
                    };
                    match r {
                        Ok(()) => self.message(title, "No error detected."),
                        Err(e) => self.message(title, &e),
                    }
                }
                return;
            }
            FormatOp::Auto => return,
        };
        if ok {
            // Plain-text documents adopt the matching language.
            let target = if matches!(op, FormatOp::JsonPretty | FormatOp::JsonMinify) {
                "json"
            } else {
                "xml"
            };
            if let (Some(l), Some(id)) = (self.langs.by_name(target), self.active_id())
                && let Some(d) = self.docs.get_mut(&id)
                && d.lang == self.langs.normal()
            {
                d.set_language(l, &self.langs);
            }
            self.flash(format!("{title}: done"));
        }
    }

    fn hash_command(&mut self, algo: HashAlgo, source: HashSource) {
        match source {
            HashSource::Text => {
                self.dialogs.hash = Some(crate::dialogs::HashDialog::new(algo, false))
            }
            HashSource::Files => {
                let mut d = crate::dialogs::HashDialog::new(algo, true);
                if let Some(files) = rfd::FileDialog::new().pick_files() {
                    d.output = files
                        .iter()
                        .map(|f| {
                            let h = std::fs::read(f)
                                .map(|b| textops::hex_digest(algo, &b))
                                .unwrap_or_else(|e| e.to_string());
                            format!("{h}  {}", f.display())
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                }
                self.dialogs.hash = Some(d);
            }
            HashSource::SelectionToClipboard => {
                if let Some(texts) = self.with_active(|d, v, _| v.selected_texts(d)) {
                    let out: Vec<String> = texts
                        .iter()
                        .map(|t| textops::hex_digest(algo, t.as_bytes()))
                        .collect();
                    if !out.is_empty() {
                        let joined = out.join("\n");
                        self.clip_set(&joined);
                        self.flash(format!("{} copied to clipboard", algo.name()));
                    }
                }
            }
        }
    }

    pub fn step_search_result(&mut self, forward: bool) {
        let Some(r) = &self.results else { return };
        let flat: Vec<(usize, usize)> = r
            .files
            .iter()
            .enumerate()
            .flat_map(|(fi, f)| (0..f.hits.len()).map(move |hi| (fi, hi)))
            .collect();
        if flat.is_empty() {
            return;
        }
        let cur = self
            .results_cursor
            .and_then(|c| flat.iter().position(|&x| x == c));
        let next = match cur {
            None => 0,
            Some(i) if forward => (i + 1) % flat.len(),
            Some(i) => (i + flat.len() - 1) % flat.len(),
        };
        self.results_cursor = Some(flat[next]);
        self.goto_result(flat[next].0, flat[next].1);
    }

    pub fn goto_result(&mut self, fi: usize, hi: usize) {
        let Some(r) = &self.results else { return };
        let Some(f) = r.files.get(fi) else { return };
        let Some(hit) = f.hits.get(hi) else { return };
        let (line, range) = (hit.line, hit.ranges.first().copied().unwrap_or((0, 0)));
        let hit_text = hit.text.clone();
        let target = if let Some(id) = f.doc_id.filter(|id| self.docs.contains_key(id)) {
            let pane = self
                .panes
                .iter()
                .position(|p| p.tabs.contains(&id))
                .unwrap_or(0);
            self.activate(pane, id);
            Some(id)
        } else if let Some(p) = f.path.clone() {
            self.open_path(&p, self.focused)
        } else {
            None
        };
        self.results_cursor = Some((fi, hi));
        if let Some(id) = target {
            let pane = self.focused;
            if let (Some(d), Some(vs)) = (self.docs.get_mut(&id), self.views.get_mut(&(pane, id))) {
                let l = line.min(d.line_count().saturating_sub(1));
                d.ensure_line_visible(l);
                let ls = d.line_start(l);
                let cs = hit_text[..range.0.min(hit_text.len())].chars().count();
                let ce = hit_text[..range.1.min(hit_text.len())].chars().count();
                vs.set_selection(ls + cs.min(d.line_len(l)), ls + ce.min(d.line_len(l)));
                vs.center_caret = true;
            }
        }
    }

    /// Find All in Current / All Opened Documents.
    pub fn find_all_docs(&mut self, all_docs: bool) {
        let opts = self.settings.search_opts.clone();
        let what = self.find.what.clone();
        let re = match search::build_regex(&what, &opts) {
            Ok(r) => r,
            Err(e) => {
                self.find.status = Some((
                    format!("Find: Invalid regular expression: {e}"),
                    StatusKind::NotFound,
                ));
                return;
            }
        };
        let ids: Vec<DocId> = if all_docs {
            let mut v: Vec<DocId> = self.all_doc_ids().into_iter().map(|x| x.1).collect();
            v.dedup();
            v
        } else {
            self.active_id().into_iter().collect()
        };
        let mut results = SearchResults {
            query: what.clone(),
            files: Vec::new(),
            searched_files: ids.len(),
        };
        for id in ids {
            let Some(d) = self.docs.get(&id) else {
                continue;
            };
            let text = d.text();
            let matches = search::find_all(&re, &text);
            if matches.is_empty() {
                continue;
            }
            results.files.push(search::FileHits {
                path: d.path.clone(),
                doc_id: Some(id),
                title: d.full_path_or_name(),
                hits: search::hits_by_line(&text, &matches),
            });
        }
        self.find.status = Some((
            format!(
                "Find All: {} hits in {} documents",
                results.total_hits(),
                results.files.len()
            ),
            StatusKind::Info,
        ));
        self.results = Some(results);
        self.results_visible = true;
        self.results_cursor = None;
        self.push_history();
    }

    pub fn start_find_in_files(&mut self, replace: bool) {
        if self.find.what.is_empty() || self.find.fif_dir.is_empty() {
            return;
        }
        if let Some(f) = &self.fif {
            f.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let params = search::FifParams {
            pattern: self.find.what.clone(),
            replace: replace.then(|| self.find.with.clone()),
            dir: PathBuf::from(&self.find.fif_dir),
            filters: self.find.fif_filters.clone(),
            recursive: self.find.fif_recursive,
            hidden: self.find.fif_hidden,
            opts: self.settings.search_opts.clone(),
        };
        let ctx = self.egui_ctx.clone();
        search::spawn_find_in_files(params, tx, cancel.clone(), move || {
            if let Some(c) = &ctx {
                c.request_repaint();
            }
        });
        self.fif = Some(FifRunning {
            rx,
            cancel,
            progress: 0,
            replace,
            replaced: 0,
        });
        if !replace {
            self.results = Some(SearchResults {
                query: self.find.what.clone(),
                files: Vec::new(),
                searched_files: 0,
            });
            self.results_visible = true;
            self.results_cursor = None;
        }
        let dir = self.find.fif_dir.clone();
        self.settings.fif_dirs.retain(|d| *d != dir);
        self.settings.fif_dirs.insert(0, dir);
        self.settings.fif_dirs.truncate(10);
        let filt = self.find.fif_filters.clone();
        self.settings.fif_filters.retain(|d| *d != filt);
        self.settings.fif_filters.insert(0, filt);
        self.settings.fif_filters.truncate(10);
        self.push_history();
        self.find.status = Some(("Find in Files: searching...".into(), StatusKind::Info));
    }

    /// Replace (single): replaces the current match then finds the next one.
    pub fn replace_one(&mut self) {
        let opts = self.settings.search_opts.clone();
        let what = self.find.what.clone();
        let with = self.find.with.clone();
        let Ok(re) = search::build_regex(&what, &opts) else {
            self.find.status = Some((
                "Replace: Invalid regular expression".into(),
                StatusKind::NotFound,
            ));
            return;
        };
        let replaced = self.with_active(|d, v, _| {
            if d.read_only {
                return false;
            }
            let p = v.primary();
            if p.is_empty() {
                return false;
            }
            let text = d.text();
            let (bs, be) = (d.char_to_byte(p.start()), d.char_to_byte(p.end()));
            match re.find_from_pos(text.as_str(), bs) {
                Ok(Some(m)) if m.start() == bs && m.end() == be => {
                    let rep = search::expand_replacement(&re, &text, (bs, be), &with, opts.mode);
                    let before = v.sels.clone();
                    v.sels = d.edit(
                        vec![(p.start(), p.end(), rep)],
                        &before,
                        EditKind::Other,
                        |f| vec![Selection::caret(f[0].1)],
                    );
                    v.after_edit(d);
                    true
                }
                _ => false,
            }
        });
        self.push_history();
        let found = self.find_in_active(&what, self.find.backward, &opts, true);
        if replaced == Some(true) && !found {
            self.find.status = Some((
                "Replace: 1 occurrence was replaced. No more occurrences.".into(),
                StatusKind::Info,
            ));
        }
    }

    /// Replace All in the current document (or selection) / all documents.
    pub fn replace_all(&mut self, all_docs: bool) {
        let opts = self.settings.search_opts.clone();
        let what = self.find.what.clone();
        let with = self.find.with.clone();
        let in_sel = self.find.in_selection && !all_docs;
        let Ok(re) = search::build_regex(&what, &opts) else {
            self.find.status = Some((
                "Replace All: Invalid regular expression".into(),
                StatusKind::NotFound,
            ));
            return;
        };
        let ids: Vec<DocId> = if all_docs {
            let mut v: Vec<DocId> = self.all_doc_ids().into_iter().map(|x| x.1).collect();
            v.sort_unstable();
            v.dedup();
            v
        } else {
            self.active_id().into_iter().collect()
        };
        let mut total = 0;
        for id in ids {
            let pane = self
                .panes
                .iter()
                .position(|p| p.tabs.contains(&id))
                .unwrap_or(0);
            let (Some(d), Some(v)) = (self.docs.get_mut(&id), self.views.get_mut(&(pane, id)))
            else {
                continue;
            };
            if d.read_only {
                continue;
            }
            v.sync(d);
            let text = d.text();
            let (start, end) = if in_sel {
                let p = v.primary();
                (p.start(), p.end())
            } else {
                (0, d.len_chars())
            };
            let (bs, be) = (d.char_to_byte(start), d.char_to_byte(end));
            let region = &text[bs..be];
            // Run on the region but keep look-around context by searching the full text.
            let matches: Vec<(usize, usize)> = search::find_all(&re, &text)
                .into_iter()
                .filter(|&(s, e)| s >= bs && e <= be)
                .collect();
            if matches.is_empty() {
                continue;
            }
            let mut out = String::with_capacity(region.len());
            let mut last = bs;
            for &m in &matches {
                out.push_str(&text[last..m.0]);
                out.push_str(&search::expand_replacement(&re, &text, m, &with, opts.mode));
                last = m.1;
            }
            out.push_str(&text[last..be]);
            total += matches.len();
            let before = v.sels.clone();
            let caret = v.primary().head;
            v.sels = d.edit(vec![(start, end, out)], &before, EditKind::Other, |f| {
                if in_sel {
                    vec![Selection::new(f[0].0, f[0].1)]
                } else {
                    vec![Selection::caret(caret.min(f[0].1))]
                }
            });
            v.after_edit(d);
        }
        self.push_history();
        self.find.status = Some((
            format!(
                "Replace All: {total} occurrence{} replaced in {}",
                if total == 1 { " was" } else { "s were" },
                if all_docs {
                    "all opened documents"
                } else if in_sel {
                    "selected text"
                } else {
                    "entire file"
                }
            ),
            if total == 0 {
                StatusKind::NotFound
            } else {
                StatusKind::Info
            },
        ));
    }

    pub fn count_matches(&mut self) {
        let opts = self.settings.search_opts.clone();
        let what = self.find.what.clone();
        let in_sel = self.find.in_selection;
        let Ok(re) = search::build_regex(&what, &opts) else {
            return;
        };
        let n = self.with_active(|d, v, _| {
            let text = d.text();
            let all = search::find_all(&re, &text);
            if in_sel {
                let p = v.primary();
                let (bs, be) = (d.char_to_byte(p.start()), d.char_to_byte(p.end()));
                all.iter().filter(|&&(s, e)| s >= bs && e <= be).count()
            } else {
                all.len()
            }
        });
        self.push_history();
        self.find.status = Some((
            format!(
                "Count: {} match{} in {}",
                n.unwrap_or(0),
                if n == Some(1) { "" } else { "es" },
                if in_sel {
                    "selected text"
                } else {
                    "entire file"
                }
            ),
            StatusKind::Info,
        ));
    }

    pub fn mark_all(&mut self) {
        let opts = self.settings.search_opts.clone();
        let what = self.find.what.clone();
        let (bookmark, purge, in_sel) = (
            self.find.mark_bookmark,
            self.find.mark_purge,
            self.find.in_selection,
        );
        let Ok(re) = search::build_regex(&what, &opts) else {
            self.find.status = Some((
                "Mark: Invalid regular expression".into(),
                StatusKind::NotFound,
            ));
            return;
        };
        let n = self.with_active(|d, v, _| {
            if purge {
                d.marks.retain(|m| m.style != 0);
            }
            let text = d.text();
            let p = v.primary();
            let (bs, be) = if in_sel {
                (d.char_to_byte(p.start()), d.char_to_byte(p.end()))
            } else {
                (0, text.len())
            };
            let mut count = 0;
            for (s, e) in search::find_all(&re, &text) {
                if s < bs || e > be {
                    continue;
                }
                let (cs, ce) = (d.byte_to_char(s), d.byte_to_char(e));
                d.marks.push(Mark {
                    start: cs,
                    end: ce,
                    style: 0,
                });
                if bookmark {
                    let l = d.char_to_line(cs);
                    if let Some(m) = d.meta.get_mut(l) {
                        m.bookmark = true;
                    }
                }
                count += 1;
            }
            d.marks.sort_by_key(|m| (m.start, m.end));
            count
        });
        self.push_history();
        self.find.status = Some((
            format!(
                "Mark: {} match{} in entire file",
                n.unwrap_or(0),
                if n == Some(1) { "" } else { "es" }
            ),
            StatusKind::Info,
        ));
    }

    pub fn clear_marks(&mut self) {
        if let Some(id) = self.active_id()
            && let Some(d) = self.docs.get_mut(&id)
        {
            d.marks.retain(|m| m.style != 0);
        }
    }

    /// Incremental search: re-run from the origin as the text changes.
    pub fn incremental_update(&mut self, forward: Option<bool>) {
        let text = self.incremental.text.clone();
        let opts = search::SearchOptions {
            match_case: self.incremental.match_case,
            wrap: true,
            ..Default::default()
        };
        if let Some(id) = self.active_id()
            && let Some(d) = self.docs.get_mut(&id)
        {
            d.marks.retain(|m| m.style != 6);
        }
        if text.is_empty() {
            self.incremental.not_found = false;
            return;
        }
        if forward.is_none()
            && let Some(o) = self.incremental.origin
        {
            self.with_active(|_, v, _| v.set_single(o));
        }
        let found = self.find_in_active(&text, forward == Some(false), &opts, false);
        self.incremental.not_found = !found;
        if self.incremental.highlight_all
            && let Ok(re) = search::build_regex(&text, &opts)
        {
            self.with_active(|d, _, _| {
                let t = d.text();
                for (s, e) in search::find_all(&re, &t) {
                    let (cs, ce) = (d.byte_to_char(s), d.byte_to_char(e));
                    d.marks.push(Mark {
                        start: cs,
                        end: ce,
                        style: 6,
                    });
                }
                d.marks.sort_by_key(|m| (m.start, m.end));
            });
        }
    }

    pub fn sort_mode_label(m: SortMode) -> &'static str {
        match m {
            SortMode::LexAsc => "Sort Lines Lexicographically Ascending",
            SortMode::LexDesc => "Sort Lines Lexicographically Descending",
            SortMode::LexCiAsc => "Sort Lines Lex. Ascending Ignoring Case",
            SortMode::LexCiDesc => "Sort Lines Lex. Descending Ignoring Case",
            SortMode::LocaleAsc => "Sort Lines In Locale Order Ascending",
            SortMode::LocaleDesc => "Sort Lines In Locale Order Descending",
            SortMode::IntAsc => "Sort Lines As Integers Ascending",
            SortMode::IntDesc => "Sort Lines As Integers Descending",
            SortMode::DecCommaAsc => "Sort Lines As Decimals (Comma) Ascending",
            SortMode::DecCommaDesc => "Sort Lines As Decimals (Comma) Descending",
            SortMode::DecDotAsc => "Sort Lines As Decimals (Dot) Ascending",
            SortMode::DecDotDesc => "Sort Lines As Decimals (Dot) Descending",
            SortMode::LengthAsc => "Sort Lines By Length Ascending",
            SortMode::LengthDesc => "Sort Lines By Length Descending",
            SortMode::Reverse => "Reverse Line Order",
            SortMode::Random => "Randomize Line Order",
        }
    }

    pub fn case_label(m: CaseMode) -> &'static str {
        match m {
            CaseMode::Upper => "UPPERCASE",
            CaseMode::Lower => "lowercase",
            CaseMode::ProperForce => "Proper Case",
            CaseMode::ProperBlend => "Proper Case (blend)",
            CaseMode::SentenceForce => "Sentence case",
            CaseMode::SentenceBlend => "Sentence case (blend)",
            CaseMode::Invert => "iNVERT cASE",
            CaseMode::Random => "ranDOm CasE",
        }
    }
}

/// Deletes a set of whole lines in one undo step.
fn delete_line_set(doc: &mut Document, vs: &mut ViewState, lines: &[usize]) {
    if lines.is_empty() || doc.read_only {
        return;
    }
    let n = doc.line_count();
    let before = vs.sels.clone();
    let mut edits = Vec::new();
    for &l in lines {
        let s = doc.line_start(l);
        let e = if l + 1 < n {
            doc.line_start(l + 1)
        } else {
            doc.len_chars()
        };
        let s = if l + 1 >= n && l > 0 {
            doc.line_end(l - 1)
        } else {
            s
        };
        edits.push((s, e, String::new()));
    }
    // Merge overlapping (last-line case).
    edits.sort_by_key(|e| e.0);
    let mut merged: Vec<(usize, usize, String)> = Vec::new();
    for e in edits {
        if let Some(last) = merged.last_mut()
            && e.0 <= last.1
        {
            last.1 = last.1.max(e.1);
            continue;
        }
        merged.push(e);
    }
    vs.sels = doc.edit(merged, &before, EditKind::Other, |f| {
        vec![Selection::caret(f.first().map_or(0, |x| x.0))]
    });
    vs.after_edit(doc);
}
