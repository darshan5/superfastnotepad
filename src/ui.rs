//! Main window: menus, toolbar, tab bars, split editors and status bar.

use egui::{Align, Color32, CornerRadius, Layout, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

use crate::app::App;
use crate::commands::{Command, FormatOp, HashSource, MimeOp, TabSort, shortcut_for};
use crate::document::Document;
use crate::editor::{self, EditorEnv};
use crate::encoding::TextEncoding;
use crate::textops::{CaseMode, HashAlgo, SortMode};

pub const APP_NAME: &str = "superfastnotepad";

/// Commands still allowed while a dialog text field has keyboard focus.
fn allowed_in_text_field(c: &Command) -> bool {
    matches!(
        c,
        Command::New
            | Command::Open
            | Command::Save
            | Command::SaveAs
            | Command::SaveAll
            | Command::Close
            | Command::Find
            | Command::Replace
            | Command::FindInFiles
            | Command::Mark
            | Command::FindNext
            | Command::FindPrev
            | Command::NextTab
            | Command::PrevTab
            | Command::FullScreen
            | Command::SearchResultsWindow
            | Command::NextSearchResult
            | Command::PrevSearchResult
            | Command::Exit
            | Command::GoTo
            | Command::IncrementalSearch
    )
}

pub const TAB_COLORS: [Color32; 5] = [
    Color32::from_rgb(0xE9, 0xB2, 0x3A),
    Color32::from_rgb(0x5D, 0xB8, 0x5D),
    Color32::from_rgb(0x4A, 0x9B, 0xE8),
    Color32::from_rgb(0xE8, 0x7B, 0x4A),
    Color32::from_rgb(0xB0, 0x6B, 0xD8),
];

/// The right-click menu of a tab. `compare` is the active tab when this one
/// isn't it. `out(cmd, true)` runs a command on this tab; `false` as is.
fn tab_context_menu(
    ui: &mut Ui,
    d: &Document,
    compare: Option<crate::document::DocId>,
    out: &mut dyn FnMut(Command, bool),
) {
    // Shared (not `&mut`) so the submenus can use it too.
    let picked = std::cell::RefCell::new(None);
    let item = |ui: &mut Ui, on: bool, label: &str, cmd: Command| {
        if ui.button(gutter(on, label)).clicked() {
            *picked.borrow_mut() = Some((cmd, true));
            ui.close();
        }
    };
    item(ui, false, "Close", Command::Close);
    item(ui, false, "Close All Tabs", Command::CloseAll);
    item(ui, false, "Close All but This", Command::CloseAllButCurrent);
    item(ui, false, "Close All Unchanged", Command::CloseAllUnchanged);
    ui.separator();
    item(ui, false, "Save", Command::Save);
    item(ui, false, "Save As...", Command::SaveAs);
    item(ui, false, "Rename...", Command::Rename);
    // Everything below needs a file on disk (saved at least once; it may
    // still have unsaved changes).
    if d.path.is_some() {
        item(ui, false, "Reload from Disk", Command::Reload);
        ui.separator();
        ui.menu_button(gutter(false, "Open in"), |ui| {
            let (fm, term) = if cfg!(target_os = "macos") {
                ("Finder", "Terminal")
            } else {
                ("Explorer", "Command Prompt")
            };
            item(ui, false, fm, Command::OpenContainingFolder);
            item(ui, false, term, Command::OpenTerminalHere);
            item(ui, false, "Default App", Command::OpenInDefaultViewer);
            item(ui, false, "Browser", Command::ViewInBrowser);
        });
        item(
            ui,
            false,
            "Open Folder in Workspace",
            Command::OpenFileFolderInWorkspace,
        );
        ui.menu_button(gutter(false, "Copy"), |ui| {
            item(ui, false, "Full File Path", Command::CopyFullPath);
            item(ui, false, "File Name", Command::CopyFileName);
            item(ui, false, "Folder Path", Command::CopyDirPath);
        });
    }
    ui.separator();
    if let Some(cur) = compare
        && ui
            .button(gutter(false, "Compare with Active Tab"))
            .clicked()
    {
        *picked.borrow_mut() = Some((Command::CompareWith(cur, d.id), false));
        ui.close();
    }
    item(ui, false, "Move to Other View", Command::MoveToOtherView);
    item(ui, false, "Clone to Other View", Command::CloneToOtherView);
    item(ui, false, "Move to New Window", Command::MoveToNewWindow);
    ui.separator();
    item(ui, d.read_only, "Read-Only", Command::ToggleReadOnly);
    item(ui, d.pinned, "Pinned", Command::TogglePin);
    ui.menu_button(gutter(d.tab_color.is_some(), "Tab Color"), |ui| {
        for c in 0..5u8 {
            let on = d.tab_color == Some(c);
            let txt = RichText::new(gutter(on, &format!("■ Color {}", c + 1)))
                .color(TAB_COLORS[usize::from(c)]);
            if ui.button(txt).clicked() {
                *picked.borrow_mut() = Some((Command::TabColor(Some(c)), true));
                ui.close();
            }
        }
        item(
            ui,
            d.tab_color.is_none(),
            "No Color",
            Command::TabColor(None),
        );
    });
    // Run what was clicked, here or in a submenu.
    if let Some((cmd, this_tab)) = picked.into_inner() {
        out(cmd, this_tab);
    }
}

/// One menu command, for the menu search box and the shortcut list.
#[derive(Clone, Debug)]
pub struct MenuEntry {
    /// Submenus leading to it, such as `Edit > Lines`.
    pub path: String,
    pub label: String,
    pub cmd: Command,
}

/// State of the toolbar's menu search box.
#[derive(Default)]
pub struct MenuSearch {
    pub text: String,
    /// Focus the box next frame.
    pub focus: bool,
    pub open: bool,
    pub selected: usize,
    pub entries: Vec<MenuEntry>,
    popup_rect: Option<egui::Rect>,
}

impl MenuSearch {
    /// Entries whose label and path contain every word typed; labels that
    /// start with the text come first, then labels containing it.
    pub fn matches(&self, limit: usize) -> Vec<MenuEntry> {
        let q = self.text.trim().to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        let mut hits: Vec<(u8, &MenuEntry)> = self
            .entries
            .iter()
            .filter_map(|e| {
                let label = e.label.trim().to_lowercase();
                let all = format!("{label} {}", e.path.to_lowercase());
                if !words.iter().all(|w| all.contains(w)) {
                    return None;
                }
                let rank = if label.starts_with(&q) {
                    0
                } else if label.contains(&q) {
                    1
                } else {
                    2
                };
                Some((rank, e))
            })
            .collect();
        hits.sort_by_key(|(rank, _)| *rank);
        hits.into_iter()
            .take(limit)
            .map(|(_, e)| e.clone())
            .collect()
    }
}

#[derive(Default)]
struct MenuCollector {
    path: Vec<String>,
    entries: Vec<MenuEntry>,
}

thread_local! {
    /// Set while `App::menu_entries` walks the menus.
    static COLLECTING: std::cell::RefCell<Option<MenuCollector>> =
        const { std::cell::RefCell::new(None) };
}

fn collecting() -> bool {
    COLLECTING.with(|c| c.borrow().is_some())
}

fn record(label: &str, cmd: &Command) {
    COLLECTING.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            c.entries.push(MenuEntry {
                path: c.path.join(" > "),
                label: label.to_string(),
                cmd: cmd.clone(),
            });
        }
    });
}

/// A menu (`top`) or submenu. While collecting, its contents are walked
/// directly instead of waiting for it to be opened.
fn sub(ui: &mut Ui, top: bool, on: bool, label: &str, add: impl FnOnce(&mut Ui)) {
    if collecting() {
        COLLECTING.with(|c| {
            if let Some(c) = c.borrow_mut().as_mut() {
                // "Language: Rust" is filed under "Language".
                c.path
                    .push(label.split(':').next().unwrap_or(label).to_string());
            }
        });
        add(ui);
        COLLECTING.with(|c| {
            if let Some(c) = c.borrow_mut().as_mut() {
                c.path.pop();
            }
        });
    } else if top {
        ui.menu_button(label, add);
    } else {
        ui.menu_button(gutter(on, label), add);
    }
}

/// A menu label with a fixed-width slot in front: a dot when the option is
/// on, a space otherwise, so every entry lines up (the UI font is monospaced).
fn gutter(on: bool, label: &str) -> String {
    format!("{} {label}", if on { "•" } else { " " })
}

impl App {
    pub fn editor_id(pane: usize) -> egui::Id {
        egui::Id::unique(("editor", pane))
    }

    fn sc(&self, c: &Command) -> String {
        shortcut_for(&self.shortcuts, c).unwrap_or_default()
    }

    /// Menu item that pushes a command when clicked.
    fn item(&self, ui: &mut Ui, label: &str, cmd: Command, out: &mut Vec<Command>) {
        record(label, &cmd);
        let sc = self.sc(&cmd);
        let btn = egui::Button::new(gutter(false, label)).shortcut_text(sc);
        if ui.add(btn).clicked() {
            out.push(cmd);
            ui.close();
        }
    }

    fn check(&self, ui: &mut Ui, label: &str, on: bool, cmd: Command, out: &mut Vec<Command>) {
        record(label, &cmd);
        let sc = self.sc(&cmd);
        if ui
            .add(egui::Button::new(gutter(on, label)).shortcut_text(sc))
            .clicked()
        {
            out.push(cmd);
            ui.close();
        }
    }

    // ---------------------------------------------------------------- frame

    #[allow(clippy::too_many_lines)]
    pub fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.track_window_geometry(&ctx);
        self.autosave(&ctx);
        let highlight = self.settings.syntax_highlighting;
        for d in self.docs.values_mut() {
            d.set_highlighting(highlight);
        }
        if highlight {
            // Colour the rest of the shown documents a little each frame, so
            // jumping anywhere later doesn't have to colour everything above.
            let mut more = false;
            for pane in &self.panes {
                if let Some(d) = pane.active_doc().and_then(|id| self.docs.get_mut(&id)) {
                    more |= d.lex_ahead(std::time::Duration::from_millis(2));
                }
            }
            if more {
                ctx.request_repaint();
            }
        }
        if self.egui_ctx.is_none() {
            self.egui_ctx = Some(ctx.clone());
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
        }
        let now = ctx.input(|i| i.time);
        self.poll(now);
        self.apply_pending_goto();
        self.follow_system_theme(ctx.system_theme() == Some(egui::Theme::Dark));
        ctx.set_visuals(if self.globals.is_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });

        // Window close button.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.exit_requested {
                self.request_exit();
            }
        }
        // Drag & drop of files/folders.
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        for p in dropped {
            if p.is_dir() {
                self.add_workspace_folder(p);
            } else {
                self.open_path(&p, self.focused);
            }
        }

        let mut cmds: Vec<Command> = Vec::new();
        // Global shortcuts.
        let editor_focused = ctx.memory(|m| {
            m.focused()
                .is_none_or(|f| f == Self::editor_id(0) || f == Self::editor_id(1))
        });
        let modal_open = !self.prompts.is_empty();
        if !modal_open {
            let table = self.shortcuts.clone();
            for (cmd, sc) in &table {
                if !editor_focused && !allowed_in_text_field(cmd) {
                    continue;
                }
                if ctx.input_mut(|i| i.consume_shortcut(&sc.to_egui())) {
                    cmds.push(cmd.clone());
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                if self.find.open && !editor_focused {
                    self.find.open = false;
                    self.request_focus_editor = true;
                } else if self.incremental.open && !editor_focused {
                    self.incremental.open = false;
                    self.incremental.text.clear();
                    self.incremental_update(None);
                    self.request_focus_editor = true;
                } else if self.postit {
                    self.postit = false;
                }
            }
        }

        // Window title.
        if let Some(d) = self.active_doc() {
            let title = format!(
                "{}{} - {APP_NAME}",
                if d.is_dirty() { "*" } else { "" },
                d.full_path_or_name()
            );
            // Sending a viewport command requests a new frame, so only send
            // the title when it changes (otherwise the app redraws nonstop).
            if self.window_title != title {
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
                self.window_title = title;
            }
        }

        if !self.postit {
            egui::Panel::top("menu").show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| self.menu_bar(ui, &mut cmds));
            });
            if self.settings.show_toolbar {
                egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui, &mut cmds));
            }
            if self.settings.show_statusbar {
                egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, &mut cmds));
            }
        }
        if self.incremental.open {
            egui::Panel::bottom("incremental").show(ui, |ui| self.incremental_bar(ui));
        }
        self.update_line_filter(now);
        if self.filter.open {
            egui::Panel::bottom("filter").show(ui, |ui| self.filter_bar(ui));
        }
        if self.results_visible && self.results.is_some() {
            egui::Panel::bottom("results")
                .resizable(true)
                .default_size(200.0)
                .show(ui, |ui| self.results_panel(ui));
        }
        // A docked Find panel spans the window, above the search results.
        self.find_dialog(ui, true);
        if !self.postit {
            if self.settings.show_folder_tree || self.settings.show_doc_list {
                egui::Panel::left("left_panels")
                    .resizable(true)
                    .default_size(220.0)
                    .show(ui, |ui| {
                        self.left_panels(ui);
                    });
            }
            let right = self.settings.show_function_list
                || self.settings.show_json_tree
                || self.settings.show_doc_map
                || self.settings.show_char_panel
                || self.settings.show_clipboard_history;
            if right {
                egui::Panel::right("right_panels")
                    .resizable(true)
                    .default_size(240.0)
                    .show(ui, |ui| {
                        self.right_panels(ui, &mut cmds);
                    });
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(self.globals.bg))
            .show(ui, |ui| self.editors(ui, &mut cmds));

        self.find_dialog(ui, false);
        self.dialogs_ui(&ctx, &mut cmds);
        self.prompts_ui(&ctx);

        for c in cmds {
            self.run(c);
        }
        if let Some(t) = editor::take_copy_request() {
            ctx.copy_text(t);
        }
    }

    // ---------------------------------------------------------------- menus

    /// Backs up the session and unsaved content every few seconds, but only
    /// when something changed (an edit, a tab, the window geometry).
    fn autosave(&mut self, ctx: &egui::Context) {
        const INTERVAL: f64 = 3.0;
        let now = ctx.input(|i| i.time);
        if self.exit_requested || now - self.last_autosave < INTERVAL {
            if !self.exit_requested {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(INTERVAL));
            }
            return;
        }
        self.last_autosave = now;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        {
            use std::hash::Hash;
            let mut docs: Vec<_> = self.docs.iter().map(|(id, d)| (*id, d.version)).collect();
            docs.sort_unstable();
            docs.hash(&mut h);
            for p in &self.panes {
                (&p.tabs, p.active).hash(&mut h);
            }
            self.focused.hash(&mut h);
            if let Some(g) = self.window_geometry {
                (
                    g.x.to_bits(),
                    g.y.to_bits(),
                    g.width.to_bits(),
                    g.height.to_bits(),
                    g.maximized,
                )
                    .hash(&mut h);
            }
        }
        let sig = std::hash::Hasher::finish(&h);
        if sig != self.autosave_sig {
            self.autosave_sig = sig;
            self.save_all_state();
        }
    }

    /// Remembers where the window is, so the session can restore it. While
    /// maximized or full screen, the last normal size and position are kept.
    fn track_window_geometry(&mut self, ctx: &egui::Context) {
        let vp = ctx.input(|i| i.viewport().clone());
        let maximized = vp.maximized.unwrap_or(false);
        if vp.fullscreen.unwrap_or(false) || vp.minimized.unwrap_or(false) {
            return;
        }
        if maximized {
            if let Some(g) = &mut self.window_geometry {
                g.maximized = true;
            }
            return;
        }
        if let (Some(outer), Some(inner)) = (vp.outer_rect, vp.inner_rect) {
            self.window_geometry = Some(crate::config::WindowGeometry {
                x: outer.min.x,
                y: outer.min.y,
                width: inner.width(),
                height: inner.height(),
                maximized: false,
            });
        }
    }

    /// Adds one plain menu item per (label, command).
    fn items(&self, ui: &mut Ui, list: &[(&str, Command)], out: &mut Vec<Command>) {
        for (label, cmd) in list {
            self.item(ui, label, cmd.clone(), out);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn menu_bar(&mut self, ui: &mut Ui, out: &mut Vec<Command>) {
        use Command as C;
        sub(ui, true, false, "File", |ui| {
            if let Some(u) = &self.update.available {
                let label = format!("Update Available (v{})...", u.version);
                self.item(ui, &label, C::OpenUpdatePage, out);
                ui.separator();
            }
            self.items(
                ui,
                &[
                    ("New", C::New),
                    ("New Window", C::NewWindow),
                    ("Open...", C::Open),
                    ("Open Workspace...", C::OpenFolderAsWorkspace),
                ],
                out,
            );
            let recent = !self.persisted.recent_files.is_empty();
            ui.add_enabled_ui(recent, |ui| {
                sub(ui, false, false, "Open Recent", |ui| {
                    for (i, f) in self
                        .persisted
                        .recent_files
                        .iter()
                        .take(self.settings.recent_max)
                        .enumerate()
                    {
                        if ui
                            .button(gutter(false, &format!("{}: {f}", i + 1)))
                            .clicked()
                        {
                            out.push(C::OpenRecent(f.clone()));
                            ui.close();
                        }
                    }
                    ui.separator();
                    self.items(
                        ui,
                        &[
                            ("Reopen Last Closed File", C::RestoreLastClosed),
                            ("Open All Recent Files", C::OpenAllRecent),
                            ("Clear Recent Files", C::ClearRecent),
                        ],
                        out,
                    );
                });
            });
            ui.separator();
            self.items(
                ui,
                &[
                    ("Save", C::Save),
                    ("Save As...", C::SaveAs),
                    ("Save a Copy As...", C::SaveCopyAs),
                    ("Save All", C::SaveAll),
                ],
                out,
            );
            ui.separator();
            self.items(
                ui,
                &[("Reload from Disk", C::Reload), ("Rename...", C::Rename)],
                out,
            );
            let ro = self.active_doc().is_some_and(|d| d.read_only);
            self.check(ui, "Read-Only", ro, C::ToggleReadOnly, out);
            self.item(ui, "Document Summary...", C::Summary, out);
            ui.separator();
            self.items(ui, &[("Close", C::Close), ("Close All", C::CloseAll)], out);
            sub(ui, false, false, "Close Others", |ui| {
                self.items(
                    ui,
                    &[
                        ("All but This", C::CloseAllButCurrent),
                        ("All but Pinned", C::CloseAllButPinned),
                        ("All Unchanged", C::CloseAllUnchanged),
                    ],
                    out,
                );
            });
            ui.separator();
            sub(ui, false, false, "Session", |ui| {
                self.items(
                    ui,
                    &[
                        ("Load Session...", C::LoadSession),
                        ("Save Session...", C::SaveSession),
                    ],
                    out,
                );
            });
            self.item(ui, "Settings...", C::Preferences, out);
            ui.separator();
            self.item(ui, "Exit", C::Exit, out);
        });

        sub(ui, true, false, "Edit", |ui| {
            self.items(ui, &[("Undo", C::Undo), ("Redo", C::Redo)], out);
            ui.separator();
            let cmd = if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl+"
            };
            for (l, c, k) in [
                ("Cut", C::Cut, "X"),
                ("Copy", C::Copy, "C"),
                ("Paste", C::Paste, "V"),
            ] {
                record(l, &c);
                if ui
                    .add(egui::Button::new(gutter(false, l)).shortcut_text(format!("{cmd}{k}")))
                    .clicked()
                {
                    out.push(c);
                    ui.close();
                }
            }
            self.items(
                ui,
                &[("Delete", C::DeleteSelection), ("Select All", C::SelectAll)],
                out,
            );
            ui.separator();
            sub(ui, false, false, "Insert", |ui| {
                self.items(
                    ui,
                    &[
                        ("Date and Time (Short)", C::InsertDateTimeShort),
                        ("Date and Time (Long)", C::InsertDateTimeLong),
                        ("Date and Time (Custom)", C::InsertDateTimeCustom),
                        ("Unix Time", C::InsertUnixTime),
                        ("UUID", C::InsertUuid),
                    ],
                    out,
                );
            });
            sub(ui, false, false, "Copy Path", |ui| {
                self.items(
                    ui,
                    &[
                        ("Full File Path", C::CopyFullPath),
                        ("File Name", C::CopyFileName),
                        ("Folder Path", C::CopyDirPath),
                        ("All Open File Names", C::CopyAllNames),
                        ("All Open File Paths", C::CopyAllPaths),
                    ],
                    out,
                );
            });
            ui.separator();
            sub(ui, false, false, "Lines", |ui| {
                self.items(
                    ui,
                    &[
                        ("Duplicate Line", C::DuplicateLine),
                        ("Delete Line", C::DeleteLine),
                        ("Move Line Up", C::MoveLineUp),
                        ("Move Line Down", C::MoveLineDown),
                        ("Swap with Previous Line", C::TransposeLine),
                        ("Insert Blank Line Above", C::InsertBlankAbove),
                        ("Insert Blank Line Below", C::InsertBlankBelow),
                    ],
                    out,
                );
                ui.separator();
                self.items(
                    ui,
                    &[("Split Lines", C::SplitLines), ("Join Lines", C::JoinLines)],
                    out,
                );
                ui.separator();
                self.items(
                    ui,
                    &[
                        ("Remove Duplicate Lines", C::RemoveDuplicateLines),
                        (
                            "Remove Consecutive Duplicates",
                            C::RemoveConsecutiveDuplicates,
                        ),
                        ("Remove Empty Lines", C::RemoveEmptyLines),
                        (
                            "Remove Blank Lines (incl. Whitespace)",
                            C::RemoveEmptyLinesWithBlank,
                        ),
                    ],
                    out,
                );
            });
            sub(ui, false, false, "Sort Lines", |ui| {
                self.items(
                    ui,
                    &[
                        ("Alphabetically", C::Sort(SortMode::LexCiAsc)),
                        ("By Number", C::Sort(SortMode::NumberDesc)),
                    ],
                    out,
                );
            });
            sub(ui, false, false, "Indent", |ui| {
                self.items(
                    ui,
                    &[
                        ("Increase Indent", C::IndentIncrease),
                        ("Decrease Indent", C::IndentDecrease),
                    ],
                    out,
                );
            });
            sub(ui, false, false, "Convert Case", |ui| {
                for m in [
                    CaseMode::Upper,
                    CaseMode::Lower,
                    CaseMode::ProperForce,
                    CaseMode::ProperBlend,
                    CaseMode::SentenceForce,
                    CaseMode::SentenceBlend,
                    CaseMode::Invert,
                    CaseMode::Random,
                ] {
                    self.item(ui, App::case_label(m), C::ConvertCase(m), out);
                }
            });
            sub(ui, false, false, "Comment", |ui| {
                self.items(
                    ui,
                    &[
                        ("Toggle Line Comment", C::ToggleComment),
                        ("Comment Lines", C::BlockComment),
                        ("Uncomment Lines", C::BlockUncomment),
                        ("Block Comment", C::StreamComment),
                        ("Block Uncomment", C::StreamUncomment),
                    ],
                    out,
                );
            });
            sub(ui, false, false, "Whitespace", |ui| {
                self.items(
                    ui,
                    &[
                        ("Trim Trailing Spaces", C::TrimTrailing),
                        ("Trim Leading Spaces", C::TrimLeading),
                        ("Trim Leading and Trailing Spaces", C::TrimBoth),
                        ("Line Breaks to Spaces", C::EolToSpace),
                        ("Trim and Line Breaks to Spaces", C::TrimAndEolToSpace),
                    ],
                    out,
                );
                ui.separator();
                self.items(
                    ui,
                    &[
                        ("Tabs to Spaces", C::TabToSpace),
                        ("Spaces to Tabs (All)", C::SpaceToTabAll),
                        ("Spaces to Tabs (Leading)", C::SpaceToTabLeading),
                    ],
                    out,
                );
            });
            sub(ui, false, false, "Auto-Completion", |ui| {
                self.items(
                    ui,
                    &[
                        ("Complete Keyword", C::FunctionCompletion),
                        ("Complete Word", C::WordCompletion),
                    ],
                    out,
                );
            });
        });

        sub(ui, true, false, "Search", |ui| {
            self.items(
                ui,
                &[
                    ("Find...", C::Find),
                    ("Replace...", C::Replace),
                    ("Find in Files...", C::FindInFiles),
                    ("Incremental Search", C::IncrementalSearch),
                    ("Mark...", C::Mark),
                    ("Filter Lines...", C::FilterLines),
                    ("Find Characters in Range...", C::FindCharsInRange),
                ],
                out,
            );
            ui.separator();
            self.items(
                ui,
                &[("Find Next", C::FindNext), ("Find Previous", C::FindPrev)],
                out,
            );
            ui.separator();
            self.items(
                ui,
                &[
                    ("Go to Line...", C::GoTo),
                    ("Go to Matching Bracket", C::GoToMatchingBrace),
                    ("Select Between Brackets", C::SelectToMatchingBrace),
                ],
                out,
            );
            ui.separator();
            let styles = ["Style 1", "Style 2", "Style 3", "Style 4", "Style 5"];
            sub(ui, false, false, "Highlight", |ui| {
                sub(ui, false, false, "Highlight All Occurrences", |ui| {
                    for (i, s) in (1u8..).zip(styles) {
                        self.item(ui, s, C::StyleAll(i), out);
                    }
                });
                sub(ui, false, false, "Highlight This Occurrence", |ui| {
                    for (i, s) in (1u8..).zip(styles) {
                        self.item(ui, s, C::StyleOne(i), out);
                    }
                });
                ui.separator();
                sub(ui, false, false, "Next Highlight", |ui| {
                    for (i, s) in (1u8..).zip(styles) {
                        self.item(ui, s, C::JumpNextStyle(i), out);
                    }
                    self.item(ui, "Find Marks", C::JumpNextStyle(0), out);
                });
                sub(ui, false, false, "Previous Highlight", |ui| {
                    for (i, s) in (1u8..).zip(styles) {
                        self.item(ui, s, C::JumpPrevStyle(i), out);
                    }
                    self.item(ui, "Find Marks", C::JumpPrevStyle(0), out);
                });
                sub(ui, false, false, "Copy Highlighted Text", |ui| {
                    for (i, s) in (1u8..).zip(styles) {
                        self.item(ui, s, C::CopyStyledText(i), out);
                    }
                    self.item(ui, "All Styles", C::CopyStyledText(0), out);
                });
                ui.separator();
                sub(ui, false, false, "Clear Highlight", |ui| {
                    for (i, s) in (1u8..).zip(styles) {
                        self.item(ui, s, C::ClearStyle(i), out);
                    }
                });
                self.item(ui, "Clear All Highlights", C::ClearAllStyles, out);
            });
            sub(ui, false, false, "Bookmarks", |ui| {
                self.items(
                    ui,
                    &[
                        ("Toggle Bookmark", C::ToggleBookmark),
                        ("Next Bookmark", C::NextBookmark),
                        ("Previous Bookmark", C::PrevBookmark),
                        ("Clear All Bookmarks", C::ClearBookmarks),
                        ("Invert Bookmarks", C::InverseBookmarks),
                    ],
                    out,
                );
                ui.separator();
                self.items(
                    ui,
                    &[
                        ("Cut Bookmarked Lines", C::CutBookmarkedLines),
                        ("Copy Bookmarked Lines", C::CopyBookmarkedLines),
                        ("Paste over Bookmarked Lines", C::PasteToBookmarkedLines),
                        ("Delete Bookmarked Lines", C::RemoveBookmarkedLines),
                        ("Delete Lines Without Bookmarks", C::RemoveUnbookmarkedLines),
                    ],
                    out,
                );
            });
            sub(ui, false, false, "Change History", |ui| {
                self.items(
                    ui,
                    &[
                        ("Next Change", C::ChangeHistoryNext),
                        ("Previous Change", C::ChangeHistoryPrev),
                        ("Clear Change History", C::ChangeHistoryClear),
                    ],
                    out,
                );
            });
        });

        sub(ui, true, false, "View", |ui| {
            let s = &self.settings;
            let (wrap, hl, fs) = (s.word_wrap, s.syntax_highlighting, s.font_size);
            self.check(ui, "Word Wrap", wrap, C::WordWrap, out);
            self.check(ui, "Syntax Highlighting", hl, C::SyntaxHighlighting, out);
            let ig = self.settings.show_indent_guides;
            self.check(ui, "Indent Guides", ig, C::ShowIndentGuides, out);
            sub(ui, false, false, "Show Symbols", |ui| {
                let s = &self.settings;
                let (ws, eol, wsym) = (s.show_whitespace, s.show_eol, s.show_wrap_symbol);
                self.check(ui, "Spaces and Tabs", ws, C::ShowWhitespace, out);
                self.check(ui, "Line Endings", eol, C::ShowEol, out);
                self.check(ui, "All Characters", ws && eol, C::ShowAllChars, out);
                ui.separator();
                self.check(ui, "Wrap Symbol", wsym, C::ShowWrapSymbol, out);
            });
            sub(ui, false, false, "Folding", |ui| {
                self.items(
                    ui,
                    &[
                        ("Fold All", C::FoldAll),
                        ("Unfold All", C::UnfoldAll),
                        ("Fold Current Block", C::FoldCurrent),
                        ("Unfold Current Block", C::UnfoldCurrent),
                    ],
                    out,
                );
                sub(ui, false, false, "Fold Level", |ui| {
                    for i in 1..=8 {
                        self.item(ui, &i.to_string(), C::FoldLevel(i), out);
                    }
                });
                sub(ui, false, false, "Unfold Level", |ui| {
                    for i in 1..=8 {
                        self.item(ui, &i.to_string(), C::UnfoldLevel(i), out);
                    }
                });
            });
            sub(ui, false, false, "Font Size", |ui| {
                self.items(
                    ui,
                    &[
                        ("Increase Font Size", C::FontBigger),
                        ("Decrease Font Size", C::FontSmaller),
                        (
                            "Default Size",
                            C::SetFontSize(crate::app::DEFAULT_FONT_SIZE),
                        ),
                    ],
                    out,
                );
                ui.separator();
                for size in [9u8, 10, 11, 12, 13, 14, 16, 18, 20, 24, 28] {
                    let on = (fs - f32::from(size)).abs() < 0.25;
                    self.check(ui, &format!("{size} pt"), on, C::SetFontSize(size), out);
                }
            });
            ui.separator();
            sub(ui, false, false, "Panels", |ui| {
                let s = &self.settings;
                let panels = [
                    ("Document Map", s.show_doc_map, C::ToggleDocMap),
                    ("Document List", s.show_doc_list, C::ToggleDocList),
                    ("Function List", s.show_function_list, C::ToggleFunctionList),
                    ("JSON Tree", s.show_json_tree, C::ToggleJsonTree),
                    ("Workspace", s.show_folder_tree, C::ToggleFolderTree),
                    ("Character Panel", s.show_char_panel, C::CharacterPanel),
                    (
                        "Clipboard History",
                        s.show_clipboard_history,
                        C::ClipboardHistory,
                    ),
                ];
                for (l, on, c) in panels {
                    self.check(ui, l, on, c, out);
                }
            });
            let s = &self.settings;
            let (tb, sb) = (s.show_toolbar, s.show_statusbar);
            self.check(ui, "Toolbar", tb, C::ToggleToolbar, out);
            self.check(ui, "Status Bar", sb, C::ToggleStatusbar, out);
            ui.separator();
            let (top, full, postit) = (self.always_on_top, self.fullscreen, self.postit);
            self.check(ui, "Always on Top", top, C::AlwaysOnTop, out);
            self.check(ui, "Full Screen", full, C::FullScreen, out);
            self.check(ui, "Post-It", postit, C::PostIt, out);
            self.item(ui, "Distraction-Free Mode", C::DistractionFree, out);
        });

        sub(ui, true, false, "Encoding", |ui| {
            let cur = self.active_doc().map(|d| d.lang);
            let normal = self.langs.normal();
            let current = cur.map_or("Normal Text", |l| self.langs.langs[l].display.as_str());
            sub(ui, false, false, &format!("Language: {current}"), |ui| {
                // Scrolls when the list is taller than the window.
                let max = (ui.ctx().content_rect().height() - 80.0).max(200.0);
                egui::ScrollArea::vertical().max_height(max).show(ui, |ui| {
                    self.check(
                        ui,
                        "Normal Text",
                        cur == Some(normal),
                        C::SetLanguage("normal".into()),
                        out,
                    );
                    ui.separator();
                    for i in self.langs.sorted_for_menu() {
                        let l = &self.langs.langs[i];
                        self.check(
                            ui,
                            &l.display,
                            cur == Some(i),
                            C::SetLanguage(l.name.clone()),
                            out,
                        );
                    }
                });
            });
            ui.separator();
            let cur = self.active_doc().map(|d| d.encoding);
            let main = [
                TextEncoding::Ansi,
                TextEncoding::Utf8,
                TextEncoding::Utf8Bom,
                TextEncoding::Utf16BeBom,
                TextEncoding::Utf16LeBom,
            ];
            for e in main {
                self.check(ui, &e.label(), cur == Some(e), C::SetEncoding(e.key()), out);
            }
            sub(ui, false, false, "Convert to Encoding", |ui| {
                for e in main {
                    self.item(ui, &e.label(), C::ConvertEncoding(e.key()), out);
                }
            });
            ui.separator();
            use crate::encoding::Eol;
            let eol = self.active_doc().map(|d| d.eol);
            self.check(
                ui,
                "Windows (CR LF)",
                eol == Some(Eol::Crlf),
                C::EolWindows,
                out,
            );
            self.check(
                ui,
                "Unix and macOS (LF)",
                eol == Some(Eol::Lf),
                C::EolUnix,
                out,
            );
            self.check(ui, "Classic Mac (CR)", eol == Some(Eol::Cr), C::EolMac, out);
        });

        sub(ui, true, false, "Tools", |ui| {
            sub(ui, false, false, "Format", |ui| {
                self.item(
                    ui,
                    "Format Document (Auto-Detect)",
                    C::Format(FormatOp::Auto),
                    out,
                );
                ui.separator();
                self.items(
                    ui,
                    &[
                        ("JSON: Pretty Print", C::Format(FormatOp::JsonPretty)),
                        ("JSON: Minify", C::Format(FormatOp::JsonMinify)),
                        ("JSON: Validate", C::Format(FormatOp::JsonValidate)),
                    ],
                    out,
                );
                ui.separator();
                self.items(
                    ui,
                    &[
                        ("XML: Pretty Print", C::Format(FormatOp::XmlPretty)),
                        ("XML: Minify", C::Format(FormatOp::XmlMinify)),
                        ("XML: Validate", C::Format(FormatOp::XmlValidate)),
                    ],
                    out,
                );
                ui.separator();
                self.items(
                    ui,
                    &[
                        ("HTML: Pretty Print", C::Format(FormatOp::HtmlPretty)),
                        ("CSS: Pretty Print", C::Format(FormatOp::CssPretty)),
                        ("CSS: Minify", C::Format(FormatOp::CssMinify)),
                        (
                            "JavaScript / TypeScript: Pretty Print",
                            C::Format(FormatOp::JsPretty),
                        ),
                        ("Markdown: Tidy", C::Format(FormatOp::MarkdownPretty)),
                    ],
                    out,
                );
                ui.separator();
                if ui.button(gutter(false, "Formatter Options...")).clicked() {
                    self.dialogs.preferences = true;
                    self.dialogs.pref_tab = crate::dialogs::settings_section::FORMATTING;
                    ui.close();
                }
            });
            sub(ui, false, false, "Encode / Decode", |ui| {
                self.items(
                    ui,
                    &[
                        ("Base64 Encode", C::Mime(MimeOp::Base64Encode)),
                        (
                            "Base64 Encode Each Line",
                            C::Mime(MimeOp::Base64EncodeByLine),
                        ),
                        ("Base64 URL-Safe Encode", C::Mime(MimeOp::Base64UrlEncode)),
                        ("Base64 Decode", C::Mime(MimeOp::Base64Decode)),
                        (
                            "Base64 Decode Each Line",
                            C::Mime(MimeOp::Base64DecodeByLine),
                        ),
                    ],
                    out,
                );
                ui.separator();
                self.items(
                    ui,
                    &[
                        ("URL Encode", C::Mime(MimeOp::UrlEncode)),
                        ("URL Decode", C::Mime(MimeOp::UrlDecode)),
                        ("URL Decode (+ as Space)", C::Mime(MimeOp::UrlDecodeForm)),
                    ],
                    out,
                );
                ui.separator();
                self.items(
                    ui,
                    &[
                        ("Quoted-Printable Encode", C::Mime(MimeOp::QpEncode)),
                        ("Quoted-Printable Decode", C::Mime(MimeOp::QpDecode)),
                    ],
                    out,
                );
            });
            sub(ui, false, false, "Data", |ui| {
                use crate::commands::DataOp;
                self.items(
                    ui,
                    &[
                        ("CSV to JSON", C::Data(DataOp::CsvToJson)),
                        ("JSON to CSV", C::Data(DataOp::JsonToCsv)),
                        ("CSV to TSV", C::Data(DataOp::CsvToTsv)),
                        ("TSV to CSV", C::Data(DataOp::TsvToCsv)),
                    ],
                    out,
                );
            });
            sub(ui, false, false, "Hash", |ui| {
                for algo in [
                    HashAlgo::Md5,
                    HashAlgo::Sha1,
                    HashAlgo::Sha256,
                    HashAlgo::Sha512,
                ] {
                    sub(ui, false, false, algo.name(), |ui| {
                        self.items(
                            ui,
                            &[
                                ("Hash Text...", C::Hash(algo, HashSource::Text)),
                                ("Hash Files...", C::Hash(algo, HashSource::Files)),
                                (
                                    "Copy Hash of Selection",
                                    C::Hash(algo, HashSource::SelectionToClipboard),
                                ),
                            ],
                            out,
                        );
                    });
                }
            });
            ui.separator();
            self.item(ui, "Color Picker...", C::ColorPicker, out);
            self.item(ui, "Password Generator...", C::PasswordGenerator, out);
            self.item(ui, "Date and Unix Time...", C::DateTimeTool, out);
        });

        sub(ui, true, false, "Compare", |ui| {
            self.items(
                ui,
                &[
                    ("Compare", C::Compare),
                    ("Compare Selected Lines", C::CompareSelections),
                    ("Compare with Clipboard", C::CompareClipboard),
                    ("Compare with Last Save", C::CompareSaved),
                    ("New Blank Compare", C::CompareBlank),
                    ("Show Differences Inline", C::CompareInline),
                ],
                out,
            );
            ui.separator();
            ui.add_enabled_ui(self.compare.is_some(), |ui| {
                self.items(
                    ui,
                    &[
                        ("Next Difference", C::CompareNext),
                        ("Previous Difference", C::ComparePrev),
                        ("Copy Difference to Right", C::CompareCopyToRight),
                        ("Copy Difference to Left", C::CompareCopyToLeft),
                        ("Copy All to Right", C::CompareCopyAll(true)),
                        ("Copy All to Left", C::CompareCopyAll(false)),
                        ("Swap Sides", C::CompareSwap),
                        ("Compare Again", C::CompareRefresh),
                        ("End Compare", C::CompareClear),
                    ],
                    out,
                );
            });
            ui.separator();
            // Scroll the two split views together (compares always do).
            let (sv, sh) = (self.settings.sync_scroll_v, self.settings.sync_scroll_h);
            self.check(ui, "Sync Vertical Scrolling", sv, C::SyncScrollV, out);
            self.check(ui, "Sync Horizontal Scrolling", sh, C::SyncScrollH, out);
            sub(ui, false, false, "Options", |ui| {
                let o = self.settings.compare.clone();
                for (i, l, on) in [
                    (0u8, "Ignore Whitespace", o.ignore_whitespace),
                    (1, "Ignore Case", o.ignore_case),
                    (2, "Ignore Empty Lines", o.ignore_empty_lines),
                    (3, "Highlight Changed Words", o.word_diff),
                    (5, "Show Moved Lines", o.detect_moves),
                    (4, "Compare Again While Editing", o.auto),
                    (6, "Hide Side Panels While Comparing", o.hide_panels),
                ] {
                    self.check(ui, l, on, C::CompareOption(i), out);
                }
            });
        });

        sub(ui, true, false, "Window", |ui| {
            let (fm, term) = if cfg!(target_os = "macos") {
                ("Show in Finder", "Open Terminal Here")
            } else {
                ("Show in Explorer", "Open Command Prompt Here")
            };
            self.items(
                ui,
                &[
                    (fm, C::OpenContainingFolder),
                    (term, C::OpenTerminalHere),
                    ("Open in Default App", C::OpenInDefaultViewer),
                    ("Open in Browser", C::ViewInBrowser),
                ],
                out,
            );
            ui.separator();
            sub(ui, false, false, "View", |ui| {
                self.items(
                    ui,
                    &[
                        ("Move to Other View", C::MoveToOtherView),
                        ("Clone to Other View", C::CloneToOtherView),
                        ("Switch to Other View", C::FocusOtherView),
                    ],
                    out,
                );
            });
            sub(ui, false, false, "Tabs", |ui| {
                self.items(
                    ui,
                    &[
                        ("Move to New Window", C::MoveToNewWindow),
                        ("Close All Tabs", C::CloseAll),
                        ("Close All but This", C::CloseAllButCurrent),
                    ],
                    out,
                );
                ui.separator();
                self.items(
                    ui,
                    &[
                        ("Next Tab", C::NextTab),
                        ("Previous Tab", C::PrevTab),
                        ("Move Tab Right", C::MoveTabForward),
                        ("Move Tab Left", C::MoveTabBackward),
                        ("Pin/Unpin Tab", C::TogglePin),
                    ],
                    out,
                );
                sub(ui, false, false, "Tab Color", |ui| {
                    for i in 0..5u8 {
                        self.item(ui, &format!("Color {}", i + 1), C::TabColor(Some(i)), out);
                    }
                    self.item(ui, "No Color", C::TabColor(None), out);
                });
                sub(ui, false, false, "Sort Tabs", |ui| {
                    self.items(
                        ui,
                        &[
                            ("Name A to Z", C::SortTabs(TabSort::NameAsc)),
                            ("Name Z to A", C::SortTabs(TabSort::NameDesc)),
                            ("Path A to Z", C::SortTabs(TabSort::PathAsc)),
                            ("Path Z to A", C::SortTabs(TabSort::PathDesc)),
                            ("Type A to Z", C::SortTabs(TabSort::TypeAsc)),
                            ("Type Z to A", C::SortTabs(TabSort::TypeDesc)),
                        ],
                        out,
                    );
                });
            });
            self.item(ui, "All Open Documents...", C::WindowsDialog, out);
            ui.separator();
            let pane = self.focused;
            for (i, id) in self.panes[pane].tabs.iter().enumerate().take(20) {
                if let Some(d) = self.docs.get(id) {
                    let on = i == self.panes[pane].active;
                    let label = gutter(on, &format!("{}: {}", i + 1, d.display_name()));
                    if ui.button(label).clicked() {
                        out.push(C::GoToTab(i));
                        ui.close();
                    }
                }
            }
        });
    }

    fn toolbar(&mut self, ui: &mut Ui, out: &mut Vec<Command>) {
        use crate::icons::Icon;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let fg = ui.visuals().text_color();
            let sel_bg = ui.visuals().selection.bg_fill;
            // The buttons collect clicks here (not in `out`), so the menu
            // search box in the middle of the toolbar can use `self` and `out`.
            let shortcuts = self.shortcuts.clone();
            let clicked = std::cell::RefCell::new(Vec::new());
            let b = |ui: &mut Ui, icon: Icon, tip: &str, cmd: Command, on: bool| {
                let sc = shortcut_for(&shortcuts, &cmd)
                    .map(|s| format!(" ({s})"))
                    .unwrap_or_default();
                let (rect, r) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                if on {
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(3),
                        editor::translucent(sel_bg, 120),
                    );
                } else if r.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(3),
                        ui.visuals().widgets.hovered.weak_bg_fill,
                    );
                }
                crate::icons::draw(ui.painter(), rect, icon, fg);
                if r.on_hover_text(format!("{tip}{sc}")).clicked() {
                    clicked.borrow_mut().push(cmd);
                }
            };
            b(ui, Icon::New, "New", Command::New, false);
            b(ui, Icon::Open, "Open", Command::Open, false);
            b(
                ui,
                Icon::Folder,
                "Workspace",
                Command::ToggleFolderTree,
                self.settings.show_folder_tree,
            );
            b(ui, Icon::Save, "Save", Command::Save, false);
            b(ui, Icon::SaveAll, "Save All", Command::SaveAll, false);
            b(ui, Icon::Close, "Close", Command::Close, false);
            ui.separator();
            b(ui, Icon::Cut, "Cut", Command::Cut, false);
            b(ui, Icon::Copy, "Copy", Command::Copy, false);
            b(ui, Icon::Paste, "Paste", Command::Paste, false);
            ui.separator();
            b(ui, Icon::Undo, "Undo", Command::Undo, false);
            b(ui, Icon::Redo, "Redo", Command::Redo, false);
            ui.separator();
            b(ui, Icon::Find, "Find", Command::Find, false);
            b(ui, Icon::Replace, "Replace", Command::Replace, false);
            ui.separator();
            self.menu_search_box(ui, out);
            ui.separator();
            b(
                ui,
                Icon::Highlight,
                "Syntax Highlighting",
                Command::SyntaxHighlighting,
                self.settings.syntax_highlighting,
            );
            b(
                ui,
                Icon::AllChars,
                "Show All Characters",
                Command::ShowAllChars,
                self.settings.show_whitespace && self.settings.show_eol,
            );
            ui.separator();
            b(
                ui,
                Icon::DocMap,
                "Document Map",
                Command::ToggleDocMap,
                self.settings.show_doc_map,
            );
            b(
                ui,
                Icon::DocList,
                "Document List",
                Command::ToggleDocList,
                self.settings.show_doc_list,
            );
            b(
                ui,
                Icon::FunctionList,
                "Function List",
                Command::ToggleFunctionList,
                self.settings.show_function_list,
            );
            ui.separator();
            // Type a line number and press Enter to jump there.
            let r = ui
                .add(
                    egui::TextEdit::singleline(&mut self.goto_line_text)
                        .hint_text("Line #")
                        .desired_width(70.0),
                )
                .on_hover_text("Type a line number and press Enter");
            self.goto_line_text.retain(|c| c.is_ascii_digit());
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let (Some(id), Ok(n)) = (self.active_id(), self.goto_line_text.parse::<usize>())
                {
                    self.pending_goto = Some((id, n.saturating_sub(1), 0));
                    self.request_focus_editor = true;
                    // The jump happens at the start of the next frame.
                    ui.ctx().request_repaint();
                }
                self.goto_line_text.clear();
            }
            out.extend(clicked.take());
        });
    }

    /// Every menu command with its menu path (walks the menus off screen).
    pub fn menu_entries(&mut self) -> Vec<MenuEntry> {
        COLLECTING.with(|c| *c.borrow_mut() = Some(MenuCollector::default()));
        let ctx = egui::Context::default();
        let mut ignored = Vec::new();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            self.menu_bar(ui, &mut ignored);
        });
        let entries = COLLECTING
            .with(|c| c.borrow_mut().take())
            .map(|c| c.entries)
            .unwrap_or_default();
        // The same command can appear in two menus; keep the first.
        let mut seen = Vec::new();
        entries
            .into_iter()
            .filter(|e| {
                let new = !seen.contains(&e.cmd);
                if new {
                    seen.push(e.cmd.clone());
                }
                new
            })
            .collect()
    }

    /// Type part of a menu item's name; Up/Down choose, Enter runs it.
    fn menu_search_box(&mut self, ui: &mut Ui, out: &mut Vec<Command>) {
        let hint = if cfg!(target_os = "macos") {
            "Search menus (⌘⇧P)"
        } else {
            "Search menus (Ctrl+Shift+P)"
        };
        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.menu_search.text)
                .hint_text(hint)
                .desired_width(220.0),
        );
        if std::mem::take(&mut self.menu_search.focus) {
            resp.request_focus();
        }
        if resp.gained_focus() {
            self.menu_search.entries = self.menu_entries();
            self.menu_search.selected = 0;
            self.menu_search.open = true;
        }
        if resp.changed() {
            self.menu_search.selected = 0;
            self.menu_search.open = true;
        }
        let over_popup = self
            .menu_search
            .popup_rect
            .zip(ui.input(|i| i.pointer.hover_pos()))
            .is_some_and(|(r, p)| r.contains(p));
        if !resp.has_focus() && !over_popup {
            self.menu_search.open = false;
        }
        if !self.menu_search.open || self.menu_search.text.trim().is_empty() {
            self.menu_search.popup_rect = None;
            return;
        }
        let hits = self.menu_search.matches(30);
        let (down, up, enter, esc) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let sel = &mut self.menu_search.selected;
        if down {
            *sel = (*sel + 1).min(hits.len().saturating_sub(1));
        }
        if up {
            *sel = sel.saturating_sub(1);
        }
        let mut run = None;
        if enter && let Some(e) = hits.get(*sel) {
            run = Some(e.cmd.clone());
        }
        if esc {
            self.menu_search.text.clear();
            self.request_focus_editor = true;
        }
        let selected = self.menu_search.selected;
        let area = egui::Area::new(egui::Id::unique("menu_search_popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(resp.rect.left_bottom() + vec2(0.0, 2.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(380.0);
                    if hits.is_empty() {
                        ui.label(RichText::new("No matching menu items").weak());
                    }
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            for (i, e) in hits.iter().enumerate() {
                                let sc = self.sc(&e.cmd);
                                let text = format!("{}    {}", e.label, e.path);
                                let r = ui.add(
                                    egui::Button::selectable(i == selected, text)
                                        .shortcut_text(sc)
                                        .min_size(vec2(ui.available_width(), 0.0)),
                                );
                                if i == selected && (down || up) {
                                    r.scroll_to_me(None);
                                }
                                if r.clicked() {
                                    run = Some(e.cmd.clone());
                                }
                            }
                        });
                });
            });
        self.menu_search.popup_rect = Some(area.response.rect);
        if let Some(cmd) = run {
            out.push(cmd);
            self.menu_search.text.clear();
            self.menu_search.open = false;
            self.menu_search.popup_rect = None;
            self.request_focus_editor = true;
        }
    }

    fn status_bar(&mut self, ui: &mut Ui, out: &mut Vec<Command>) {
        let now = ui.input(|i| i.time);
        let pane = self.focused;
        let Some(id) = self.active_id() else { return };
        let Some(doc) = self.docs.get(&id) else {
            return;
        };
        let vs = self.views.get(&(pane, id));
        let lang = &self.langs.langs[doc.lang];
        let (line, col, pos, sel_chars, sel_lines) = vs.map_or((0, 0, 0, 0, 0), |v| {
            let p = v.primary();
            let (l, _) = doc.pos_to_line_col(p.head);
            let col = v.display_col(doc, p.head);
            let chars: usize = v.sels.iter().map(|s| s.end() - s.start()).sum();
            let lines = if chars > 0 {
                v.sels
                    .iter()
                    .map(|s| doc.char_to_line(s.end()) - doc.char_to_line(s.start()) + 1)
                    .sum()
            } else {
                0
            };
            (l + 1, col + 1, p.head + 1, chars, lines)
        });
        let nsel = vs.map_or(1, |v| v.sels.len());
        let eol = doc.eol;
        let enc = doc.encoding;
        let lang_name = if lang.is_plain() {
            "Normal text file".to_string()
        } else {
            format!("{} file", lang.display)
        };
        let (len, lines) = (doc.len_chars(), doc.line_count());
        let ro = doc.read_only;
        ui.horizontal(|ui| {
            ui.label(lang_name);
            ui.separator();
            ui.label(format!("length : {len}    lines : {lines}"));
            ui.separator();
            let sel = if nsel > 1 {
                format!("Sel : {nsel} carets")
            } else {
                format!("Sel : {sel_chars} | {sel_lines}")
            };
            if ui
                .add(
                    egui::Label::new(format!(
                        "Ln : {line}    Col : {col}    Pos : {pos}    {sel}"
                    ))
                    .sense(Sense::click()),
                )
                .on_hover_text("Click to go to line")
                .clicked()
            {
                out.push(Command::GoTo);
            }
            if let Some(c) = self.compare_status() {
                ui.separator();
                ui.label(RichText::new(c).strong());
            }
            if let Some((msg, t)) = &self.status_msg {
                if now - t < 4.0 {
                    ui.separator();
                    ui.label(RichText::new(msg).italics());
                } else {
                    self.status_msg = None;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(if self.overtype { "OVR" } else { "INS" });
                ui.separator();
                ui.menu_button(enc.label(), |ui| {
                    for e in [
                        TextEncoding::Ansi,
                        TextEncoding::Utf8,
                        TextEncoding::Utf8Bom,
                        TextEncoding::Utf16BeBom,
                        TextEncoding::Utf16LeBom,
                    ] {
                        if ui.button(format!("Convert to {}", e.label())).clicked() {
                            out.push(Command::ConvertEncoding(e.key()));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                let mixed = self.active_doc().is_some_and(Document::has_mixed_eol);
                let eol_label = if mixed {
                    format!("Mixed ({})", eol.label())
                } else {
                    eol.label().to_string()
                };
                ui.menu_button(eol_label, |ui| {
                    for (l, c) in [
                        ("Windows (CR LF)", Command::EolWindows),
                        ("Unix (LF)", Command::EolUnix),
                        ("Macintosh (CR)", Command::EolMac),
                    ] {
                        if ui.button(l).clicked() {
                            out.push(c);
                            ui.close();
                        }
                    }
                });
                if ro {
                    ui.separator();
                    ui.label("Read-only");
                }
            });
        });
    }

    // ---------------------------------------------------------------- editors & tabs

    fn editors(&mut self, ui: &mut Ui, out: &mut Vec<Command>) {
        let full = ui.available_rect_before_wrap();
        let split = !self.panes[1].tabs.is_empty();
        if !split {
            let r = full;
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                self.pane_ui(ui, 0, out)
            });
            return;
        }
        let div = 5.0;
        let x = full.min.x + full.width() * self.split_ratio;
        let left = egui::Rect::from_min_max(full.min, egui::pos2(x - div / 2.0, full.max.y));
        let right = egui::Rect::from_min_max(egui::pos2(x + div / 2.0, full.min.y), full.max);
        let divider = egui::Rect::from_min_max(
            egui::pos2(x - div / 2.0, full.min.y),
            egui::pos2(x + div / 2.0, full.max.y),
        );
        let resp = ui.interact(divider, egui::Id::unique("split_div"), Sense::drag());
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.split_ratio = ((p.x - full.min.x) / full.width()).clamp(0.15, 0.85);
        }
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        ui.painter().rect_filled(
            divider,
            CornerRadius::ZERO,
            ui.visuals().widgets.noninteractive.bg_fill,
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
            self.pane_ui(ui, 0, out)
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
            self.pane_ui(ui, 1, out)
        });
        self.sync_compare_scroll();
        // Synchronized scrolling between the two views.
        if self.compare.is_none() && (self.settings.sync_scroll_v || self.settings.sync_scroll_h) {
            let (a, b) = (self.panes[0].active_doc(), self.panes[1].active_doc());
            if let (Some(a), Some(b)) = (a, b) {
                let src = (self.focused, if self.focused == 0 { a } else { b });
                let dst = (1 - self.focused, if self.focused == 0 { b } else { a });
                if let Some(v) = self.views.get(&src).map(|v| (v.top_row, v.scroll_x)) {
                    // The last position of the view being scrolled. Only a
                    // change in that same view moves the other one, so
                    // switching views or tabs doesn't make it jump.
                    let key = egui::Id::unique("sync_prev");
                    let prev: Option<((usize, crate::document::DocId), f64, f32)> =
                        ui.ctx().data(|d| d.get_temp(key));
                    if let Some((from, pt, px)) = prev
                        && from == src
                        && let Some(dv) = self.views.get_mut(&dst)
                    {
                        if self.settings.sync_scroll_v {
                            dv.top_row = (dv.top_row + (v.0 - pt)).max(0.0);
                        }
                        if self.settings.sync_scroll_h {
                            dv.scroll_x = (dv.scroll_x + (v.1 - px)).max(0.0);
                        }
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(key, (src, v.0, v.1)));
                }
            }
        }
    }

    fn pane_ui(&mut self, ui: &mut Ui, pane: usize, out: &mut Vec<Command>) {
        if self.settings.show_tabbar && !self.postit {
            self.tab_bar(ui, pane, out);
        }
        let Some(id) = self.panes[pane].active_doc() else {
            return;
        };
        let lang = self.docs.get(&id).map_or(0, |d| d.lang);
        let styles = self.styles_for(lang).clone();
        let keywords = self.keywords_for(lang);
        let globals = self.globals.clone();
        let settings = self.settings.clone();
        let font_size = settings.font_size;
        let Some(doc) = self.docs.get_mut(&id) else {
            return;
        };
        let vs = self.views.entry((pane, id)).or_default();
        let mut overtype = self.overtype;
        let mut env = EditorEnv {
            settings: &settings,
            globals: &globals,
            styles: &styles,
            keywords: &keywords,
            clip: &mut self.clip,
            overtype: &mut overtype,
            font_size,
        };
        let eid = Self::editor_id(pane);
        let o = editor::show(ui, eid, vs, doc, &mut env);
        self.overtype = overtype;
        if self.request_focus_editor && self.focused == pane {
            ui.memory_mut(|m| m.request_focus(eid));
            self.request_focus_editor = false;
        }
        // A view keeps focus while it has it, unless focus is being moved
        // to the other view (Switch to Other View) this frame.
        let has_focus = o.response.as_ref().is_some_and(egui::Response::has_focus);
        if o.clicked || (has_focus && !self.request_focus_editor) {
            self.focused = pane;
        }
        out.extend(o.commands);
    }

    #[allow(clippy::too_many_lines)]
    fn tab_bar(&mut self, ui: &mut Ui, pane: usize, out: &mut Vec<Command>) {
        let tabs = self.panes[pane].tabs.clone();
        let active = self.panes[pane].active;
        let focused = self.focused == pane;
        let accent = self.globals.active_tab_indicator;
        let mut activate: Option<usize> = None;
        let mut close: Option<usize> = None;
        let mut new_doc = false;
        let mut ctx_cmds: Vec<(usize, Command)> = Vec::new();
        let visuals = ui.visuals().clone();
        let bar = egui::Frame::NONE
            .fill(visuals.extreme_bg_color)
            .inner_margin(egui::Margin::symmetric(2, 2));
        let mut list_pick: Option<usize> = None;
        bar.show(ui, |ui| {
            ui.horizontal(|ui| {
                const LIST_BUTTON: f32 = 26.0;
                let tabs_w = (ui.available_width() - LIST_BUTTON - 4.0).max(0.0);
                ui.allocate_ui(vec2(tabs_w, 24.0), |ui| {
                    egui::ScrollArea::horizontal()
                        .id_salt(("tabs", pane))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 1.0;
                                let mut hover_index = None;
                                for (i, id) in tabs.iter().enumerate() {
                                    let Some(d) = self.docs.get(id) else { continue };
                                    let name = d.display_name();
                                    let dirty = d.is_dirty();
                                    let ro = d.read_only;
                                    let pinned = d.pinned;
                                    let color = d.tab_color.map(|c| TAB_COLORS[usize::from(c) % 5]);
                                    let font = egui::FontId::proportional(13.0);
                                    let galley = ui.painter().layout_no_wrap(
                                        name.clone(),
                                        font,
                                        visuals.text_color(),
                                    );
                                    let w = galley.size().x + 50.0;
                                    let (rect, resp) = ui.allocate_exact_size(
                                        vec2(w, 24.0),
                                        Sense::click_and_drag(),
                                    );
                                    let is_active = i == active;
                                    if is_active && self.tab_scrolled[pane] != Some(*id) {
                                        resp.scroll_to_me(None);
                                        self.tab_scrolled[pane] = Some(*id);
                                    }
                                    let bg = if is_active {
                                        visuals.panel_fill
                                    } else if resp.hovered() {
                                        visuals.widgets.hovered.weak_bg_fill
                                    } else {
                                        visuals.widgets.inactive.weak_bg_fill
                                    };
                                    let p = ui.painter();
                                    p.rect_filled(
                                        rect,
                                        CornerRadius {
                                            nw: 4,
                                            ne: 4,
                                            sw: 0,
                                            se: 0,
                                        },
                                        bg,
                                    );
                                    if let Some(c) = color {
                                        p.rect_filled(
                                            egui::Rect::from_min_size(
                                                rect.left_bottom() - vec2(0.0, 3.0),
                                                vec2(rect.width(), 3.0),
                                            ),
                                            CornerRadius::ZERO,
                                            c,
                                        );
                                    }
                                    if is_active {
                                        let c = if focused {
                                            accent
                                        } else {
                                            editor::translucent(accent, 120)
                                        };
                                        p.rect_filled(
                                            egui::Rect::from_min_size(
                                                rect.min,
                                                vec2(rect.width(), 3.0),
                                            ),
                                            CornerRadius {
                                                nw: 4,
                                                ne: 4,
                                                sw: 0,
                                                se: 0,
                                            },
                                            c,
                                        );
                                    }
                                    // Save-state icon: blue = saved, red = modified, grey lock = read-only.
                                    let icon_c = rect.left_center() + vec2(11.0, 0.0);
                                    if ro {
                                        let lr = egui::Rect::from_center_size(
                                            icon_c + vec2(0.0, 1.5),
                                            vec2(9.0, 7.0),
                                        );
                                        p.rect_filled(
                                            lr,
                                            CornerRadius::same(1),
                                            Color32::from_gray(130),
                                        );
                                        p.circle_stroke(
                                            icon_c - vec2(0.0, 2.5),
                                            2.6,
                                            Stroke::new(1.3, Color32::from_gray(130)),
                                        );
                                    } else {
                                        let c = if dirty {
                                            Color32::from_rgb(0xE0, 0x40, 0x40)
                                        } else {
                                            Color32::from_rgb(0x40, 0x80, 0xE0)
                                        };
                                        p.rect_filled(
                                            egui::Rect::from_center_size(icon_c, vec2(9.0, 10.0)),
                                            CornerRadius::same(1),
                                            c,
                                        );
                                        p.rect_filled(
                                            egui::Rect::from_center_size(
                                                icon_c - vec2(0.0, 2.5),
                                                vec2(5.0, 3.0),
                                            ),
                                            CornerRadius::ZERO,
                                            Color32::WHITE,
                                        );
                                    }
                                    let text_color = if is_active {
                                        visuals.strong_text_color()
                                    } else {
                                        visuals.text_color()
                                    };
                                    p.galley(
                                        rect.left_center() + vec2(22.0, -galley.size().y / 2.0),
                                        galley,
                                        text_color,
                                    );
                                    // Close button / pin.
                                    let close_rect = egui::Rect::from_center_size(
                                        rect.right_center() - vec2(12.0, 0.0),
                                        vec2(14.0, 14.0),
                                    );
                                    let hover_close = ui
                                        .input(|inp| inp.pointer.hover_pos())
                                        .is_some_and(|h| close_rect.contains(h));
                                    if pinned {
                                        p.circle_filled(
                                            close_rect.center() - vec2(0.0, 2.0),
                                            3.5,
                                            Color32::from_rgb(0xD0, 0x40, 0x40),
                                        );
                                        p.line_segment(
                                            [
                                                close_rect.center() - vec2(0.0, 1.0),
                                                close_rect.center() + vec2(0.0, 5.0),
                                            ],
                                            Stroke::new(1.2, visuals.text_color()),
                                        );
                                    } else if self.settings.tab_close_button
                                        && (resp.hovered() || is_active)
                                    {
                                        if hover_close {
                                            p.rect_filled(
                                                close_rect,
                                                CornerRadius::same(3),
                                                Color32::from_rgb(0xD0, 0x40, 0x40),
                                            );
                                        }
                                        let c = if hover_close {
                                            Color32::WHITE
                                        } else {
                                            visuals.weak_text_color()
                                        };
                                        let cc = close_rect.center();
                                        let s = 3.5;
                                        p.line_segment(
                                            [cc - vec2(s, s), cc + vec2(s, s)],
                                            Stroke::new(1.4, c),
                                        );
                                        p.line_segment(
                                            [cc + vec2(-s, s), cc + vec2(s, -s)],
                                            Stroke::new(1.4, c),
                                        );
                                    }
                                    if resp.clicked() {
                                        if hover_close && !pinned && self.settings.tab_close_button
                                        {
                                            close = Some(i);
                                        } else {
                                            activate = Some(i);
                                        }
                                    }
                                    if resp.middle_clicked()
                                        || (resp.double_clicked()
                                            && self.settings.double_click_close_tab)
                                    {
                                        close = Some(i);
                                    }
                                    if resp.drag_started() {
                                        self.tab_drag = Some((pane, i));
                                    }
                                    if let Some(h) = ui.input(|inp| inp.pointer.hover_pos())
                                        && rect.contains(h)
                                    {
                                        hover_index = Some(i);
                                    }
                                    let path = d.full_path_or_name();
                                    let resp = resp.on_hover_text(path);
                                    resp.context_menu(|ui| {
                                        let compare = (i != active)
                                            .then(|| tabs.get(active).copied())
                                            .flatten();
                                        tab_context_menu(ui, d, compare, &mut |cmd, this_tab| {
                                            ctx_cmds
                                                .push((if this_tab { i } else { usize::MAX }, cmd));
                                        });
                                    });
                                }
                                // Drag-to-reorder.
                                if let Some((dp, from)) = self.tab_drag
                                    && dp == pane
                                {
                                    if ui.input(|i| i.pointer.any_released()) {
                                        if let Some(to) = hover_index
                                            && to != from
                                        {
                                            let p = &mut self.panes[pane];
                                            let id = p.tabs.remove(from);
                                            p.tabs.insert(to, id);
                                            p.active = to;
                                        }
                                        self.tab_drag = None;
                                    } else if let Some(to) = hover_index
                                        && to != from
                                    {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                    }
                                }
                                // Double-click on empty tab bar area: new document.
                                let rest = ui.available_rect_before_wrap();
                                let (r, resp) = ui.allocate_exact_size(
                                    vec2(rest.width().max(40.0), 24.0),
                                    Sense::click(),
                                );
                                let _ = r;
                                if resp.double_clicked() {
                                    new_doc = true;
                                }
                            });
                        });
                });
                self.tab_list_button(ui, &tabs, active, &mut list_pick);
            });
        });
        if let Some(i) = list_pick {
            activate = Some(i);
        }
        if let Some(i) = activate {
            self.panes[pane].active = i;
            self.focused = pane;
            self.request_focus_editor = true;
        }
        if let Some(i) = close
            && let Some(&id) = tabs.get(i)
        {
            self.close_tab(pane, id, false);
        }
        if new_doc {
            self.new_document(pane);
        }
        for (i, cmd) in ctx_cmds {
            if i == usize::MAX {
                out.push(cmd);
                continue;
            }
            self.panes[pane].active = i;
            self.focused = pane;
            out.push(cmd);
        }
        let _ = StrokeKind::Inside;
    }

    /// The button at the end of a tab bar that lists every tab, for when
    /// they don't all fit.
    fn tab_list_button(
        &mut self,
        ui: &mut Ui,
        tabs: &[crate::document::DocId],
        active: usize,
        pick: &mut Option<usize>,
    ) {
        let label = RichText::new("▼").size(10.0);
        let resp = ui
            .menu_button(label, |ui| {
                ui.set_min_width(260.0);
                let filter = ui.add(
                    egui::TextEdit::singleline(&mut self.tab_list_filter)
                        .hint_text("Search tabs")
                        .desired_width(f32::INFINITY),
                );
                if filter.gained_focus() || self.tab_list_filter.is_empty() {
                    filter.request_focus();
                }
                let needle = self.tab_list_filter.to_lowercase();
                let max = (ui.ctx().content_rect().height() - 120.0).max(200.0);
                let mut first = None;
                egui::ScrollArea::vertical().max_height(max).show(ui, |ui| {
                    for (i, id) in tabs.iter().enumerate() {
                        let Some(d) = self.docs.get(id) else { continue };
                        let path = d.full_path_or_name();
                        if !needle.is_empty() && !path.to_lowercase().contains(&needle) {
                            continue;
                        }
                        first.get_or_insert(i);
                        let text = RichText::new(Self::doc_label(d));
                        let text = if i == active { text.strong() } else { text };
                        if ui
                            .selectable_label(i == active, text)
                            .on_hover_text(path)
                            .clicked()
                        {
                            *pick = Some(i);
                            ui.close();
                        }
                    }
                });
                if filter.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && let Some(i) = first
                {
                    *pick = Some(i);
                    ui.close();
                }
            })
            .response
            .on_hover_text(format!("All tabs ({})", tabs.len()));
        if pick.is_some()
            || !egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp))
        {
            self.tab_list_filter.clear();
        }
    }

    pub fn doc_label(d: &Document) -> String {
        format!(
            "{}{}",
            if d.is_dirty() { "*" } else { "" },
            d.display_name()
        )
    }
}

#[cfg(test)]
mod bench {
    use std::time::{Duration, Instant};

    use egui::{Event, RawInput, Rect, pos2};

    use crate::app::{App, StartupArgs};

    /// Tests share one process, and the settings folder comes from an
    /// environment variable; hold this while a test uses its own folder.
    fn use_config_dir(dir: impl AsRef<std::path::Path>) -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: only these tests change the variable, one at a time under the lock.
        unsafe { std::env::set_var("SUPERFASTNOTEPAD_CONFIG_DIR", dir.as_ref()) };
        guard
    }

    fn stats(name: &str, t: &mut [Duration]) {
        t.sort_unstable();
        let n = t.len();
        let avg = t.iter().sum::<Duration>() / u32::try_from(n).unwrap();
        println!(
            "{name:<28} avg {avg:>9.2?}  p95 {:>9.2?}  max {:>9.2?}",
            t[n * 95 / 100],
            t[n - 1]
        );
    }

    /// Opens the new tool windows and renamed dialogs off-screen and checks
    /// they draw, and that the font size commands stay within limits.
    #[test]
    fn dialogs_and_font_commands() {
        let cfg = std::env::temp_dir().join("superfastnotepad-dialog-test");
        let _ = std::fs::remove_dir_all(&cfg);
        let _config = use_config_dir(cfg);
        let ctx = egui::Context::default();
        crate::setup_fonts(&ctx).unwrap();
        let mut app = App::new(StartupArgs {
            files: Vec::new(),
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        app.run(crate::commands::Command::DateTimeTool);
        app.dialogs.preferences = true;
        app.dialogs.windows = true;
        for _ in 0..3 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    egui::vec2(1400.0, 900.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| app.frame(ui));
        }
        assert!(app.dialogs.datetime.is_some());
        app.settings.font_size = 47.0;
        app.run(crate::commands::Command::FontBigger);
        app.run(crate::commands::Command::FontBigger);
        assert!((app.settings.font_size - 48.0).abs() < f32::EPSILON);
        app.run(crate::commands::Command::SetFontSize(7));
        app.run(crate::commands::Command::FontSmaller);
        app.run(crate::commands::Command::FontSmaller);
        assert!((app.settings.font_size - 6.0).abs() < f32::EPSILON);
        // Following the system light/dark setting.
        assert!(app.settings.follow_system_theme);
        app.follow_system_theme(true);
        assert_eq!(app.themes.themes[app.theme_idx].name, "Dark");
        app.follow_system_theme(false);
        assert_eq!(app.themes.themes[app.theme_idx].name, "Light");
        // Picking a theme while the system is dark changes the dark theme.
        app.follow_system_theme(true);
        let midnight = app.themes.index_of("Midnight");
        app.set_theme(midnight);
        assert_eq!(app.settings.dark_theme, "Midnight");
        assert_eq!(app.settings.theme, "Light");
        app.settings.follow_system_theme = false;
        app.follow_system_theme(false);
        assert_eq!(app.themes.themes[app.theme_idx].name, "Midnight");
        app.run(crate::commands::Command::InsertUuid);
        let id = app.active_id().unwrap();
        assert_eq!(app.docs[&id].len_chars(), 36);
    }

    /// Blank compare, moved lines, hidden panels, swap, copy all and compare
    /// selected lines, driven through commands.
    #[test]
    fn compare_session_features() {
        use crate::commands::Command as C;
        let dir = std::env::temp_dir().join("superfastnotepad-compare-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _config = use_config_dir(dir);
        let mut app = App::new(StartupArgs {
            files: Vec::new(),
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        app.settings.show_function_list = true;
        app.run(C::CompareBlank);
        assert!(
            !app.settings.show_function_list,
            "panels hidden while comparing"
        );
        let s = app.compare.as_ref().unwrap();
        let (l, r) = (s.left, s.right);
        assert_eq!(app.docs[&l].display_name(), "Original");
        let set = |app: &mut App, id, text: &str| {
            let d = app.docs.get_mut(&id).unwrap();
            let len = d.len_chars();
            d.edit(
                vec![(0, len, text.into())],
                &[],
                crate::document::EditKind::Other,
                |_| Vec::new(),
            );
        };
        set(&mut app, l, "a\nb\nc\nd");
        set(&mut app, r, "b\nc\nD\na");
        app.run(C::CompareRefresh);
        assert_eq!(app.compare.as_ref().unwrap().stats, (0, 0, 1, 1));
        app.run(C::CompareSwap);
        let s = app.compare.as_ref().unwrap();
        assert_eq!((s.left, s.right), (r, l));
        assert!(!app.settings.show_function_list);
        app.run(C::CompareCopyAll(true));
        assert_eq!(app.docs[&l].rope.to_string(), "b\nc\nD\na");
        assert!(app.compare.as_ref().unwrap().hunks.is_empty());
        app.run(C::CompareClear);
        assert!(app.settings.show_function_list, "panels shown again");
        // Compare selected lines: selections extended to whole lines.
        let (l, r) = (
            app.panes[0].active_doc().unwrap(),
            app.panes[1].active_doc().unwrap(),
        );
        set(&mut app, l, "x\none\ntwo\ny");
        set(&mut app, r, "one\nTWO");
        app.views.get_mut(&(0, l)).unwrap().sels =
            vec![crate::document::Selection { anchor: 3, head: 8 }];
        app.views.get_mut(&(1, r)).unwrap().sels =
            vec![crate::document::Selection { anchor: 0, head: 7 }];
        app.run(C::CompareSelections);
        let s = app.compare.as_ref().unwrap();
        assert_eq!(s.snapshots.len(), 2);
        assert_eq!(app.docs[&s.left].rope.to_string(), "one\ntwo");
        assert_eq!(s.stats.2, 1);
        let snaps = s.snapshots.clone();
        app.run(C::CompareClear);
        assert!(snaps.iter().all(|id| !app.docs.contains_key(id)));
        // Inline view of the two views: removed line, then its replacement.
        set(&mut app, l, "same\nold\nend");
        set(&mut app, r, "same\nnew\nend");
        app.focused = 0;
        app.run(C::CompareInline);
        let id = app.active_id().unwrap();
        let d = &app.docs[&id];
        assert!(d.read_only && d.transient);
        assert_eq!(d.rope.to_string(), "same\nold\nnew\nend");
        use crate::compare::LineKind as K;
        let decor = app.views[&(0, id)].compare_decor(d).expect("decorated");
        assert_eq!(decor.kinds, vec![K::Same, K::Removed, K::Added, K::Same]);
        assert!(
            app.build_session(false)
                .files
                .iter()
                .all(|f| !f.untitled_name.starts_with("Diff"))
        );
    }

    /// An unchanged document's backup isn't rewritten; an edited one is.
    #[test]
    fn backups_rewrite_only_changed_documents() {
        let dir = std::env::temp_dir().join("superfastnotepad-backup-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _config = use_config_dir(dir);
        let mut app = App::new(StartupArgs {
            files: Vec::new(),
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        let id = app.active_id().unwrap();
        app.with_active(|d, v, _| {
            crate::ops::insert_text(d, v, "hello", crate::document::EditKind::Other)
        });
        let first = app.build_session(true);
        let path = std::path::PathBuf::from(first.files[0].backup.clone().expect("backup written"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        let (v1, t1, _) = app.backups_written[&id].clone();
        app.build_session(true);
        let (v2, t2, _) = app.backups_written[&id].clone();
        assert_eq!((v1, t1), (v2, t2), "unchanged document was rewritten");
        app.with_active(|d, v, _| {
            crate::ops::insert_text(d, v, "!", crate::document::EditKind::Other)
        });
        app.build_session(true);
        assert_ne!(app.backups_written[&id].0, v1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello!");
    }

    /// Filters a log off-screen: only matching lines stay visible, the count
    /// is right, and closing the bar shows every line again.
    #[test]
    fn line_filter_hides_other_lines() {
        let dir = std::env::temp_dir().join("superfastnotepad-filter-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("app.log");
        let mut text = String::new();
        for i in 0..2000 {
            let level = if i % 100 == 7 { "ERROR" } else { "INFO" };
            text.push_str(&format!(
                "2026-10-30 10:00:{:02} {level} request {i}\n",
                i % 60
            ));
        }
        std::fs::write(&log, &text).unwrap();
        let _config = use_config_dir(dir.join("cfg"));
        let ctx = egui::Context::default();
        crate::setup_fonts(&ctx).unwrap();
        let mut app = App::new(StartupArgs {
            files: vec![log.display().to_string()],
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        let mut time = 0.0;
        let mut frame = |app: &mut App| {
            time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    egui::vec2(1200.0, 800.0),
                )),
                time: Some(time),
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| app.frame(ui));
        };
        frame(&mut app);
        app.run(crate::commands::Command::FilterLines);
        app.filter.query.text = "error".into();
        for _ in 0..200 {
            frame(&mut app);
            if app.filter.last.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let id = app.active_id().unwrap();
        let shown = app.docs[&id].line_filter.clone().expect("filter applied");
        assert_eq!(shown.len(), 20);
        assert!(shown.iter().all(|&l| l % 100 == 7));
        assert_eq!(app.filter.last.as_ref().unwrap().total, 2001);
        // Export copies just those lines into a new tab.
        app.run(crate::commands::Command::ExportFilteredLines);
        let new_id = app.active_id().unwrap();
        assert_ne!(new_id, id);
        assert_eq!(app.docs[&new_id].rope.to_string().lines().count(), 20);
        // Back on the log, closing the bar removes the filter.
        app.activate(app.focused, id);
        frame(&mut app);
        assert!(app.docs[&id].line_filter.is_some());
        app.filter.open = false;
        frame(&mut app);
        assert_eq!(app.active_id(), Some(id));
        assert!(app.docs[&id].line_filter.is_none());
    }

    /// Moving a tab to a new window hands over unsaved text (untitled or
    /// with a path) and saved files, and the hand-off files are removed.
    #[test]
    fn move_tab_to_new_window_hand_off() {
        let dir = std::env::temp_dir().join("superfastnotepad-handoff-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let saved = dir.join("saved.rs");
        std::fs::write(&saved, "fn main() {}\n").unwrap();
        let _config = use_config_dir(dir.join("cfg"));
        let args = |files: Vec<String>, take| StartupArgs {
            files,
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take,
        };
        let mut app = App::new(args(vec![saved.display().to_string()], None));
        let saved_id = app.active_id().unwrap();
        let untitled = app.new_document(0);
        let edit = |app: &mut App, id, text: &str| {
            let d = app.docs.get_mut(&id).unwrap();
            d.edit(
                vec![(0, 0, text.into())],
                &[],
                crate::document::EditKind::Other,
                |_| Vec::new(),
            );
        };
        edit(&mut app, untitled, "unsaved notes\nline two");
        let cases = [
            (untitled, None, "unsaved notes\nline two", true),
            (saved_id, Some(&saved), "fn main() {}\n", false),
        ];
        for (id, path, text, dirty) in cases {
            let take = app.hand_off(0, id).unwrap();
            let moved = App::new(args(Vec::new(), Some(take.clone())));
            let d = moved.active_doc().unwrap();
            assert_eq!(d.rope.to_string(), text);
            assert_eq!(d.is_dirty(), dirty);
            assert_eq!(d.path.as_ref(), path);
            assert_eq!(moved.panes[0].tabs.len(), 1, "no extra empty tab");
            assert!(!take.parent().unwrap().exists(), "hand-off files removed");
        }
        assert_eq!(
            App::new(args(Vec::new(), None))
                .active_doc()
                .unwrap()
                .display_name(),
            "new 1"
        );
        // A modified file keeps its path and its unsaved text.
        edit(&mut app, saved_id, "// edited\n");
        let moved = App::new(args(Vec::new(), Some(app.hand_off(0, saved_id).unwrap())));
        let d = moved.active_doc().unwrap();
        assert_eq!(d.rope.to_string(), "// edited\nfn main() {}\n");
        assert!(d.is_dirty());
        assert_eq!(d.path.as_ref(), Some(&saved));
    }

    /// Web search defaults to Google, and settings saved with the old
    /// default move to it; a custom engine is kept.
    #[test]
    fn search_engine_defaults_to_google() {
        let dir = std::env::temp_dir().join("superfastnotepad-search-engine-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _config = use_config_dir(&dir);
        let google = crate::config::DEFAULT_SEARCH_ENGINE;
        assert_eq!(crate::config::load_settings().search_engine, google);
        for (saved, want) in [
            ("https://duckduckgo.com/?q=$(CURRENT_WORD)", google),
            (
                "https://example.com/?s=$(CURRENT_WORD)",
                "https://example.com/?s=$(CURRENT_WORD)",
            ),
        ] {
            let json = format!("{{\"search_engine\": \"{saved}\"}}");
            std::fs::write(dir.join("config.json"), json).unwrap();
            assert_eq!(crate::config::load_settings().search_engine, want);
        }
    }

    /// Menu entries line up whether or not they show the "on" dot.
    #[test]
    fn menu_gutter_has_the_same_width() {
        let ctx = egui::Context::default();
        crate::setup_fonts(&ctx).unwrap();
        let _ = ctx.run_ui(RawInput::default(), |ui| {
            let width = |text: String| {
                let font = egui::TextStyle::Button.resolve(ui.style());
                ui.painter()
                    .layout_no_wrap(text, font, egui::Color32::WHITE)
                    .size()
                    .x
            };
            for label in ["Word Wrap", "Font Size", "Status Bar"] {
                let on = width(super::gutter(true, label));
                let off = width(super::gutter(false, label));
                assert!((on - off).abs() < 0.01, "{label}: {on} vs {off}");
            }
        });
    }

    /// The color picker starts from a selected hex color, draws, and
    /// inserting puts `#RRGGBB` at the caret.
    #[test]
    fn color_picker_starts_from_selection() {
        let dir = std::env::temp_dir().join("superfastnotepad-color-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _config = use_config_dir(dir.join("cfg"));
        let ctx = egui::Context::default();
        crate::setup_fonts(&ctx).unwrap();
        let mut app = App::new(StartupArgs {
            files: Vec::new(),
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        let id = app.active_id().unwrap();
        app.docs.get_mut(&id).unwrap().edit(
            vec![(0, 0, "color: #1a2B3c;".into())],
            &[],
            crate::document::EditKind::Other,
            |_| Vec::new(),
        );
        let version = app.docs[&id].version;
        let view = app.views.entry((0, id)).or_default();
        view.sels = vec![crate::document::Selection::new(7, 14)];
        view.seen_version = version;
        app.run(crate::commands::Command::ColorPicker);
        let picker = app.dialogs.color_picker.as_ref().unwrap();
        assert_eq!(picker.rgb(), [0x1a, 0x2b, 0x3c]);
        assert_eq!(picker.hex, "1A2B3C");
        for _ in 0..3 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| app.frame(ui));
        }
        assert!(app.dialogs.color_picker.is_some(), "stays open");
        // What Insert does: replace the selection with the picked color.
        app.dialogs
            .color_picker
            .as_mut()
            .unwrap()
            .set_rgb([255, 255, 255]);
        let hex = crate::colorpick::to_hex(app.dialogs.color_picker.as_ref().unwrap().rgb());
        app.with_active(|d, v, _| {
            crate::ops::insert_text(d, v, &hex, crate::document::EditKind::Other)
        });
        assert_eq!(app.docs[&id].rope.to_string(), "color: #FFFFFF;");
    }

    /// "Open Folder in Workspace" adds a saved file's folder (unsaved
    /// changes or not) and shows the panel; untitled tabs do nothing.
    #[test]
    fn duplicate_names_count_up() {
        use crate::app::copy_name;
        use std::path::{Path, PathBuf};
        let p = Path::new("/w/notes.txt");
        assert_eq!(copy_name(p, |_| false), PathBuf::from("/w/notes copy.txt"));
        let taken = [
            PathBuf::from("/w/notes copy.txt"),
            PathBuf::from("/w/notes copy 2.txt"),
        ];
        assert_eq!(
            copy_name(p, |q| taken.iter().any(|t| t == q)),
            PathBuf::from("/w/notes copy 3.txt")
        );
        assert_eq!(
            copy_name(Path::new("/w/Makefile"), |_| false),
            PathBuf::from("/w/Makefile copy")
        );
    }

    #[test]
    fn duplicate_rename_and_delete_files() {
        use crate::app::{FileAction, Prompt, RenameTarget};
        let dir = std::env::temp_dir().join("superfastnotepad-fileops-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        std::fs::write(dir.join("taken.txt"), "").unwrap();
        let _config = use_config_dir(dir.join("cfg"));
        let mut app = App::new(StartupArgs {
            files: vec![file.display().to_string()],
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        let id = app.active_id().unwrap();

        // Duplicate: a copy beside it, opened and selected.
        app.file_action(FileAction::Duplicate(file.clone()));
        let copy = dir.join("notes copy.txt");
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "fn main() {}\n");
        assert_eq!(app.workspace_selected.as_ref(), Some(&copy));
        let copy_id = app.active_id().unwrap();
        assert_ne!(copy_id, id);
        assert_eq!(app.docs[&copy_id].path.as_ref(), Some(&copy));

        // Rename the open file: its tab follows, and so does the language.
        app.file_action(FileAction::Rename(file.clone()));
        assert_eq!(
            app.dialogs.rename,
            Some((RenameTarget::File(file.clone()), "notes.txt".into()))
        );
        let renamed = app.rename_file(&file, "main.rs").unwrap();
        assert!(!file.exists() && renamed.exists());
        assert_eq!(app.docs[&id].path.as_ref(), Some(&renamed));
        assert_eq!(
            app.langs.langs[app.docs[&id].lang].name,
            app.langs.langs[app.langs.detect("main.rs", "")].name
        );
        // Never replaces another file, and rejects paths.
        assert!(app.rename_file(&renamed, "taken.txt").is_err());
        assert!(app.rename_file(&renamed, "../x.rs").is_err());
        assert!(app.rename_file(&renamed, "  ").is_err());
        assert!(renamed.exists());

        // Delete asks first.
        app.file_action(FileAction::Delete(copy.clone()));
        assert!(matches!(app.prompts.last(), Some(Prompt::DeleteFile(p)) if *p == copy));
        assert!(copy.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Moves a temporary file to the real Trash:
    /// `cargo test trash_closes_the_tab -- --ignored`
    #[test]
    #[ignore = "uses the system Trash"]
    fn trash_closes_the_tab() {
        let dir = std::env::temp_dir().join("superfastnotepad-trash-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("superfastnotepad trash test.txt");
        std::fs::write(&file, "bye\n").unwrap();
        let _config = use_config_dir(dir.join("cfg"));
        let mut app = App::new(StartupArgs {
            files: vec![file.display().to_string()],
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        let id = app.active_id().unwrap();
        app.trash_file(&file).unwrap();
        assert!(!file.exists());
        assert!(!app.docs.contains_key(&id));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_ops_menu_items() {
        use crate::panels::{FileOp, file_ops_menu};
        let saved = std::env::temp_dir().join("superfastnotepad-fileops-menu.txt");
        std::fs::write(&saved, "").unwrap();
        // Draws the menu (in place, not in a popup) and clicks button `n`.
        let click = |path: Option<&std::path::Path>, n: Option<usize>| {
            let ctx = egui::Context::default();
            crate::setup_fonts(&ctx).unwrap();
            let mut got = None;
            let mut buttons = Vec::new();
            for (i, pressed) in [None, Some(true), Some(false)].into_iter().enumerate() {
                let events = match (pressed, n.and_then(|n| buttons.get(n))) {
                    (Some(pressed), Some(r)) => {
                        let r: &Rect = r;
                        vec![
                            Event::PointerMoved(r.center()),
                            Event::PointerButton {
                                pos: r.center(),
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]
                    }
                    _ => Vec::new(),
                };
                let input = RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        pos2(0.0, 0.0),
                        egui::vec2(800.0, 600.0),
                    )),
                    time: Some(i as f64 * 0.1),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run_ui(input, |ui| {
                    if let Some(op) = file_ops_menu(ui, path) {
                        got = Some(op);
                    }
                });
                buttons = ctx.viewport(|v| {
                    let w = &v.prev_pass.widgets;
                    let mut r: Vec<Rect> = w
                        .layer_ids()
                        .flat_map(|l| {
                            w.get_layer(l)
                                .filter(|x| x.sense.senses_click() && x.rect.height() < 40.0)
                                .map(|x| x.rect)
                                .collect::<Vec<_>>()
                        })
                        .collect();
                    r.sort_by(|a, b| a.min.y.total_cmp(&b.min.y));
                    r
                });
            }
            (buttons.len(), got)
        };
        assert_eq!(click(Some(&saved), None).0, 3, "duplicate, rename, delete");
        assert_eq!(click(None, None).0, 1, "unsaved: rename only");
        assert_eq!(click(Some(&saved), Some(0)).1, Some(FileOp::Duplicate));
        assert_eq!(click(Some(&saved), Some(1)).1, Some(FileOp::Rename));
        assert_eq!(click(Some(&saved), Some(2)).1, Some(FileOp::Delete));
        assert_eq!(click(None, Some(0)).1, Some(FileOp::Rename));
        let _ = std::fs::remove_file(&saved);
    }

    #[test]
    fn open_file_folder_in_workspace() {
        let dir = std::env::temp_dir().join("superfastnotepad-ws-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let file = dir.join("src").join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let _config = use_config_dir(dir.join("cfg"));
        let args = |files| StartupArgs {
            files,
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        };
        let mut app = App::new(args(Vec::new()));
        app.settings.workspace_folders.clear();
        app.settings.show_folder_tree = false;
        app.run(crate::commands::Command::OpenFileFolderInWorkspace);
        assert!(
            app.settings.workspace_folders.is_empty(),
            "untitled: nothing"
        );
        let mut app = App::new(args(vec![file.display().to_string()]));
        app.settings.workspace_folders.clear();
        app.settings.show_folder_tree = false;
        let id = app.active_id().unwrap();
        app.docs.get_mut(&id).unwrap().edit(
            vec![(0, 0, "// edit\n".into())],
            &[],
            crate::document::EditKind::Other,
            |_| Vec::new(),
        );
        app.run(crate::commands::Command::OpenFileFolderInWorkspace);
        assert_eq!(
            app.settings.workspace_folders,
            vec![dir.join("src").display().to_string()]
        );
        assert!(app.settings.show_folder_tree);
    }

    /// The menu search finds commands anywhere in the menus, with their
    /// path, and ranks labels that start with the text first.
    #[test]
    fn menu_search_finds_commands() {
        let dir = std::env::temp_dir().join("superfastnotepad-menu-search-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _config = use_config_dir(dir.join("cfg"));
        let mut app = App::new(StartupArgs {
            files: Vec::new(),
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        let entries = app.menu_entries();
        assert!(entries.len() > 200, "{} entries", entries.len());
        let find = |label: &str| entries.iter().find(|e| e.label == label).cloned();
        let dup = find("Duplicate Line").unwrap();
        assert_eq!(dup.path, "Edit > Lines");
        assert_eq!(dup.cmd, crate::commands::Command::DuplicateLine);
        assert_eq!(find("Copy").unwrap().cmd, crate::commands::Command::Copy);
        assert_eq!(find("Python").unwrap().path, "Encoding > Language");
        let mut search = super::MenuSearch {
            entries,
            ..Default::default()
        };
        search.text = "color pick".into();
        assert_eq!(
            search.matches(5).first().map(|e| e.cmd.clone()),
            Some(crate::commands::Command::ColorPicker)
        );
        search.text = "word wrap".into();
        assert_eq!(search.matches(5)[0].cmd, crate::commands::Command::WordWrap);
        search.text = "zzzz nothing".into();
        assert!(search.matches(5).is_empty());
    }

    /// Split views scroll together by default, and switching which view
    /// has focus doesn't make the other one jump.
    #[test]
    fn split_views_scroll_together() {
        let dir = std::env::temp_dir().join("superfastnotepad-sync-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _config = use_config_dir(dir.join("cfg"));
        let ctx = egui::Context::default();
        crate::setup_fonts(&ctx).unwrap();
        let mut app = App::new(StartupArgs {
            files: Vec::new(),
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        assert!(app.settings.sync_scroll_v && app.settings.sync_scroll_h);
        assert!(app.settings.word_wrap && app.settings.show_indent_guides);
        let text: String = (0..2000).map(|i| format!("line {i}\n")).collect();
        let a = app.active_id().unwrap();
        app.docs.get_mut(&a).unwrap().edit(
            vec![(0, 0, text)],
            &[],
            crate::document::EditKind::Other,
            |_| Vec::new(),
        );
        app.run(crate::commands::Command::CloneToOtherView);
        app.focused = 0;
        let frame = |app: &mut App| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| app.frame(ui));
        };
        let top = |app: &App, pane: usize| app.views[&(pane, a)].top_row;
        frame(&mut app);
        app.views.get_mut(&(0, a)).unwrap().top_row = 100.0;
        frame(&mut app);
        assert!((top(&app, 1) - 100.0).abs() < 0.01, "{}", top(&app, 1));
        // Switch focus: the left view stays where it is.
        app.run(crate::commands::Command::FocusOtherView);
        frame(&mut app);
        frame(&mut app);
        assert!((top(&app, 0) - 100.0).abs() < 0.01, "{}", top(&app, 0));
        app.views.get_mut(&(1, a)).unwrap().top_row = 150.0;
        frame(&mut app);
        assert!((top(&app, 0) - 150.0).abs() < 0.01, "{}", top(&app, 0));
    }

    /// Dragging the floating Find window doesn't select text in the
    /// document behind it.
    #[test]
    fn dragging_find_window_leaves_text_alone() {
        let dir = std::env::temp_dir().join("superfastnotepad-find-drag-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _config = use_config_dir(dir.join("cfg"));
        let ctx = egui::Context::default();
        crate::setup_fonts(&ctx).unwrap();
        let mut app = App::new(StartupArgs {
            files: Vec::new(),
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        let id = app.active_id().unwrap();
        let text: String = (0..200)
            .map(|i| format!("some text on line {i}\n"))
            .collect();
        app.docs.get_mut(&id).unwrap().edit(
            vec![(0, 0, text)],
            &[],
            crate::document::EditKind::Other,
            |_| Vec::new(),
        );
        app.find.open = true;
        app.settings.find_docked = false;
        let mut time = 0.0;
        let mut frame = |app: &mut App, events: Vec<Event>| {
            time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    egui::vec2(1200.0, 800.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| app.frame(ui));
        };
        frame(&mut app, Vec::new());
        frame(&mut app, Vec::new());
        let window = app.find.window_rect.expect("find window shown");
        let title = window.center_top() + egui::vec2(-60.0, 10.0);
        let press = |pos, pressed| Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, vec![Event::PointerMoved(title)]);
        frame(&mut app, vec![press(title, true)]);
        for i in 1..=10 {
            let p = title + egui::vec2(-30.0 * i as f32, 20.0 * i as f32);
            frame(&mut app, vec![Event::PointerMoved(p)]);
        }
        let end = title + egui::vec2(-300.0, 200.0);
        frame(&mut app, vec![press(end, false)]);
        let moved = app.find.window_rect.unwrap();
        assert!(moved.min.distance(window.min) > 50.0, "the window moved");
        let sel = app.views[&(0, id)].primary();
        assert!(sel.is_empty(), "nothing selected: {sel:?}");
    }

    /// Clicks every item of the tab right-click menu (submenus too) and
    /// checks the command each one sends. Items that need a file on disk
    /// only appear for saved files.
    #[test]
    fn tab_menu_items_all_send_their_command() {
        use crate::commands::Command as C;
        use crate::document::Document;
        let langs = crate::langs::LangRegistry::load();
        let mut saved = Document::from_text(7, "a.txt".into(), "x\n", &langs, langs.normal());
        saved.path = Some(std::env::temp_dir().join("a.txt"));
        let untitled = Document::from_text(8, "new 1".into(), "", &langs, langs.normal());

        // Opens the menu on a button, then clicks the item at `path`
        // (an index, or a submenu index then an item index) and returns
        // what the menu sent.
        let click = |doc: &Document, compare: Option<u64>, path: &[usize]| -> Vec<(C, bool)> {
            let ctx = egui::Context::default();
            crate::setup_fonts(&ctx).unwrap();
            let mut got = Vec::new();
            let mut t = 0.0;
            let mut frame = |events: Vec<Event>, got: &mut Vec<(C, bool)>| {
                t += 0.1;
                let input = RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        pos2(0.0, 0.0),
                        egui::vec2(1200.0, 900.0),
                    )),
                    time: Some(t),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run_ui(input, |ui| {
                    let r = ui.button("tab");
                    r.context_menu(|ui| {
                        super::tab_context_menu(ui, doc, compare, &mut |c, this| {
                            got.push((c, this));
                        });
                    });
                });
            };
            let btn = |pos, button, pressed| Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            // Menu items in the topmost popup layers, top to bottom.
            let items = |ctx: &egui::Context, right_of: f32| -> Vec<Rect> {
                ctx.viewport(|v| {
                    let w = &v.prev_pass.widgets;
                    let mut r: Vec<Rect> = w
                        .layer_ids()
                        .filter(|l| l.order == egui::Order::Foreground)
                        .flat_map(|l| {
                            w.get_layer(l)
                                .filter(|x| x.sense.senses_click() && x.rect.height() < 40.0)
                                .map(|x| x.rect)
                                .collect::<Vec<_>>()
                        })
                        .filter(|r| r.min.x > right_of)
                        .collect();
                    r.sort_by(|a, b| a.min.y.total_cmp(&b.min.y));
                    r
                })
            };
            let at = pos2(20.0, 12.0);
            frame(vec![Event::PointerMoved(at)], &mut got);
            frame(
                vec![btn(at, egui::PointerButton::Secondary, true)],
                &mut got,
            );
            frame(
                vec![btn(at, egui::PointerButton::Secondary, false)],
                &mut got,
            );
            frame(vec![], &mut got);
            let top = items(&ctx, 0.0);
            let mut target = top[path[0]];
            if let Some(&j) = path.get(1) {
                // Hover the submenu, then move right into it.
                frame(vec![Event::PointerMoved(target.center())], &mut got);
                frame(vec![], &mut got);
                frame(vec![], &mut got);
                let sub = items(&ctx, target.right() - 4.0);
                let row = target.center().y;
                target = sub[j];
                frame(
                    vec![Event::PointerMoved(pos2(target.left() + 4.0, row))],
                    &mut got,
                );
            }
            let c = target.center();
            frame(vec![Event::PointerMoved(c)], &mut got);
            frame(vec![btn(c, egui::PointerButton::Primary, true)], &mut got);
            frame(vec![btn(c, egui::PointerButton::Primary, false)], &mut got);
            frame(vec![], &mut got);
            got
        };

        let saved_items: Vec<(Vec<usize>, C)> = vec![
            (vec![0], C::Close),
            (vec![1], C::CloseAll),
            (vec![2], C::CloseAllButCurrent),
            (vec![3], C::CloseAllUnchanged),
            (vec![4], C::Save),
            (vec![5], C::SaveAs),
            (vec![6], C::Rename),
            (vec![7], C::Reload),
            (vec![8, 0], C::OpenContainingFolder),
            (vec![8, 1], C::OpenTerminalHere),
            (vec![8, 2], C::OpenInDefaultViewer),
            (vec![8, 3], C::ViewInBrowser),
            (vec![9], C::OpenFileFolderInWorkspace),
            (vec![10, 0], C::CopyFullPath),
            (vec![10, 1], C::CopyFileName),
            (vec![10, 2], C::CopyDirPath),
            (vec![11], C::MoveToOtherView),
            (vec![12], C::CloneToOtherView),
            (vec![13], C::MoveToNewWindow),
            (vec![14], C::ToggleReadOnly),
            (vec![15], C::TogglePin),
            (vec![16, 0], C::TabColor(Some(0))),
            (vec![16, 4], C::TabColor(Some(4))),
            (vec![16, 5], C::TabColor(None)),
        ];
        for (path, want) in saved_items {
            assert_eq!(
                click(&saved, None, &path),
                vec![(want.clone(), true)],
                "{path:?} {want:?}"
            );
        }
        // Untitled: no file items; Compare uses the two tabs as given.
        assert_eq!(
            click(&untitled, None, &[7]),
            vec![(C::MoveToOtherView, true)]
        );
        assert_eq!(
            click(&untitled, Some(3), &[7]),
            vec![(C::CompareWith(3, 8), false)]
        );
    }

    /// The tab menu's Copy items copy the saved file's path, name and folder.
    #[test]
    fn copy_path_commands_for_a_saved_file() {
        let dir = std::env::temp_dir().join("superfastnotepad-copy-path-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "hi\n").unwrap();
        let _config = use_config_dir(dir.join("cfg"));
        let mut app = App::new(StartupArgs {
            files: vec![file.display().to_string()],
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        use crate::commands::Command as C;
        for (cmd, want) in [
            (C::CopyFullPath, file.display().to_string()),
            (C::CopyFileName, "notes.txt".to_string()),
            (C::CopyDirPath, dir.display().to_string()),
        ] {
            app.run(cmd);
            assert_eq!(app.clip.text, want);
        }
    }

    /// A finished update check that found a newer release adds "Update
    /// Available" to the File menu and reports it in Settings.
    #[test]
    fn update_available_shows_in_file_menu() {
        let dir = std::env::temp_dir().join("superfastnotepad-update-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _config = use_config_dir(dir.join("cfg"));
        let mut app = App::new(StartupArgs {
            files: Vec::new(),
            line: None,
            column: None,
            lang: None,
            no_session: false,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        assert!(app.settings.check_for_updates, "on by default");
        let has_item = |app: &mut App| {
            app.menu_entries()
                .iter()
                .any(|e| e.cmd == crate::commands::Command::OpenUpdatePage)
        };
        assert!(!has_item(&mut app));
        // Tests never start the real check (no network).
        app.poll(0.0);
        assert!(app.update.job.is_none());
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Ok(Some(crate::update::Update {
            version: "9.0.0".into(),
            url: "https://example.com/release".into(),
        })))
        .unwrap();
        app.update.job = Some(rx);
        app.update.manual = true;
        app.poll(0.0);
        assert_eq!(app.update.available.as_ref().unwrap().version, "9.0.0");
        assert_eq!(
            app.update.status.as_deref(),
            Some("Version 9.0.0 is available.")
        );
        let entries = app.menu_entries();
        let item = entries
            .iter()
            .find(|e| e.cmd == crate::commands::Command::OpenUpdatePage)
            .expect("Update Available in the File menu");
        assert_eq!(item.label, "Update Available (v9.0.0)...");
        assert_eq!(item.path, "File");
        // Up to date.
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Ok(None)).unwrap();
        app.update.job = Some(rx);
        app.update.manual = true;
        app.poll(0.0);
        assert_eq!(
            app.update.status,
            Some(format!(
                "You have the latest version ({}).",
                crate::update::current_version()
            ))
        );
    }

    /// Syntax highlighting at startup follows the Settings option, not how
    /// the toolbar toggle was left; older settings files keep the toggle.
    #[test]
    fn highlighting_on_start_setting() {
        let dir = std::env::temp_dir().join("superfastnotepad-hl-start-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _config = use_config_dir(&dir);
        let load = |json: &str| {
            std::fs::write(dir.join("config.json"), json).unwrap();
            let s = crate::config::load_settings();
            (s.highlight_on_start, s.syntax_highlighting)
        };
        assert_eq!(load("{}"), (false, false));
        assert_eq!(
            load(r#"{"highlight_on_start": true, "syntax_highlighting": false}"#),
            (true, true)
        );
        assert_eq!(
            load(r#"{"highlight_on_start": false, "syntax_highlighting": true}"#),
            (false, false)
        );
        // Saved before the option existed: start the way the toggle was left.
        assert_eq!(load(r#"{"syntax_highlighting": true}"#), (true, true));
    }

    /// Many tabs: the bar draws with the tab list button and the active tab
    /// is scrolled into view.
    #[test]
    fn many_tabs_render() {
        let dir = std::env::temp_dir().join("superfastnotepad-tabs-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _config = use_config_dir(dir.join("cfg"));
        let ctx = egui::Context::default();
        crate::setup_fonts(&ctx).unwrap();
        let mut app = App::new(StartupArgs {
            files: Vec::new(),
            line: None,
            column: None,
            lang: None,
            no_session: true,
            read_only: false,
            always_on_top: false,
            take: None,
        });
        for _ in 0..60 {
            app.new_document(0);
        }
        let last = *app.panes[0].tabs.last().unwrap();
        app.activate(0, last);
        for _ in 0..3 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    egui::vec2(900.0, 600.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| app.frame(ui));
        }
        assert_eq!(app.tab_scrolled[0], Some(last));
        app.run(crate::commands::Command::CloseAll);
        assert_eq!(app.panes[0].tabs.len(), 1);
    }

    /// Runs the whole UI off-screen (no window, no GPU) and times building
    /// and tessellating frames while idle, scrolling and typing.
    #[test]
    #[ignore = "benchmark: BENCH_FILE=path cargo test --release ui_bench -- --ignored --nocapture"]
    fn ui_bench() {
        let path = std::env::var("BENCH_FILE").expect("BENCH_FILE");
        let cfg = std::env::temp_dir().join("superfastnotepad-bench");
        let _ = std::fs::remove_dir_all(&cfg);
        let _config = use_config_dir(cfg);
        for highlight in [false, true] {
            let ctx = egui::Context::default();
            crate::setup_fonts(&ctx).unwrap();
            let mut app = App::new(StartupArgs {
                files: vec![path.clone()],
                line: None,
                column: None,
                lang: None,
                no_session: true,
                read_only: false,
                always_on_top: false,
                take: None,
            });
            app.settings.syntax_highlighting = highlight;
            app.settings.show_function_list = std::env::var_os("BENCH_FUNCLIST").is_some();
            if let Ok(v) = std::env::var("BENCH_INDENT_GUIDES") {
                app.settings.show_indent_guides = v == "1";
            }
            let mut time = 0.0;
            let mut frame = |app: &mut App, events: Vec<Event>| {
                time += 1.0 / 120.0;
                let input = RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        pos2(0.0, 0.0),
                        egui::vec2(1600.0, 1000.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                };
                let t = Instant::now();
                let out = ctx.run_ui(input, |ui| app.frame(ui));
                let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
                t.elapsed()
            };
            for _ in 0..5 {
                frame(&mut app, Vec::new());
            }
            let key = |app: &App| (app.focused, app.panes[app.focused].active_doc().unwrap());
            let mut idle: Vec<_> = (0..60).map(|_| frame(&mut app, Vec::new())).collect();
            // Idle frames keep colouring in the background; count how long
            // until the whole file is done, then time a jump to the end.
            let k = key(&app);
            let t = Instant::now();
            let mut fill_frames = 0;
            while app.docs[&k.1].is_lexing_pending() && fill_frames < 10_000 {
                frame(&mut app, Vec::new());
                fill_frames += 1;
            }
            let fill = t.elapsed();
            app.views.get_mut(&k).unwrap().top_row = 1e9;
            let mut jump = vec![frame(&mut app, Vec::new())];
            app.views.get_mut(&k).unwrap().top_row = 0.0;
            frame(&mut app, Vec::new());
            let mut scroll = Vec::new();
            for i in 0..300 {
                let k = key(&app);
                app.views.get_mut(&k).unwrap().top_row = f64::from(i) * 40.0;
                scroll.push(frame(&mut app, Vec::new()));
            }
            // Jump to the middle, then type there.
            let k = key(&app);
            app.views.get_mut(&k).unwrap().top_row = 60_000.0;
            app.request_focus_editor = true;
            frame(&mut app, Vec::new());
            frame(&mut app, Vec::new());
            let mut typing: Vec<_> = "notify_all_listeners(x); "
                .chars()
                .cycle()
                .take(100)
                .map(|c| frame(&mut app, vec![Event::Text(c.to_string())]))
                .collect();
            let mut after: Vec<_> = (0..60).map(|_| frame(&mut app, Vec::new())).collect();
            let doc = &app.docs[&k.1];
            assert!(doc.is_dirty(), "typing didn't reach the editor");
            println!(
                "--- syntax highlighting {}",
                if highlight { "ON" } else { "OFF" }
            );
            stats("idle frame", &mut idle);
            println!("background colouring done after {fill_frames} frames ({fill:.0?})");
            stats("jump to end frame", &mut jump);
            stats("scroll frame (new rows)", &mut scroll);
            stats("typing frame", &mut typing);
            stats("idle frame after typing", &mut after);
        }
    }
}
