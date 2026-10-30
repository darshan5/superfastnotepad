//! Main window: menus, toolbar, tab bars, split editors and status bar.

use egui::{Align, Color32, CornerRadius, Layout, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

use crate::app::App;
use crate::commands::{Command, FormatOp, HashSource, MimeOp, TabSort, shortcut_for};
use crate::document::Document;
use crate::editor::{self, EditorEnv};
use crate::encoding::{CHARSETS, TextEncoding};
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
            | Command::About
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

impl App {
    pub fn editor_id(pane: usize) -> egui::Id {
        egui::Id::unique(("editor", pane))
    }

    fn sc(&self, c: &Command) -> String {
        shortcut_for(&self.shortcuts, c).unwrap_or_default()
    }

    /// Menu item that pushes a command when clicked.
    fn item(&self, ui: &mut Ui, label: &str, cmd: Command, out: &mut Vec<Command>) {
        let sc = self.sc(&cmd);
        let btn = egui::Button::new(label).shortcut_text(sc);
        if ui.add(btn).clicked() {
            out.push(cmd);
            ui.close();
        }
    }

    fn check(&self, ui: &mut Ui, label: &str, on: bool, cmd: Command, out: &mut Vec<Command>) {
        let sc = self.sc(&cmd);
        let text = if on {
            format!("• {label}")
        } else {
            format!("    {label}")
        };
        if ui.add(egui::Button::new(text).shortcut_text(sc)).clicked() {
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

        self.find_dialog(&ctx);
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
        ui.menu_button("File", |ui| {
            self.items(
                ui,
                &[
                    ("New", C::New),
                    ("New Window", C::NewWindow),
                    ("Open...", C::Open),
                    ("Open Folder as Workspace...", C::OpenFolderAsWorkspace),
                ],
                out,
            );
            let recent = !self.persisted.recent_files.is_empty();
            ui.add_enabled_ui(recent, |ui| {
                ui.menu_button("Open Recent", |ui| {
                    for (i, f) in self
                        .persisted
                        .recent_files
                        .iter()
                        .take(self.settings.recent_max)
                        .enumerate()
                    {
                        if ui.button(format!("{}: {f}", i + 1)).clicked() {
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
            ui.separator();
            self.items(ui, &[("Close", C::Close), ("Close All", C::CloseAll)], out);
            ui.menu_button("Close Others", |ui| {
                self.items(
                    ui,
                    &[
                        ("All but This", C::CloseAllButCurrent),
                        ("All but Pinned", C::CloseAllButPinned),
                        ("All to the Left", C::CloseAllToLeft),
                        ("All to the Right", C::CloseAllToRight),
                        ("All Unchanged", C::CloseAllUnchanged),
                    ],
                    out,
                );
            });
            ui.separator();
            ui.menu_button("Session", |ui| {
                self.items(
                    ui,
                    &[
                        ("Load Session...", C::LoadSession),
                        ("Save Session...", C::SaveSession),
                    ],
                    out,
                );
            });
            ui.menu_button("Settings", |ui| {
                self.items(
                    ui,
                    &[
                        ("Preferences...", C::Preferences),
                        ("Theme and Font...", C::StyleConfigurator),
                        ("Keyboard Shortcuts...", C::ShortcutMapper),
                    ],
                    out,
                );
            });
            ui.separator();
            self.item(ui, "Exit", C::Exit, out);
        });

        ui.menu_button("Edit", |ui| {
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
                if ui
                    .add(egui::Button::new(l).shortcut_text(format!("{cmd}{k}")))
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
            ui.menu_button("Selection", |ui| {
                self.items(
                    ui,
                    &[
                        ("Begin/End Select", C::BeginEndSelect),
                        ("Begin/End Column Select", C::BeginEndSelectColumn),
                        ("Column Editor...", C::ColumnEditor),
                    ],
                    out,
                );
                ui.separator();
                let variants = [
                    ("Any Case, Partial Words", false, false),
                    ("Match Case", true, false),
                    ("Whole Words", false, true),
                    ("Match Case, Whole Words", true, true),
                ];
                ui.menu_button("Select All Occurrences", |ui| {
                    for (l, match_case, whole_word) in variants {
                        self.item(
                            ui,
                            l,
                            C::MultiSelectAll {
                                match_case,
                                whole_word,
                            },
                            out,
                        );
                    }
                });
                ui.menu_button("Add Next Occurrence", |ui| {
                    for (l, match_case, whole_word) in variants {
                        self.item(
                            ui,
                            l,
                            C::MultiSelectNext {
                                match_case,
                                whole_word,
                            },
                            out,
                        );
                    }
                });
                self.items(
                    ui,
                    &[
                        ("Undo Last Added Occurrence", C::MultiSelectUndo),
                        ("Skip Occurrence", C::MultiSelectSkip),
                    ],
                    out,
                );
            });
            ui.menu_button("Insert", |ui| {
                self.items(
                    ui,
                    &[
                        ("Date and Time (Short)", C::InsertDateTimeShort),
                        ("Date and Time (Long)", C::InsertDateTimeLong),
                        ("Date and Time (Custom)", C::InsertDateTimeCustom),
                        ("UUID", C::InsertUuid),
                    ],
                    out,
                );
            });
            ui.menu_button("Copy Path", |ui| {
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
            ui.menu_button("Lines", |ui| {
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
            ui.menu_button("Sort Lines", |ui| {
                for m in [
                    SortMode::LexAsc,
                    SortMode::LexCiAsc,
                    SortMode::LocaleAsc,
                    SortMode::IntAsc,
                    SortMode::DecCommaAsc,
                    SortMode::DecDotAsc,
                    SortMode::LengthAsc,
                ] {
                    self.item(ui, App::sort_mode_label(m), C::Sort(m), out);
                }
                ui.separator();
                for m in [
                    SortMode::LexDesc,
                    SortMode::LexCiDesc,
                    SortMode::LocaleDesc,
                    SortMode::IntDesc,
                    SortMode::DecCommaDesc,
                    SortMode::DecDotDesc,
                    SortMode::LengthDesc,
                ] {
                    self.item(ui, App::sort_mode_label(m), C::Sort(m), out);
                }
                ui.separator();
                for m in [SortMode::Reverse, SortMode::Random] {
                    self.item(ui, App::sort_mode_label(m), C::Sort(m), out);
                }
            });
            ui.menu_button("Indent", |ui| {
                self.items(
                    ui,
                    &[
                        ("Increase Indent", C::IndentIncrease),
                        ("Decrease Indent", C::IndentDecrease),
                    ],
                    out,
                );
            });
            ui.menu_button("Convert Case", |ui| {
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
            ui.menu_button("Comment", |ui| {
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
            ui.menu_button("Whitespace", |ui| {
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
            ui.menu_button("Auto-Completion", |ui| {
                self.items(
                    ui,
                    &[
                        ("Complete Keyword", C::FunctionCompletion),
                        ("Complete Word", C::WordCompletion),
                    ],
                    out,
                );
            });
            ui.separator();
            ui.menu_button("Selected Text", |ui| {
                self.items(
                    ui,
                    &[
                        ("Open as File", C::OpenSelectedFile),
                        ("Search on the Web", C::SearchOnInternet),
                        ("Redact", C::Redact),
                    ],
                    out,
                );
            });
            let ro = self.active_doc().is_some_and(|d| d.read_only);
            self.check(ui, "Read-Only", ro, C::ToggleReadOnly, out);
        });

        ui.menu_button("Search", |ui| {
            self.items(
                ui,
                &[
                    ("Find...", C::Find),
                    ("Replace...", C::Replace),
                    ("Find in Files...", C::FindInFiles),
                    ("Incremental Search", C::IncrementalSearch),
                    ("Mark...", C::Mark),
                    ("Filter Lines...", C::FilterLines),
                ],
                out,
            );
            ui.separator();
            self.items(
                ui,
                &[("Find Next", C::FindNext), ("Find Previous", C::FindPrev)],
                out,
            );
            ui.menu_button("Find Selected Text", |ui| {
                self.items(
                    ui,
                    &[
                        ("Select and Find Next", C::SelectAndFindNext),
                        ("Select and Find Previous", C::SelectAndFindPrev),
                        ("Find Next (Keep Search Text)", C::VolatileFindNext),
                        ("Find Previous (Keep Search Text)", C::VolatileFindPrev),
                    ],
                    out,
                );
            });
            ui.separator();
            self.items(
                ui,
                &[
                    ("Search Results Panel", C::SearchResultsWindow),
                    ("Next Search Result", C::NextSearchResult),
                    ("Previous Search Result", C::PrevSearchResult),
                ],
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
            ui.menu_button("Highlight", |ui| {
                ui.menu_button("Highlight All Occurrences", |ui| {
                    for (i, s) in (1u8..).zip(styles) {
                        self.item(ui, s, C::StyleAll(i), out);
                    }
                });
                ui.menu_button("Highlight This Occurrence", |ui| {
                    for (i, s) in (1u8..).zip(styles) {
                        self.item(ui, s, C::StyleOne(i), out);
                    }
                });
                ui.separator();
                ui.menu_button("Next Highlight", |ui| {
                    for (i, s) in (1u8..).zip(styles) {
                        self.item(ui, s, C::JumpNextStyle(i), out);
                    }
                    self.item(ui, "Find Marks", C::JumpNextStyle(0), out);
                });
                ui.menu_button("Previous Highlight", |ui| {
                    for (i, s) in (1u8..).zip(styles) {
                        self.item(ui, s, C::JumpPrevStyle(i), out);
                    }
                    self.item(ui, "Find Marks", C::JumpPrevStyle(0), out);
                });
                ui.menu_button("Copy Highlighted Text", |ui| {
                    for (i, s) in (1u8..).zip(styles) {
                        self.item(ui, s, C::CopyStyledText(i), out);
                    }
                    self.item(ui, "All Styles", C::CopyStyledText(0), out);
                });
                ui.separator();
                ui.menu_button("Clear Highlight", |ui| {
                    for (i, s) in (1u8..).zip(styles) {
                        self.item(ui, s, C::ClearStyle(i), out);
                    }
                });
                self.item(ui, "Clear All Highlights", C::ClearAllStyles, out);
            });
            ui.menu_button("Bookmarks", |ui| {
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
            ui.menu_button("Change History", |ui| {
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
            ui.separator();
            self.item(ui, "Find Characters in Range...", C::FindCharsInRange, out);
        });

        ui.menu_button("View", |ui| {
            let s = &self.settings;
            let (wrap, hl, fs) = (s.word_wrap, s.syntax_highlighting, s.font_size);
            self.check(ui, "Word Wrap", wrap, C::WordWrap, out);
            self.check(ui, "Syntax Highlighting", hl, C::SyntaxHighlighting, out);
            ui.menu_button("Font Size", |ui| {
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
            ui.menu_button("Show Symbols", |ui| {
                let s = &self.settings;
                let (ws, eol, ig, wsym) = (
                    s.show_whitespace,
                    s.show_eol,
                    s.show_indent_guides,
                    s.show_wrap_symbol,
                );
                self.check(ui, "Spaces and Tabs", ws, C::ShowWhitespace, out);
                self.check(ui, "Line Endings", eol, C::ShowEol, out);
                self.check(ui, "All Characters", ws && eol, C::ShowAllChars, out);
                ui.separator();
                self.check(ui, "Indent Guides", ig, C::ShowIndentGuides, out);
                self.check(ui, "Wrap Symbol", wsym, C::ShowWrapSymbol, out);
            });
            ui.menu_button("Folding", |ui| {
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
                ui.menu_button("Fold Level", |ui| {
                    for i in 1..=8 {
                        self.item(ui, &i.to_string(), C::FoldLevel(i), out);
                    }
                });
                ui.menu_button("Unfold Level", |ui| {
                    for i in 1..=8 {
                        self.item(ui, &i.to_string(), C::UnfoldLevel(i), out);
                    }
                });
            });
            ui.separator();
            let s = &self.settings;
            let panels = [
                ("Document Map", s.show_doc_map, C::ToggleDocMap),
                ("Document List", s.show_doc_list, C::ToggleDocList),
                ("Function List", s.show_function_list, C::ToggleFunctionList),
                ("JSON Tree", s.show_json_tree, C::ToggleJsonTree),
                (
                    "Folder as Workspace",
                    s.show_folder_tree,
                    C::ToggleFolderTree,
                ),
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
            ui.separator();
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
            ui.separator();
            self.item(ui, "Document Summary...", C::Summary, out);
        });

        ui.menu_button("Encoding", |ui| {
            let cur = self.active_doc().map(|d| d.lang);
            let normal = self.langs.normal();
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
            ui.separator();
            let cur = self.active_doc().map(|d| d.encoding);
            let main = [
                TextEncoding::Ansi,
                TextEncoding::Utf8,
                TextEncoding::Utf8Bom,
                TextEncoding::Utf16BeBom,
                TextEncoding::Utf16LeBom,
            ];
            ui.menu_button("Encoding", |ui| {
                for e in main {
                    self.check(ui, &e.label(), cur == Some(e), C::SetEncoding(e.key()), out);
                }
            });
            ui.menu_button("Character Sets", |ui| {
                let mut groups: Vec<&str> = CHARSETS.iter().map(|c| c.0).collect();
                groups.dedup();
                for g in groups {
                    ui.menu_button(g, |ui| {
                        for c in CHARSETS.iter().filter(|c| c.0 == g) {
                            let e = TextEncoding::Charset(c.2);
                            self.check(ui, c.1, cur == Some(e), C::SetEncoding(e.key()), out);
                        }
                    });
                }
            });
            ui.menu_button("Convert to", |ui| {
                for e in main {
                    self.item(ui, &e.label(), C::ConvertEncoding(e.key()), out);
                }
            });
            ui.menu_button("Line Endings", |ui| {
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
        });

        ui.menu_button("Tools", |ui| {
            ui.menu_button("Format", |ui| {
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
                if ui.button("Formatter Options...").clicked() {
                    self.dialogs.preferences = true;
                    self.dialogs.pref_tab = 7;
                    ui.close();
                }
            });
            ui.menu_button("Encode / Decode", |ui| {
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
            ui.menu_button("Data", |ui| {
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
            ui.menu_button("Hash", |ui| {
                for algo in [
                    HashAlgo::Md5,
                    HashAlgo::Sha1,
                    HashAlgo::Sha256,
                    HashAlgo::Sha512,
                ] {
                    ui.menu_button(algo.name(), |ui| {
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
            self.item(ui, "Date and Unix Time...", C::DateTimeTool, out);
        });

        ui.menu_button("Compare", |ui| {
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
            ui.menu_button("Options", |ui| {
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
                ui.separator();
                let label = match &o.ignore_pattern {
                    Some(p) => format!("Ignore Pattern: {p}"),
                    None => "Ignore Text Matching the Find Box (Regex)".into(),
                };
                self.check(
                    ui,
                    &label,
                    o.ignore_pattern.is_some(),
                    C::CompareIgnoreFindText,
                    out,
                );
                if o.ignore_pattern.is_some() {
                    self.item(ui, "Clear Ignore Pattern", C::CompareClearIgnore, out);
                }
            });
        });

        ui.menu_button("Window", |ui| {
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
            self.items(
                ui,
                &[
                    ("Move to Other View", C::MoveToOtherView),
                    ("Clone to Other View", C::CloneToOtherView),
                    ("Move to New Window", C::MoveToNewWindow),
                    ("Switch to Other View", C::FocusOtherView),
                ],
                out,
            );
            let (sv, sh) = (self.settings.sync_scroll_v, self.settings.sync_scroll_h);
            self.check(ui, "Sync Vertical Scrolling", sv, C::SyncScrollV, out);
            self.check(ui, "Sync Horizontal Scrolling", sh, C::SyncScrollH, out);
            ui.separator();
            ui.menu_button("Tabs", |ui| {
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
                ui.menu_button("Tab Color", |ui| {
                    for i in 0..5u8 {
                        self.item(ui, &format!("Color {}", i + 1), C::TabColor(Some(i)), out);
                    }
                    self.item(ui, "No Color", C::TabColor(None), out);
                });
                ui.menu_button("Sort Tabs", |ui| {
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
                    let label = format!(
                        "{}{}: {}",
                        if on { "• " } else { "    " },
                        i + 1,
                        d.display_name()
                    );
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
            let mut b = |ui: &mut Ui, icon: Icon, tip: &str, cmd: Command, on: bool| {
                let sc = shortcut_for(&self.shortcuts, &cmd)
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
                    out.push(cmd);
                }
            };
            b(ui, Icon::New, "New", Command::New, false);
            b(ui, Icon::Open, "Open", Command::Open, false);
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
            b(
                ui,
                Icon::SyncV,
                "Sync Vertical Scrolling",
                Command::SyncScrollV,
                self.settings.sync_scroll_v,
            );
            b(
                ui,
                Icon::SyncH,
                "Sync Horizontal Scrolling",
                Command::SyncScrollH,
                self.settings.sync_scroll_h,
            );
            ui.separator();
            b(
                ui,
                Icon::Wrap,
                "Word Wrap",
                Command::WordWrap,
                self.settings.word_wrap,
            );
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
            b(
                ui,
                Icon::IndentGuide,
                "Show Indent Guides",
                Command::ShowIndentGuides,
                self.settings.show_indent_guides,
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
            b(
                ui,
                Icon::Folder,
                "Folder as Workspace",
                Command::ToggleFolderTree,
                self.settings.show_folder_tree,
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
        });
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
                    let key = egui::Id::unique("sync_prev");
                    let prev: Option<(f64, f32)> = ui.ctx().data(|d| d.get_temp(key));
                    if let Some((pt, px)) = prev
                        && let Some(dv) = self.views.get_mut(&dst)
                    {
                        if self.settings.sync_scroll_v {
                            dv.top_row = (dv.top_row + (v.0 - pt)).max(0.0);
                        }
                        if self.settings.sync_scroll_h {
                            dv.scroll_x = (dv.scroll_x + (v.1 - px)).max(0.0);
                        }
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(key, v));
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
        if o.clicked || o.response.as_ref().is_some_and(egui::Response::has_focus) {
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
        bar.show(ui, |ui| {
            egui::ScrollArea::horizontal()
                .id_salt(("tabs", pane))
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
                            let (rect, resp) =
                                ui.allocate_exact_size(vec2(w, 24.0), Sense::click_and_drag());
                            let is_active = i == active;
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
                                    egui::Rect::from_min_size(rect.min, vec2(rect.width(), 3.0)),
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
                                p.rect_filled(lr, CornerRadius::same(1), Color32::from_gray(130));
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
                                if hover_close && !pinned && self.settings.tab_close_button {
                                    close = Some(i);
                                } else {
                                    activate = Some(i);
                                }
                            }
                            if resp.middle_clicked()
                                || (resp.double_clicked() && self.settings.double_click_close_tab)
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
                                let items: Vec<(&str, Command)> = vec![
                                    ("Close", Command::Close),
                                    ("Close All but This", Command::CloseAllButCurrent),
                                    ("Close All to the Left", Command::CloseAllToLeft),
                                    ("Close All to the Right", Command::CloseAllToRight),
                                    ("Close All Unchanged", Command::CloseAllUnchanged),
                                    ("Save", Command::Save),
                                    ("Save As...", Command::SaveAs),
                                    ("Rename...", Command::Rename),
                                    ("Reload from Disk", Command::Reload),
                                    (
                                        if cfg!(target_os = "macos") {
                                            "Show in Finder"
                                        } else {
                                            "Show in Explorer"
                                        },
                                        Command::OpenContainingFolder,
                                    ),
                                    ("Copy Full File Path", Command::CopyFullPath),
                                    ("Copy File Name", Command::CopyFileName),
                                    ("Copy Folder Path", Command::CopyDirPath),
                                    ("Read-Only", Command::ToggleReadOnly),
                                    ("Pin/Unpin Tab", Command::TogglePin),
                                    ("Move to Other View", Command::MoveToOtherView),
                                    ("Clone to Other View", Command::CloneToOtherView),
                                    ("Move to New Window", Command::MoveToNewWindow),
                                ];
                                if i != active
                                    && let Some(&cur) = tabs.get(active)
                                    && ui.button("Compare with this tab").clicked()
                                {
                                    ctx_cmds.push((usize::MAX, Command::CompareWith(cur, *id)));
                                    ui.close();
                                }
                                for (label, cmd) in items {
                                    if ui.button(label).clicked() {
                                        ctx_cmds.push((i, cmd));
                                        ui.close();
                                    }
                                }
                                ui.menu_button("Tab Color", |ui| {
                                    for c in 0..5u8 {
                                        let txt = RichText::new(format!("■ Color {}", c + 1))
                                            .color(TAB_COLORS[usize::from(c)]);
                                        if ui.button(txt).clicked() {
                                            ctx_cmds.push((i, Command::TabColor(Some(c))));
                                            ui.close();
                                        }
                                    }
                                    if ui.button("No Color").clicked() {
                                        ctx_cmds.push((i, Command::TabColor(None)));
                                        ui.close();
                                    }
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
        });
        app.run(crate::commands::Command::DateTimeTool);
        app.dialogs.style_config = true;
        app.dialogs.shortcut_mapper = true;
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
            });
            app.settings.syntax_highlighting = highlight;
            app.settings.show_function_list = std::env::var_os("BENCH_FUNCLIST").is_some();
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
