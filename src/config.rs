//! Persistent settings and session (settings and the open-files session
//! equivalents, stored as JSON).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::encoding::Eol;
use crate::search::SearchOptions;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)]
pub struct Settings {
    /// Theme in use; while following the system setting, the light theme.
    pub theme: String,
    /// Switch between `theme` and `dark_theme` with the OS light/dark setting.
    pub follow_system_theme: bool,
    pub dark_theme: String,
    pub font_size: f32,
    pub ui_scale: f32,
    pub tab_size: usize,
    pub use_spaces: bool,
    pub auto_indent: bool,
    pub word_wrap: bool,
    /// Colour code by language. Off by default: plain text is the fastest view.
    pub syntax_highlighting: bool,
    pub show_line_numbers: bool,
    pub show_bookmark_margin: bool,
    pub show_fold_margin: bool,
    pub show_change_history: bool,
    pub show_whitespace: bool,
    pub show_eol: bool,
    pub show_indent_guides: bool,
    pub show_wrap_symbol: bool,
    pub highlight_current_line: bool,
    pub smart_highlight: bool,
    pub smart_match_case: bool,
    pub smart_whole_word: bool,
    pub brace_highlight: bool,
    /// 0 = no vertical edge.
    pub edge_column: usize,
    pub caret_width: f32,
    pub caret_blink: bool,
    pub auto_complete: bool,
    pub auto_complete_min_chars: usize,
    pub auto_close_brackets: bool,
    pub auto_close_quotes: bool,
    pub remember_session: bool,
    pub backup_unsaved: bool,
    pub backup_on_save: bool,
    pub default_eol: Eol,
    pub default_encoding: String,
    pub default_lang: String,
    pub file_monitoring: bool,
    pub update_silently: bool,
    pub scroll_to_end_on_update: bool,
    pub recent_max: usize,
    pub tab_close_button: bool,
    pub double_click_close_tab: bool,
    pub show_toolbar: bool,
    pub show_statusbar: bool,
    pub show_tabbar: bool,
    pub date_time_format: String,
    pub search_engine: String,
    pub copy_line_without_selection: bool,
    pub single_instance: bool,
    pub show_function_list: bool,
    pub show_json_tree: bool,
    /// The Find dialog is a panel at the bottom instead of a window.
    pub find_docked: bool,
    /// Look for a newer release on GitHub when the app starts.
    pub check_for_updates: bool,
    pub show_doc_list: bool,
    pub show_folder_tree: bool,
    pub show_doc_map: bool,
    pub show_char_panel: bool,
    pub show_clipboard_history: bool,
    pub sync_scroll_v: bool,
    pub sync_scroll_h: bool,
    pub search_opts: SearchOptions,
    pub find_history: Vec<String>,
    pub replace_history: Vec<String>,
    pub fif_dirs: Vec<String>,
    pub fif_filters: Vec<String>,
    pub workspace_folders: Vec<String>,
    pub smooth_scrolling: bool,
    pub virtual_space: bool,
    pub multi_editing: bool,
    pub confirm_reload: bool,
    pub format: crate::formatters::FormatOptions,
    pub compare: crate::compare::CompareOptions,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: crate::theme::DEFAULT_THEME.into(),
            follow_system_theme: true,
            dark_theme: crate::theme::DEFAULT_DARK_THEME.into(),
            font_size: 13.0,
            ui_scale: 1.0,
            tab_size: 4,
            use_spaces: false,
            auto_indent: true,
            word_wrap: true,
            syntax_highlighting: false,
            show_line_numbers: true,
            show_bookmark_margin: true,
            show_fold_margin: true,
            show_change_history: true,
            show_whitespace: false,
            show_eol: false,
            show_indent_guides: true,
            show_wrap_symbol: false,
            highlight_current_line: true,
            smart_highlight: true,
            smart_match_case: false,
            smart_whole_word: true,
            brace_highlight: true,
            edge_column: 0,
            caret_width: 1.5,
            caret_blink: true,
            auto_complete: true,
            auto_complete_min_chars: 3,
            auto_close_brackets: false,
            auto_close_quotes: false,
            remember_session: true,
            backup_unsaved: true,
            backup_on_save: false,
            default_eol: Eol::platform_default(),
            default_encoding: "utf8".into(),
            default_lang: "normal".into(),
            file_monitoring: true,
            update_silently: false,
            scroll_to_end_on_update: false,
            recent_max: 15,
            tab_close_button: true,
            double_click_close_tab: false,
            show_toolbar: true,
            show_statusbar: true,
            show_tabbar: true,
            date_time_format: "yyyy-MM-dd HH:mm:ss".into(),
            search_engine: DEFAULT_SEARCH_ENGINE.into(),
            copy_line_without_selection: true,
            single_instance: true,
            show_function_list: false,
            show_json_tree: false,
            find_docked: false,
            check_for_updates: true,
            show_doc_list: false,
            show_folder_tree: false,
            show_doc_map: false,
            show_char_panel: false,
            show_clipboard_history: false,
            sync_scroll_v: true,
            sync_scroll_h: true,
            search_opts: SearchOptions {
                wrap: true,
                ..Default::default()
            },
            find_history: Vec::new(),
            replace_history: Vec::new(),
            fif_dirs: Vec::new(),
            fif_filters: vec!["*.*".into()],
            workspace_folders: Vec::new(),
            smooth_scrolling: true,
            virtual_space: false,
            multi_editing: true,
            confirm_reload: true,
            format: crate::formatters::FormatOptions::default(),
            compare: crate::compare::CompareOptions::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionFile {
    pub path: Option<String>,
    /// Snapshot of unsaved content, restored on the next start.
    pub backup: Option<String>,
    pub untitled_name: String,
    pub lang: String,
    pub encoding: String,
    pub eol: Option<Eol>,
    pub anchor: usize,
    pub caret: usize,
    pub top_row: f64,
    pub bookmarks: Vec<usize>,
    pub folds: Vec<usize>,
    pub read_only: bool,
    pub view: u8,
    pub tab_color: Option<u8>,
    pub pinned: bool,
    pub dirty: bool,
}

/// Window position and size in points, restored with the session.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub window: Option<WindowGeometry>,
    /// Compare options of this session (each `--session-dir` keeps its own).
    pub compare: Option<crate::compare::CompareOptions>,
    pub files: Vec<SessionFile>,
    pub active: [Option<usize>; 2],
    pub focused_view: u8,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Persisted {
    pub recent_files: Vec<String>,
}

pub fn config_dir() -> PathBuf {
    if let Ok(p) = std::env::var("SUPERFASTNOTEPAD_CONFIG_DIR") {
        return PathBuf::from(p);
    }
    directories::ProjectDirs::from("org", "superfastnotepad", "superfastnotepad").map_or_else(
        || PathBuf::from(".superfastnotepad"),
        |d| d.config_dir().to_path_buf(),
    )
}

static SESSION_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Keeps the session, unsaved-content backups and the single-instance lock
/// in `dir` instead of the settings folder (`--session-dir`), so separate
/// workspaces don't share open files. Settings stay shared.
pub fn set_session_dir(dir: PathBuf) {
    let _ = SESSION_DIR.set(dir);
}

pub fn session_dir() -> PathBuf {
    SESSION_DIR.get().cloned().unwrap_or_else(config_dir)
}

pub fn backup_dir() -> PathBuf {
    session_dir().join("backup")
}

fn load_json<T: for<'de> Deserialize<'de> + Default>(name: &str) -> T {
    load_json_in(&config_dir(), name)
}

fn load_json_in<T: for<'de> Deserialize<'de> + Default>(dir: &std::path::Path, name: &str) -> T {
    std::fs::read_to_string(dir.join(name))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_json<T: Serialize>(name: &str, value: &T) {
    save_json_in(&config_dir(), name, value);
}

fn save_json_in<T: Serialize>(dir: &std::path::Path, name: &str, value: &T) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(s) = serde_json::to_string_pretty(value) {
        let tmp = dir.join(format!("{name}.tmp"));
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join(name));
        }
    }
}

/// Web search for selected text; `$(CURRENT_WORD)` is replaced by the text.
pub const DEFAULT_SEARCH_ENGINE: &str = "https://www.google.com/search?q=$(CURRENT_WORD)";

pub fn load_settings() -> Settings {
    let mut s: Settings = load_json("config.json");
    // Settings saved before Google became the default keep the old default.
    if s.search_engine == "https://duckduckgo.com/?q=$(CURRENT_WORD)" {
        s.search_engine = DEFAULT_SEARCH_ENGINE.into();
    }
    s
}

pub fn save_settings(s: &Settings) {
    save_json("config.json", s);
}

pub fn load_session() -> Session {
    load_json_in(&session_dir(), "session.json")
}

pub fn save_session(s: &Session) {
    save_json_in(&session_dir(), "session.json", s);
}

pub fn load_persisted() -> Persisted {
    load_json("persisted.json")
}

pub fn save_persisted(p: &Persisted) {
    save_json("persisted.json", p);
}

/// Converts a Windows style date format (yyyy-MM-dd HH:mm:ss) into chrono's.
pub fn format_date(fmt: &str, now: chrono::DateTime<chrono::Local>) -> String {
    let map = [
        ("yyyy", "%Y"),
        ("yy", "%y"),
        ("MMMM", "%B"),
        ("MMM", "%b"),
        ("MM", "%m"),
        ("dddd", "%A"),
        ("ddd", "%a"),
        ("dd", "%d"),
        ("HH", "%H"),
        ("hh", "%I"),
        ("mm", "%M"),
        ("ss", "%S"),
        ("tt", "%p"),
    ];
    let mut out = String::new();
    let mut rest = fmt;
    'outer: while !rest.is_empty() {
        for (k, v) in map {
            if let Some(r) = rest.strip_prefix(k) {
                out.push_str(v);
                rest = r;
                continue 'outer;
            }
        }
        let c = rest.chars().next().unwrap();
        if c == '%' {
            out.push_str("%%");
        } else {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    now.format(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn date_format_conversion() {
        let t = chrono::Local.with_ymd_and_hms(2026, 3, 4, 5, 6, 7).unwrap();
        assert_eq!(format_date("yyyy-MM-dd HH:mm:ss", t), "2026-03-04 05:06:07");
        assert_eq!(format_date("dd/MM/yy 100%", t), "04/03/26 100%");
    }

    #[test]
    fn settings_roundtrip_with_missing_fields() {
        let s: Settings = serde_json::from_str("{\"tab_size\": 2}").unwrap();
        assert_eq!(s.tab_size, 2);
        assert!(s.show_line_numbers);
    }

    #[test]
    fn syntax_highlighting_is_off_by_default() {
        assert!(!Settings::default().syntax_highlighting);
        // Settings files saved before the option existed also load with it off.
        let s: Settings = serde_json::from_str("{\"tab_size\": 4}").unwrap();
        assert!(!s.syntax_highlighting);
    }
}
