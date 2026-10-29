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
        if self.egui_ctx.is_none() {
            self.egui_ctx = Some(ctx.clone());
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
        }
        let now = ctx.input(|i| i.time);
        self.poll(now);
        self.apply_pending_goto();
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

    #[allow(clippy::too_many_lines)]
    fn menu_bar(&mut self, ui: &mut Ui, out: &mut Vec<Command>) {
        ui.menu_button("File", |ui| {
            self.item(ui, "New", Command::New, out);
            self.item(ui, "Open...", Command::Open, out);
            ui.menu_button("Open Containing Folder", |ui| {
                self.item(
                    ui,
                    if cfg!(target_os = "macos") {
                        "Finder"
                    } else {
                        "Explorer"
                    },
                    Command::OpenContainingFolder,
                    out,
                );
                self.item(
                    ui,
                    if cfg!(target_os = "macos") {
                        "Terminal"
                    } else {
                        "cmd"
                    },
                    Command::OpenTerminalHere,
                    out,
                );
            });
            self.item(
                ui,
                "Open in Default Viewer",
                Command::OpenInDefaultViewer,
                out,
            );
            self.item(
                ui,
                "Open Folder as Workspace...",
                Command::OpenFolderAsWorkspace,
                out,
            );
            self.item(ui, "Reload from Disk", Command::Reload, out);
            self.item(ui, "Save", Command::Save, out);
            self.item(ui, "Save As...", Command::SaveAs, out);
            self.item(ui, "Save a Copy As...", Command::SaveCopyAs, out);
            self.item(ui, "Save All", Command::SaveAll, out);
            self.item(ui, "Rename...", Command::Rename, out);
            self.item(ui, "Close", Command::Close, out);
            self.item(ui, "Close All", Command::CloseAll, out);
            ui.menu_button("Close Multiple Documents", |ui| {
                self.item(ui, "Close All BUT This", Command::CloseAllButCurrent, out);
                self.item(ui, "Close All BUT Pinned", Command::CloseAllButPinned, out);
                self.item(ui, "Close All to the Left", Command::CloseAllToLeft, out);
                self.item(ui, "Close All to the Right", Command::CloseAllToRight, out);
                self.item(ui, "Close All Unchanged", Command::CloseAllUnchanged, out);
            });
            ui.separator();
            self.item(ui, "Load Session...", Command::LoadSession, out);
            self.item(ui, "Save Session...", Command::SaveSession, out);
            ui.separator();
            for (i, f) in self
                .persisted
                .recent_files
                .iter()
                .take(self.settings.recent_max)
                .enumerate()
            {
                if ui.button(format!("{}: {f}", i + 1)).clicked() {
                    out.push(Command::OpenRecent(f.clone()));
                    ui.close();
                }
            }
            if !self.persisted.recent_files.is_empty() {
                self.item(
                    ui,
                    "Restore Recent Closed File",
                    Command::RestoreLastClosed,
                    out,
                );
                self.item(ui, "Open All Recent Files", Command::OpenAllRecent, out);
                self.item(ui, "Empty Recent Files List", Command::ClearRecent, out);
                ui.separator();
            }
            self.item(ui, "Exit", Command::Exit, out);
        });
        ui.menu_button("Edit", |ui| {
            self.item(ui, "Undo", Command::Undo, out);
            self.item(ui, "Redo", Command::Redo, out);
            ui.separator();
            let cmd = if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl+"
            };
            for (l, c, k) in [
                ("Cut", Command::Cut, "X"),
                ("Copy", Command::Copy, "C"),
                ("Paste", Command::Paste, "V"),
            ] {
                if ui
                    .add(egui::Button::new(l).shortcut_text(format!("{cmd}{k}")))
                    .clicked()
                {
                    out.push(c);
                    ui.close();
                }
            }
            self.item(ui, "Delete", Command::DeleteSelection, out);
            self.item(ui, "Select All", Command::SelectAll, out);
            self.item(ui, "Begin/End Select", Command::BeginEndSelect, out);
            self.item(
                ui,
                "Begin/End Select in Column Mode",
                Command::BeginEndSelectColumn,
                out,
            );
            ui.separator();
            ui.menu_button("Insert", |ui| {
                self.item(ui, "Date Time (short)", Command::InsertDateTimeShort, out);
                self.item(ui, "Date Time (long)", Command::InsertDateTimeLong, out);
                self.item(
                    ui,
                    "Date Time (customized)",
                    Command::InsertDateTimeCustom,
                    out,
                );
            });
            ui.menu_button("Copy to Clipboard", |ui| {
                self.item(
                    ui,
                    "Copy Current Full File Path",
                    Command::CopyFullPath,
                    out,
                );
                self.item(ui, "Copy Current Filename", Command::CopyFileName, out);
                self.item(ui, "Copy Current Dir. Path", Command::CopyDirPath, out);
                self.item(ui, "Copy All Filenames", Command::CopyAllNames, out);
                self.item(ui, "Copy All File Paths", Command::CopyAllPaths, out);
            });
            ui.menu_button("Indent", |ui| {
                self.item(ui, "Increase Line Indent", Command::IndentIncrease, out);
                self.item(ui, "Decrease Line Indent", Command::IndentDecrease, out);
            });
            ui.menu_button("Convert Case to", |ui| {
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
                    self.item(ui, App::case_label(m), Command::ConvertCase(m), out);
                }
            });
            ui.menu_button("Line Operations", |ui| {
                self.item(ui, "Duplicate Current Line", Command::DuplicateLine, out);
                self.item(
                    ui,
                    "Remove Duplicate Lines",
                    Command::RemoveDuplicateLines,
                    out,
                );
                self.item(
                    ui,
                    "Remove Consecutive Duplicate Lines",
                    Command::RemoveConsecutiveDuplicates,
                    out,
                );
                self.item(ui, "Split Lines", Command::SplitLines, out);
                self.item(ui, "Join Lines", Command::JoinLines, out);
                self.item(ui, "Move Up Current Line", Command::MoveLineUp, out);
                self.item(ui, "Move Down Current Line", Command::MoveLineDown, out);
                self.item(ui, "Remove Empty Lines", Command::RemoveEmptyLines, out);
                self.item(
                    ui,
                    "Remove Empty Lines (Containing Blank characters)",
                    Command::RemoveEmptyLinesWithBlank,
                    out,
                );
                self.item(
                    ui,
                    "Insert Blank Line Above Current",
                    Command::InsertBlankAbove,
                    out,
                );
                self.item(
                    ui,
                    "Insert Blank Line Below Current",
                    Command::InsertBlankBelow,
                    out,
                );
                self.item(ui, "Delete Current Line", Command::DeleteLine, out);
                self.item(
                    ui,
                    "Transpose Current and Previous Lines",
                    Command::TransposeLine,
                    out,
                );
                ui.separator();
                for m in [
                    SortMode::LexAsc,
                    SortMode::LexCiAsc,
                    SortMode::LocaleAsc,
                    SortMode::IntAsc,
                    SortMode::DecCommaAsc,
                    SortMode::DecDotAsc,
                    SortMode::LengthAsc,
                    SortMode::LexDesc,
                    SortMode::LexCiDesc,
                    SortMode::LocaleDesc,
                    SortMode::IntDesc,
                    SortMode::DecCommaDesc,
                    SortMode::DecDotDesc,
                    SortMode::LengthDesc,
                    SortMode::Reverse,
                    SortMode::Random,
                ] {
                    self.item(ui, App::sort_mode_label(m), Command::Sort(m), out);
                }
            });
            ui.menu_button("Comment/Uncomment", |ui| {
                self.item(
                    ui,
                    "Toggle Single Line Comment",
                    Command::ToggleComment,
                    out,
                );
                self.item(ui, "Single Line Comment", Command::BlockComment, out);
                self.item(ui, "Single Line Uncomment", Command::BlockUncomment, out);
                self.item(ui, "Block Comment", Command::StreamComment, out);
                self.item(ui, "Block Uncomment", Command::StreamUncomment, out);
            });
            ui.menu_button("Auto-Completion", |ui| {
                self.item(ui, "Function Completion", Command::FunctionCompletion, out);
                self.item(ui, "Word Completion", Command::WordCompletion, out);
            });
            ui.menu_button("EOL Conversion", |ui| {
                let eol = self.active_doc().map(|d| d.eol);
                self.check(
                    ui,
                    "Windows (CR LF)",
                    eol == Some(crate::encoding::Eol::Crlf),
                    Command::EolWindows,
                    out,
                );
                self.check(
                    ui,
                    "Unix (LF)",
                    eol == Some(crate::encoding::Eol::Lf),
                    Command::EolUnix,
                    out,
                );
                self.check(
                    ui,
                    "Macintosh (CR)",
                    eol == Some(crate::encoding::Eol::Cr),
                    Command::EolMac,
                    out,
                );
            });
            ui.menu_button("Blank Operations", |ui| {
                self.item(ui, "Trim Trailing Space", Command::TrimTrailing, out);
                self.item(ui, "Trim Leading Space", Command::TrimLeading, out);
                self.item(
                    ui,
                    "Trim Leading and Trailing Space",
                    Command::TrimBoth,
                    out,
                );
                self.item(ui, "EOL to Space", Command::EolToSpace, out);
                self.item(
                    ui,
                    "Trim both and EOL to Space",
                    Command::TrimAndEolToSpace,
                    out,
                );
                ui.separator();
                self.item(ui, "TAB to Space", Command::TabToSpace, out);
                self.item(ui, "Space to TAB (All)", Command::SpaceToTabAll, out);
                self.item(
                    ui,
                    "Space to TAB (Leading)",
                    Command::SpaceToTabLeading,
                    out,
                );
            });
            ui.menu_button("On Selection", |ui| {
                self.item(ui, "Open File", Command::OpenSelectedFile, out);
                self.item(ui, "Search on Internet", Command::SearchOnInternet, out);
            });
            ui.menu_button("Multi-select All", |ui| {
                self.item(
                    ui,
                    "Ignore Case & Whole Word",
                    Command::MultiSelectAll {
                        match_case: false,
                        whole_word: false,
                    },
                    out,
                );
                self.item(
                    ui,
                    "Match Case Only",
                    Command::MultiSelectAll {
                        match_case: true,
                        whole_word: false,
                    },
                    out,
                );
                self.item(
                    ui,
                    "Match Whole Word Only",
                    Command::MultiSelectAll {
                        match_case: false,
                        whole_word: true,
                    },
                    out,
                );
                self.item(
                    ui,
                    "Match Case & Whole Word",
                    Command::MultiSelectAll {
                        match_case: true,
                        whole_word: true,
                    },
                    out,
                );
            });
            ui.menu_button("Multi-select Next", |ui| {
                self.item(
                    ui,
                    "Ignore Case & Whole Word",
                    Command::MultiSelectNext {
                        match_case: false,
                        whole_word: false,
                    },
                    out,
                );
                self.item(
                    ui,
                    "Match Case Only",
                    Command::MultiSelectNext {
                        match_case: true,
                        whole_word: false,
                    },
                    out,
                );
                self.item(
                    ui,
                    "Match Whole Word Only",
                    Command::MultiSelectNext {
                        match_case: false,
                        whole_word: true,
                    },
                    out,
                );
                self.item(
                    ui,
                    "Match Case & Whole Word",
                    Command::MultiSelectNext {
                        match_case: true,
                        whole_word: true,
                    },
                    out,
                );
                self.item(
                    ui,
                    "Undo the Latest Added Multi-Select",
                    Command::MultiSelectUndo,
                    out,
                );
                self.item(
                    ui,
                    "Skip Current & Go to Next Multi-select",
                    Command::MultiSelectSkip,
                    out,
                );
            });
            ui.separator();
            self.item(ui, "Column Editor...", Command::ColumnEditor, out);
            self.check(
                ui,
                "Character Panel",
                self.settings.show_char_panel,
                Command::CharacterPanel,
                out,
            );
            self.check(
                ui,
                "Clipboard History",
                self.settings.show_clipboard_history,
                Command::ClipboardHistory,
                out,
            );
            ui.separator();
            let ro = self.active_doc().is_some_and(|d| d.read_only);
            self.check(ui, "Read-Only", ro, Command::ToggleReadOnly, out);
            self.item(ui, "Redact Selection", Command::Redact, out);
        });
        ui.menu_button("Search", |ui| {
            self.item(ui, "Find...", Command::Find, out);
            self.item(ui, "Find in Files...", Command::FindInFiles, out);
            self.item(ui, "Find Next", Command::FindNext, out);
            self.item(ui, "Find Previous", Command::FindPrev, out);
            self.item(ui, "Select and Find Next", Command::SelectAndFindNext, out);
            self.item(
                ui,
                "Select and Find Previous",
                Command::SelectAndFindPrev,
                out,
            );
            self.item(ui, "Find (Volatile) Next", Command::VolatileFindNext, out);
            self.item(
                ui,
                "Find (Volatile) Previous",
                Command::VolatileFindPrev,
                out,
            );
            self.item(ui, "Replace...", Command::Replace, out);
            self.item(ui, "Incremental Search", Command::IncrementalSearch, out);
            self.item(
                ui,
                "Search Results Window",
                Command::SearchResultsWindow,
                out,
            );
            self.item(ui, "Next Search Result", Command::NextSearchResult, out);
            self.item(ui, "Previous Search Result", Command::PrevSearchResult, out);
            self.item(ui, "Go to...", Command::GoTo, out);
            self.item(ui, "Go to Matching Brace", Command::GoToMatchingBrace, out);
            self.item(
                ui,
                "Select All In-between {} [] or ()",
                Command::SelectToMatchingBrace,
                out,
            );
            self.item(ui, "Mark...", Command::Mark, out);
            ui.separator();
            ui.menu_button("Change History", |ui| {
                self.item(ui, "Go to Next Change", Command::ChangeHistoryNext, out);
                self.item(ui, "Go to Previous Change", Command::ChangeHistoryPrev, out);
                self.item(ui, "Clear Change History", Command::ChangeHistoryClear, out);
            });
            let styles = ["1st", "2nd", "3rd", "4th", "5th"];
            ui.menu_button("Style All Occurrences of Token", |ui| {
                for (i, s) in styles.iter().enumerate() {
                    self.item(
                        ui,
                        &format!("Using {s} Style"),
                        Command::StyleAll(i as u8 + 1),
                        out,
                    );
                }
            });
            ui.menu_button("Style One Token", |ui| {
                for (i, s) in styles.iter().enumerate() {
                    self.item(
                        ui,
                        &format!("Using {s} Style"),
                        Command::StyleOne(i as u8 + 1),
                        out,
                    );
                }
            });
            ui.menu_button("Clear Style", |ui| {
                for (i, s) in styles.iter().enumerate() {
                    self.item(
                        ui,
                        &format!("Clear {s} Style"),
                        Command::ClearStyle(i as u8 + 1),
                        out,
                    );
                }
                self.item(ui, "Clear all Styles", Command::ClearAllStyles, out);
            });
            ui.menu_button("Jump up", |ui| {
                for (i, s) in styles.iter().enumerate() {
                    self.item(
                        ui,
                        &format!("{s} Style"),
                        Command::JumpPrevStyle(i as u8 + 1),
                        out,
                    );
                }
                self.item(ui, "Find Mark Style", Command::JumpPrevStyle(0), out);
            });
            ui.menu_button("Jump down", |ui| {
                for (i, s) in styles.iter().enumerate() {
                    self.item(
                        ui,
                        &format!("{s} Style"),
                        Command::JumpNextStyle(i as u8 + 1),
                        out,
                    );
                }
                self.item(ui, "Find Mark Style", Command::JumpNextStyle(0), out);
            });
            ui.menu_button("Copy Styled Text", |ui| {
                for (i, s) in styles.iter().enumerate() {
                    self.item(
                        ui,
                        &format!("{s} Style"),
                        Command::CopyStyledText(i as u8 + 1),
                        out,
                    );
                }
                self.item(ui, "All Styles", Command::CopyStyledText(0), out);
            });
            ui.menu_button("Bookmark", |ui| {
                self.item(ui, "Toggle Bookmark", Command::ToggleBookmark, out);
                self.item(ui, "Next Bookmark", Command::NextBookmark, out);
                self.item(ui, "Previous Bookmark", Command::PrevBookmark, out);
                self.item(ui, "Clear All Bookmarks", Command::ClearBookmarks, out);
                self.item(ui, "Cut Bookmarked Lines", Command::CutBookmarkedLines, out);
                self.item(
                    ui,
                    "Copy Bookmarked Lines",
                    Command::CopyBookmarkedLines,
                    out,
                );
                self.item(
                    ui,
                    "Paste to (Replace) Bookmarked Lines",
                    Command::PasteToBookmarkedLines,
                    out,
                );
                self.item(
                    ui,
                    "Remove Bookmarked Lines",
                    Command::RemoveBookmarkedLines,
                    out,
                );
                self.item(
                    ui,
                    "Remove Non-Bookmarked Lines",
                    Command::RemoveUnbookmarkedLines,
                    out,
                );
                self.item(ui, "Inverse Bookmark", Command::InverseBookmarks, out);
            });
            ui.separator();
            self.item(
                ui,
                "Find characters in range...",
                Command::FindCharsInRange,
                out,
            );
        });
        ui.menu_button("View", |ui| {
            self.check(
                ui,
                "Always on Top",
                self.always_on_top,
                Command::AlwaysOnTop,
                out,
            );
            self.check(
                ui,
                "Toggle Full Screen Mode",
                self.fullscreen,
                Command::FullScreen,
                out,
            );
            self.check(ui, "Post-It", self.postit, Command::PostIt, out);
            self.item(ui, "Distraction Free Mode", Command::DistractionFree, out);
            ui.separator();
            ui.menu_button("View Current File in", |ui| {
                self.item(ui, "Default Viewer / Browser", Command::ViewInBrowser, out);
            });
            ui.separator();
            ui.menu_button("Show Symbol", |ui| {
                self.check(
                    ui,
                    "Show Space and Tab",
                    self.settings.show_whitespace,
                    Command::ShowWhitespace,
                    out,
                );
                self.check(
                    ui,
                    "Show End of Line",
                    self.settings.show_eol,
                    Command::ShowEol,
                    out,
                );
                self.check(
                    ui,
                    "Show All Characters",
                    self.settings.show_whitespace && self.settings.show_eol,
                    Command::ShowAllChars,
                    out,
                );
                ui.separator();
                self.check(
                    ui,
                    "Show Indent Guide",
                    self.settings.show_indent_guides,
                    Command::ShowIndentGuides,
                    out,
                );
                self.check(
                    ui,
                    "Show Wrap Symbol",
                    self.settings.show_wrap_symbol,
                    Command::ShowWrapSymbol,
                    out,
                );
            });
            ui.menu_button("Zoom", |ui| {
                self.item(ui, "Zoom In", Command::ZoomIn, out);
                self.item(ui, "Zoom Out", Command::ZoomOut, out);
                self.item(ui, "Restore Default Zoom", Command::ZoomRestore, out);
            });
            ui.menu_button("Move/Clone Current Document", |ui| {
                self.item(ui, "Move to Other View", Command::MoveToOtherView, out);
                self.item(ui, "Clone to Other View", Command::CloneToOtherView, out);
            });
            ui.menu_button("Tab", |ui| {
                for i in 0..9 {
                    self.item(
                        ui,
                        &format!(
                            "{}{} Tab",
                            i + 1,
                            match i {
                                0 => "st",
                                1 => "nd",
                                2 => "rd",
                                _ => "th",
                            }
                        ),
                        Command::GoToTab(i),
                        out,
                    );
                }
                self.item(ui, "Next Tab", Command::NextTab, out);
                self.item(ui, "Previous Tab", Command::PrevTab, out);
                self.item(ui, "Move Tab Forward", Command::MoveTabForward, out);
                self.item(ui, "Move Tab Backward", Command::MoveTabBackward, out);
                ui.menu_button("Apply Color to Tab", |ui| {
                    for i in 0..5u8 {
                        self.item(
                            ui,
                            &format!("Apply Color {}", i + 1),
                            Command::TabColor(Some(i)),
                            out,
                        );
                    }
                    self.item(ui, "Remove Color", Command::TabColor(None), out);
                });
                self.item(ui, "Pin/Unpin Tab", Command::TogglePin, out);
            });
            self.check(
                ui,
                "Word wrap",
                self.settings.word_wrap,
                Command::WordWrap,
                out,
            );
            self.item(ui, "Focus on Another View", Command::FocusOtherView, out);
            ui.separator();
            self.item(ui, "Fold All", Command::FoldAll, out);
            self.item(ui, "Unfold All", Command::UnfoldAll, out);
            self.item(ui, "Collapse Current Level", Command::FoldCurrent, out);
            self.item(ui, "Uncollapse Current Level", Command::UnfoldCurrent, out);
            ui.menu_button("Collapse Level", |ui| {
                for i in 1..=8 {
                    self.item(ui, &i.to_string(), Command::FoldLevel(i), out);
                }
            });
            ui.menu_button("Uncollapse Level", |ui| {
                for i in 1..=8 {
                    self.item(ui, &i.to_string(), Command::UnfoldLevel(i), out);
                }
            });
            ui.separator();
            self.item(ui, "Summary...", Command::Summary, out);
            ui.separator();
            self.check(
                ui,
                "Folder as Workspace",
                self.settings.show_folder_tree,
                Command::ToggleFolderTree,
                out,
            );
            self.check(
                ui,
                "Document Map",
                self.settings.show_doc_map,
                Command::ToggleDocMap,
                out,
            );
            self.check(
                ui,
                "Document List",
                self.settings.show_doc_list,
                Command::ToggleDocList,
                out,
            );
            self.check(
                ui,
                "Function List",
                self.settings.show_function_list,
                Command::ToggleFunctionList,
                out,
            );
            ui.separator();
            self.check(
                ui,
                "Synchronize Vertical Scrolling",
                self.settings.sync_scroll_v,
                Command::SyncScrollV,
                out,
            );
            self.check(
                ui,
                "Synchronize Horizontal Scrolling",
                self.settings.sync_scroll_h,
                Command::SyncScrollH,
                out,
            );
            ui.separator();
            let mon = self.active_doc().is_some_and(|d| d.monitoring);
            self.check(ui, "Monitoring (tail -f)", mon, Command::Monitoring, out);
            ui.separator();
            self.check(
                ui,
                "Toolbar",
                self.settings.show_toolbar,
                Command::ToggleToolbar,
                out,
            );
            self.check(
                ui,
                "Status Bar",
                self.settings.show_statusbar,
                Command::ToggleStatusbar,
                out,
            );
        });
        ui.menu_button("Encoding", |ui| {
            let cur = self.active_doc().map(|d| d.encoding);
            for e in [
                TextEncoding::Ansi,
                TextEncoding::Utf8,
                TextEncoding::Utf8Bom,
                TextEncoding::Utf16BeBom,
                TextEncoding::Utf16LeBom,
            ] {
                self.check(
                    ui,
                    &e.label(),
                    cur == Some(e),
                    Command::SetEncoding(e.key()),
                    out,
                );
            }
            ui.menu_button("Character sets", |ui| {
                let mut groups: Vec<&str> = CHARSETS.iter().map(|c| c.0).collect();
                groups.dedup();
                for g in groups {
                    ui.menu_button(g, |ui| {
                        for c in CHARSETS.iter().filter(|c| c.0 == g) {
                            let e = TextEncoding::Charset(c.2);
                            self.check(ui, c.1, cur == Some(e), Command::SetEncoding(e.key()), out);
                        }
                    });
                }
            });
            ui.separator();
            for e in [
                TextEncoding::Ansi,
                TextEncoding::Utf8,
                TextEncoding::Utf8Bom,
                TextEncoding::Utf16BeBom,
                TextEncoding::Utf16LeBom,
            ] {
                self.item(
                    ui,
                    &format!("Convert to {}", e.label()),
                    Command::ConvertEncoding(e.key()),
                    out,
                );
            }
        });
        ui.menu_button("Language", |ui| {
            let cur = self.active_doc().map(|d| d.lang);
            let normal = self.langs.normal();
            self.check(
                ui,
                "None (Normal Text)",
                cur == Some(normal),
                Command::SetLanguage("normal".into()),
                out,
            );
            ui.separator();
            let sorted = self.langs.sorted_for_menu();
            let mut letters: Vec<char> = sorted
                .iter()
                .filter_map(|&i| {
                    self.langs.langs[i]
                        .display
                        .chars()
                        .next()
                        .map(|c| c.to_ascii_uppercase())
                })
                .collect();
            letters.dedup();
            for letter in letters {
                ui.menu_button(letter.to_string(), |ui| {
                    for &i in &sorted {
                        let l = &self.langs.langs[i];
                        if l.display.chars().next().map(|c| c.to_ascii_uppercase()) == Some(letter)
                        {
                            self.check(
                                ui,
                                &l.display,
                                cur == Some(i),
                                Command::SetLanguage(l.name.clone()),
                                out,
                            );
                        }
                    }
                });
            }
        });
        ui.menu_button("Settings", |ui| {
            self.item(ui, "Preferences...", Command::Preferences, out);
            self.item(ui, "Style Configurator...", Command::StyleConfigurator, out);
            self.item(ui, "Shortcut Mapper...", Command::ShortcutMapper, out);
        });
        ui.menu_button("Tools", |ui| {
            ui.menu_button("MIME Tools", |ui| {
                self.item(
                    ui,
                    "Base64 Encode",
                    Command::Mime(MimeOp::Base64Encode),
                    out,
                );
                self.item(
                    ui,
                    "Base64 Encode (each line)",
                    Command::Mime(MimeOp::Base64EncodeByLine),
                    out,
                );
                self.item(
                    ui,
                    "Base64 URL-safe Encode",
                    Command::Mime(MimeOp::Base64UrlEncode),
                    out,
                );
                self.item(
                    ui,
                    "Base64 Decode",
                    Command::Mime(MimeOp::Base64Decode),
                    out,
                );
                self.item(
                    ui,
                    "Base64 Decode (each line)",
                    Command::Mime(MimeOp::Base64DecodeByLine),
                    out,
                );
                ui.separator();
                self.item(ui, "URL Encode", Command::Mime(MimeOp::UrlEncode), out);
                self.item(ui, "URL Decode", Command::Mime(MimeOp::UrlDecode), out);
                self.item(
                    ui,
                    "URL Decode (+ as space)",
                    Command::Mime(MimeOp::UrlDecodeForm),
                    out,
                );
                ui.separator();
                self.item(
                    ui,
                    "Quoted-printable Encode",
                    Command::Mime(MimeOp::QpEncode),
                    out,
                );
                self.item(
                    ui,
                    "Quoted-printable Decode",
                    Command::Mime(MimeOp::QpDecode),
                    out,
                );
            });
            ui.menu_button("Format", |ui| {
                self.item(
                    ui,
                    "Format Document (auto-detect)",
                    Command::Format(FormatOp::Auto),
                    out,
                );
                ui.separator();
                self.item(
                    ui,
                    "JSON: Pretty Print",
                    Command::Format(FormatOp::JsonPretty),
                    out,
                );
                self.item(
                    ui,
                    "JSON: Minify",
                    Command::Format(FormatOp::JsonMinify),
                    out,
                );
                self.item(
                    ui,
                    "JSON: Validate",
                    Command::Format(FormatOp::JsonValidate),
                    out,
                );
                ui.separator();
                self.item(
                    ui,
                    "XML: Pretty Print",
                    Command::Format(FormatOp::XmlPretty),
                    out,
                );
                self.item(
                    ui,
                    "XML: Minify (linearize)",
                    Command::Format(FormatOp::XmlMinify),
                    out,
                );
                self.item(
                    ui,
                    "XML: Check Syntax",
                    Command::Format(FormatOp::XmlValidate),
                    out,
                );
                ui.separator();
                if ui.button("Formatter Options...").clicked() {
                    self.dialogs.preferences = true;
                    self.dialogs.pref_tab = 7;
                    ui.close();
                }
            });
            ui.separator();
            for algo in [
                HashAlgo::Md5,
                HashAlgo::Sha1,
                HashAlgo::Sha256,
                HashAlgo::Sha512,
            ] {
                ui.menu_button(algo.name(), |ui| {
                    self.item(
                        ui,
                        "Generate...",
                        Command::Hash(algo, HashSource::Text),
                        out,
                    );
                    self.item(
                        ui,
                        "Generate from files...",
                        Command::Hash(algo, HashSource::Files),
                        out,
                    );
                    self.item(
                        ui,
                        "Generate from selection into clipboard",
                        Command::Hash(algo, HashSource::SelectionToClipboard),
                        out,
                    );
                });
            }
        });
        ui.menu_button("Run", |ui| {
            self.item(ui, "Run...", Command::RunDialog, out);
            if !self.settings.run_commands.is_empty() {
                ui.separator();
                for (i, (name, _)) in self.settings.run_commands.iter().enumerate() {
                    if ui.button(name).clicked() {
                        out.push(Command::RunSaved(i));
                        ui.close();
                    }
                }
            }
            ui.separator();
            self.item(ui, "Launch in Default Browser", Command::ViewInBrowser, out);
        });
        ui.menu_button("Compare", |ui| {
            self.item(ui, "Compare (two views / next tab)", Command::Compare, out);
            self.item(ui, "Compare with Clipboard", Command::CompareClipboard, out);
            self.item(ui, "Compare to Last Save", Command::CompareSaved, out);
            ui.separator();
            let on = self.compare.is_some();
            ui.add_enabled_ui(on, |ui| {
                self.item(ui, "Next Difference", Command::CompareNext, out);
                self.item(ui, "Previous Difference", Command::ComparePrev, out);
                self.item(
                    ui,
                    "Copy Difference to Right",
                    Command::CompareCopyToRight,
                    out,
                );
                self.item(
                    ui,
                    "Copy Difference to Left",
                    Command::CompareCopyToLeft,
                    out,
                );
                self.item(ui, "Re-compare", Command::CompareRefresh, out);
                self.item(ui, "Clear Compare", Command::CompareClear, out);
            });
            ui.separator();
            let o = self.settings.compare.clone();
            self.check(
                ui,
                "Ignore Whitespace",
                o.ignore_whitespace,
                Command::CompareOption(0),
                out,
            );
            self.check(
                ui,
                "Ignore Case",
                o.ignore_case,
                Command::CompareOption(1),
                out,
            );
            self.check(
                ui,
                "Ignore Empty Lines",
                o.ignore_empty_lines,
                Command::CompareOption(2),
                out,
            );
            self.check(
                ui,
                "Highlight Changed Words",
                o.word_diff,
                Command::CompareOption(3),
                out,
            );
            self.check(
                ui,
                "Re-compare Automatically",
                o.auto,
                Command::CompareOption(4),
                out,
            );
        });
        ui.menu_button("Window", |ui| {
            self.item(ui, "Windows...", Command::WindowsDialog, out);
            ui.menu_button("Sort By", |ui| {
                self.item(ui, "Name A to Z", Command::SortTabs(TabSort::NameAsc), out);
                self.item(ui, "Name Z to A", Command::SortTabs(TabSort::NameDesc), out);
                self.item(ui, "Path A to Z", Command::SortTabs(TabSort::PathAsc), out);
                self.item(ui, "Path Z to A", Command::SortTabs(TabSort::PathDesc), out);
                self.item(ui, "Type A to Z", Command::SortTabs(TabSort::TypeAsc), out);
                self.item(ui, "Type Z to A", Command::SortTabs(TabSort::TypeDesc), out);
            });
            ui.separator();
            let pane = self.focused;
            for (i, id) in self.panes[pane].tabs.iter().enumerate().take(20) {
                if let Some(d) = self.docs.get(id) {
                    let on = i == self.panes[pane].active;
                    if ui
                        .button(format!(
                            "{}{}: {}",
                            if on { "• " } else { "    " },
                            i + 1,
                            d.display_name()
                        ))
                        .clicked()
                    {
                        out.push(Command::GoToTab(i));
                        ui.close();
                    }
                }
            }
        });
        ui.menu_button("?", |ui| {
            self.item(
                ui,
                "Command Line Arguments...",
                Command::CommandLineHelp,
                out,
            );
            self.item(ui, "Debug Info...", Command::DebugInfo, out);
            self.item(ui, &format!("About {APP_NAME}"), Command::About, out);
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
            b(ui, Icon::ZoomIn, "Zoom In", Command::ZoomIn, false);
            b(ui, Icon::ZoomOut, "Zoom Out", Command::ZoomOut, false);
            ui.separator();
            b(
                ui,
                Icon::SyncV,
                "Synchronize Vertical Scrolling",
                Command::SyncScrollV,
                self.settings.sync_scroll_v,
            );
            b(
                ui,
                Icon::SyncH,
                "Synchronize Horizontal Scrolling",
                Command::SyncScrollH,
                self.settings.sync_scroll_h,
            );
            ui.separator();
            b(
                ui,
                Icon::Wrap,
                "Word wrap",
                Command::WordWrap,
                self.settings.word_wrap,
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
                "Show Indent Guide",
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
            let mon = self.active_doc().is_some_and(|d| d.monitoring);
            b(
                ui,
                Icon::Monitor,
                "Monitoring (tail -f)",
                Command::Monitoring,
                mon,
            );
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
                ui.menu_button(eol.label(), |ui| {
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
                                    ("Close All BUT This", Command::CloseAllButCurrent),
                                    ("Close All to the Left", Command::CloseAllToLeft),
                                    ("Close All to the Right", Command::CloseAllToRight),
                                    ("Close All Unchanged", Command::CloseAllUnchanged),
                                    ("Save", Command::Save),
                                    ("Save As...", Command::SaveAs),
                                    ("Rename...", Command::Rename),
                                    ("Reload", Command::Reload),
                                    ("Open Containing Folder", Command::OpenContainingFolder),
                                    ("Copy Full File Path", Command::CopyFullPath),
                                    ("Copy Filename", Command::CopyFileName),
                                    ("Copy Current Dir. Path", Command::CopyDirPath),
                                    ("Read-Only", Command::ToggleReadOnly),
                                    ("Pin/Unpin Tab", Command::TogglePin),
                                    ("Move to Other View", Command::MoveToOtherView),
                                    ("Clone to Other View", Command::CloneToOtherView),
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
                                ui.menu_button("Apply Color to Tab", |ui| {
                                    for c in 0..5u8 {
                                        let txt = RichText::new(format!("■ Color {}", c + 1))
                                            .color(TAB_COLORS[usize::from(c)]);
                                        if ui.button(txt).clicked() {
                                            ctx_cmds.push((i, Command::TabColor(Some(c))));
                                            ui.close();
                                        }
                                    }
                                    if ui.button("Remove Color").clicked() {
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
