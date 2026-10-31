//! Dialog windows (FindReplaceDlg, GoToLineDlg, preferenceDlg, ...).

use egui::{Color32, Context, RichText, Ui};

use crate::app::{App, FindTab, Prompt, RenameTarget, StatusKind};
use crate::commands::Command;
use crate::document::Selection;
use crate::search::SearchMode;
use crate::textops::{self, CharRange, HashAlgo, LeadingMode, NumFormat};
use crate::ui::APP_NAME;

#[derive(Clone, Debug)]
pub struct ColumnEditorState {
    pub text_mode: bool,
    pub text: String,
    pub initial: String,
    pub increase: String,
    pub repeat: String,
    pub leading: LeadingMode,
    pub format: NumFormat,
}

impl Default for ColumnEditorState {
    fn default() -> Self {
        Self {
            text_mode: true,
            text: String::new(),
            initial: "1".into(),
            increase: "1".into(),
            repeat: "1".into(),
            leading: LeadingMode::None,
            format: NumFormat::Dec,
        }
    }
}

/// Opacity of the floating Find window while the pointer isn't over it.
const FIND_FADED_OPACITY: f32 = 0.7;

/// Sections of the Settings window, in order.
pub const SETTINGS_SECTIONS: [&str; 8] = [
    "General",
    "Appearance",
    "Editing",
    "New Documents",
    "Auto-Completion",
    "Files and Session",
    "Formatting",
    "Keyboard Shortcuts",
];

/// Indexes into [`SETTINGS_SECTIONS`].
pub mod settings_section {
    pub const GENERAL: usize = 0;
    pub const APPEARANCE: usize = 1;
    pub const EDITING: usize = 2;
    pub const NEW_DOCUMENTS: usize = 3;
    pub const COMPLETION: usize = 4;
    pub const FILES: usize = 5;
    pub const FORMATTING: usize = 6;
    pub const SHORTCUTS: usize = 7;
}

/// State of the "Date and Unix Time" tool: a date/time picker converted to
/// Unix time, and a Unix time converted back to a date.
#[derive(Clone, Debug)]
pub struct DateTimeDialog {
    /// Picker fields: year, month, day, hour, minute, second.
    pub parts: [i32; 6],
    /// The picker is in UTC rather than local time.
    pub utc: bool,
    /// Unix time to convert to a date (seconds, or milliseconds if 13+ digits).
    pub unix_input: String,
    /// A typed or pasted date (RFC 3339, `YYYY-MM-DD HH:MM:SS` or `YYYY-MM-DD`).
    pub date_input: String,
}

impl DateTimeDialog {
    pub fn now() -> Self {
        use chrono::{Datelike, Timelike};
        let n = chrono::Local::now();
        #[allow(clippy::cast_possible_wrap)]
        let parts = [
            n.year(),
            n.month() as i32,
            n.day() as i32,
            n.hour() as i32,
            n.minute() as i32,
            n.second() as i32,
        ];
        Self {
            parts,
            utc: false,
            unix_input: n.timestamp().to_string(),
            date_input: String::new(),
        }
    }

    /// Unix seconds for the picker, or None for an impossible date (Feb 30)
    /// or a local time skipped by a daylight-saving change.
    pub fn picked_unix(&self) -> Option<i64> {
        use chrono::TimeZone;
        let [y, mo, d, h, mi, s] = self.parts;
        let date =
            chrono::NaiveDate::from_ymd_opt(y, u32::try_from(mo).ok()?, u32::try_from(d).ok()?)?;
        let naive = date.and_hms_opt(
            u32::try_from(h).ok()?,
            u32::try_from(mi).ok()?,
            u32::try_from(s).ok()?,
        )?;
        if self.utc {
            Some(chrono::Utc.from_utc_datetime(&naive).timestamp())
        } else {
            chrono::Local
                .from_local_datetime(&naive)
                .earliest()
                .map(|t| t.timestamp())
        }
    }
}

/// Parses a Unix time (seconds, or milliseconds when 13 or more digits).
pub fn parse_unix(text: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let t = text.trim();
    let n: i64 = t.parse().ok()?;
    let digits = t.trim_start_matches('-').len();
    if digits >= 13 {
        chrono::DateTime::from_timestamp_millis(n)
    } else {
        chrono::DateTime::from_timestamp(n, 0)
    }
}

#[derive(Clone, Debug)]
pub struct HashDialog {
    pub algo: HashAlgo,
    pub from_files: bool,
    pub input: String,
    pub output: String,
    pub each_line: bool,
}

impl HashDialog {
    pub fn new(algo: HashAlgo, from_files: bool) -> Self {
        Self {
            algo,
            from_files,
            input: String::new(),
            output: String::new(),
            each_line: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CharRangeDialog {
    pub range: u8,
    pub from: String,
    pub to: String,
    pub backward: bool,
    pub wrap: bool,
    pub status: String,
}

impl Default for CharRangeDialog {
    fn default() -> Self {
        Self {
            range: 0,
            from: "0".into(),
            to: "255".into(),
            backward: false,
            wrap: true,
            status: String::new(),
        }
    }
}

fn history_combo(
    ui: &mut Ui,
    id: &str,
    value: &mut String,
    history: &[String],
    width: f32,
    focus: bool,
) -> egui::Response {
    let mut resp = None;
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(width - 24.0)
                .id_salt(id),
        );
        if focus {
            r.request_focus();
        }
        egui::ComboBox::from_id_salt(format!("{id}_hist"))
            .selected_text("")
            .width(16.0)
            .show_ui(ui, |ui| {
                for h in history {
                    if ui.selectable_label(false, h.as_str()).clicked() {
                        *value = h.clone();
                    }
                }
            });
        resp = Some(r);
    });
    resp.unwrap()
}

impl App {
    #[allow(clippy::too_many_lines)]
    /// The Find/Replace/Find in Files/Mark dialog: a floating window, or a
    /// panel docked at the bottom. Called before the editors are laid out
    /// (`docked_pass`, so a docked panel takes its space) and after them.
    #[allow(clippy::too_many_lines)]
    pub fn find_dialog(&mut self, root: &mut Ui, docked_pass: bool) {
        if !self.find.open || self.settings.find_docked != docked_pass {
            return;
        }
        let ctx = root.ctx().clone();
        let mut open = true;
        let title = match self.find.tab {
            FindTab::Find => "Find",
            FindTab::Replace => "Replace",
            FindTab::FindInFiles => "Find in Files",
            FindTab::Mark => "Mark",
        };
        let mut actions: Vec<&'static str> = Vec::new();
        if docked_pass {
            egui::Panel::bottom("find_dock").show(root, |ui| {
                ui.add_space(4.0);
                self.find_body(ui, &mut actions);
                ui.add_space(2.0);
            });
        } else {
            // See-through while the pointer is elsewhere (searching from the
            // keyboard), so matches behind the window stay visible.
            let hovered = self
                .find
                .window_rect
                .zip(ctx.pointer_hover_pos())
                .is_some_and(|(r, p)| r.contains(p));
            let t = ctx.animate_bool_with_time(egui::Id::unique("find_fade"), hovered, 0.15);
            let opacity = egui::lerp(FIND_FADED_OPACITY..=1.0, t);
            let mut frame = egui::Frame::window(&ctx.global_style());
            frame.fill = frame.fill.gamma_multiply(opacity);
            frame.stroke.color = frame.stroke.color.gamma_multiply(opacity);
            frame.shadow.color = frame.shadow.color.gamma_multiply(opacity * opacity);
            let shown = egui::Window::new(title)
                .id(egui::Id::unique("find_dialog"))
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .frame(frame)
                .default_pos(ctx.content_rect().right_top() + egui::vec2(-560.0, 80.0))
                .show(&ctx, |ui| {
                    ui.multiply_opacity(opacity);
                    self.find_body(ui, &mut actions);
                });
            self.find.window_rect = shown.map(|r| r.response.rect);
        }
        if !open {
            self.find.open = false;
        }
        for a in actions {
            match a {
                "next" => {
                    self.push_history();
                    let what = self.find.what.clone();
                    let opts = self.settings.search_opts.clone();
                    let back = self.find.backward;
                    self.find_in_active(&what, back, &opts, true);
                }
                "count" => self.count_matches(),
                "find_all_cur" => self.find_all_docs(false),
                "find_all_docs" => self.find_all_docs(true),
                "replace" => self.replace_one(),
                "replace_all" => self.replace_all(false),
                "replace_all_docs" => self.replace_all(true),
                "fif" => self.start_find_in_files(false),
                "rif" => {
                    self.start_find_in_files(true);
                }
                "fif_stop" => {
                    if let Some(f) = &self.fif {
                        f.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                "mark" => self.mark_all(),
                "clear_marks" => self.clear_marks(),
                "copy_marked" => self.run(Command::CopyStyledText(0)),
                "close" => {
                    self.find.open = false;
                    self.request_focus_editor = true;
                }
                "dock" => self.settings.find_docked = !self.settings.find_docked,
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn find_body(&mut self, ui: &mut Ui, actions: &mut Vec<&'static str>) {
        ui.horizontal(|ui| {
            for (t, label) in [
                (FindTab::Find, "Find"),
                (FindTab::Replace, "Replace"),
                (FindTab::FindInFiles, "Find in Files"),
                (FindTab::Mark, "Mark"),
            ] {
                if ui.selectable_label(self.find.tab == t, label).clicked() {
                    self.find.tab = t;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (label, tip) = if self.settings.find_docked {
                    ("Float", "Show as a floating window")
                } else {
                    ("Dock", "Dock at the bottom of the window")
                };
                if ui.small_button(label).on_hover_text(tip).clicked() {
                    actions.push("dock");
                }
            });
        });
        ui.separator();
        let focus = std::mem::take(&mut self.find.focus_what);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                egui::Grid::new("find_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Find what:");
                        let hist = self.settings.find_history.clone();
                        let r = history_combo(
                            ui,
                            "find_what",
                            &mut self.find.what,
                            &hist,
                            300.0,
                            focus,
                        );
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            actions.push(match self.find.tab {
                                FindTab::FindInFiles => "fif",
                                FindTab::Mark => "mark",
                                _ => "next",
                            });
                        }
                        ui.end_row();
                        if matches!(self.find.tab, FindTab::Replace | FindTab::FindInFiles)
                        {
                            ui.label("Replace with:");
                            let hist = self.settings.replace_history.clone();
                            history_combo(
                                ui,
                                "replace_with",
                                &mut self.find.with,
                                &hist,
                                300.0,
                                false,
                            );
                            ui.end_row();
                        }
                        if self.find.tab == FindTab::FindInFiles {
                            ui.label("Filters:");
                            let hist = self.settings.fif_filters.clone();
                            history_combo(
                                ui,
                                "fif_filters",
                                &mut self.find.fif_filters,
                                &hist,
                                300.0,
                                false,
                            );
                            ui.end_row();
                            ui.label("Directory:");
                            ui.horizontal(|ui| {
                                let hist = self.settings.fif_dirs.clone();
                                history_combo(
                                    ui,
                                    "fif_dir",
                                    &mut self.find.fif_dir,
                                    &hist,
                                    270.0,
                                    false,
                                );
                                if ui.button("...").clicked()
                                    && let Some(d) = rfd::FileDialog::new().pick_folder()
                                {
                                    self.find.fif_dir = d.display().to_string();
                                }
                            });
                            ui.end_row();
                        }
                    });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        if matches!(self.find.tab, FindTab::Find | FindTab::Replace) {
                            ui.checkbox(&mut self.find.backward, "Search backward");
                        }
                        ui.checkbox(
                            &mut self.settings.search_opts.whole_word,
                            "Whole word only",
                        );
                        ui.checkbox(
                            &mut self.settings.search_opts.match_case,
                            "Match case",
                        );
                        if self.find.tab != FindTab::FindInFiles {
                            ui.checkbox(&mut self.settings.search_opts.wrap, "Wrap around");
                            ui.checkbox(&mut self.find.in_selection, "In selection");
                        }
                        if self.find.tab == FindTab::Mark {
                            ui.checkbox(&mut self.find.mark_bookmark, "Bookmark line");
                            ui.checkbox(&mut self.find.mark_purge, "Purge for each search");
                        }
                        if self.find.tab == FindTab::FindInFiles {
                            ui.checkbox(
                                &mut self.find.fif_follow_current,
                                "Follow current doc.",
                            );
                            ui.checkbox(&mut self.find.fif_recursive, "In all sub-folders");
                            ui.checkbox(&mut self.find.fif_hidden, "In hidden folders");
                        }
                    });
                    ui.add_space(12.0);
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Search mode").weak());
                            let m = &mut self.settings.search_opts.mode;
                            ui.radio_value(m, SearchMode::Normal, "Normal");
                            ui.radio_value(
                                m,
                                SearchMode::Extended,
                                "Extended (\\n \\t \\x..)",
                            )
                            .on_hover_text("\\n, \\r, \\t, \\0, \\xNN (hex), \\uNNNN (Unicode), \\dNNN (decimal), \\oNNN (octal), \\bNNNNNNNN (binary)");
                            ui.radio_value(m, SearchMode::Regex, "Regular expression");
                            // Only applies to regular expressions.
                            if *m == SearchMode::Regex {
                                ui.indent("dot_nl", |ui| {
                                    ui.checkbox(
                                        &mut self.settings.search_opts.dot_matches_newline,
                                        ". matches newline",
                                    );
                                });
                            }
                        });
                    });
                });
            });
            ui.add_space(8.0);
            ui.vertical(|ui| {
                let bw = 180.0;
                let mut btn = |ui: &mut Ui, label: &str, act: &'static str| {
                    if ui.add_sized([bw, 22.0], egui::Button::new(label)).clicked() {
                        actions.push(act);
                    }
                };
                match self.find.tab {
                    FindTab::Find => {
                        btn(ui, "Find Next", "next");
                        btn(ui, "Count", "count");
                        btn(ui, "Find All in This Tab", "find_all_cur");
                        btn(ui, "Find All in Open Tabs", "find_all_docs");
                    }
                    FindTab::Replace => {
                        btn(ui, "Find Next", "next");
                        btn(ui, "Replace", "replace");
                        btn(ui, "Replace All", "replace_all");
                        btn(ui, "Replace All in Open Tabs", "replace_all_docs");
                    }
                    FindTab::FindInFiles => {
                        if self.fif.is_some() {
                            btn(ui, "Stop", "fif_stop");
                        } else {
                            btn(ui, "Find All", "fif");
                            btn(ui, "Replace in Files", "rif");
                        }
                    }
                    FindTab::Mark => {
                        btn(ui, "Mark All", "mark");
                        btn(ui, "Clear All Marks", "clear_marks");
                        btn(ui, "Copy Marked Text", "copy_marked");
                    }
                }
                btn(ui, "Close", "close");
            });
        });
        // Status bar: what the last search found.
        ui.separator();
        ui.horizontal(|ui| {
            ui.set_min_height(16.0);
            if let Some(f) = &self.fif {
                ui.label(RichText::new(format!("Searching... {} files", f.progress)).small());
            } else if let Some((msg, kind)) = &self.find.status {
                let c = match kind {
                    StatusKind::Info => Color32::from_rgb(0x30, 0x60, 0xD0),
                    StatusKind::NotFound => Color32::from_rgb(0xD0, 0x20, 0x20),
                    StatusKind::Wrapped => Color32::from_rgb(0x20, 0x90, 0x20),
                };
                ui.add(egui::Label::new(RichText::new(msg).color(c).small()).truncate());
            }
        });
    }

    #[allow(clippy::too_many_lines)]
    pub fn dialogs_ui(&mut self, ctx: &Context, out: &mut Vec<Command>) {
        // Go To.
        if let Some((by_line, mut value)) = self.dialogs.goto.clone() {
            let mut open = true;
            let mut go = false;
            let mut by_line = by_line;
            let (cur_line, max_line, cur_pos, max_pos) = self
                .active_doc()
                .map(|d| {
                    let v = self.views.get(&(self.focused, d.id));
                    let h = v.map_or(0, |v| v.primary().head);
                    (d.char_to_line(h) + 1, d.line_count(), h, d.len_chars())
                })
                .unwrap_or((1, 1, 0, 0));
            egui::Window::new("Go to Line")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut by_line, true, "Line");
                        ui.radio_value(&mut by_line, false, "Character offset");
                    });
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut value)
                                .desired_width(120.0)
                                .hint_text(if by_line { "Line number" } else { "Offset" }),
                        );
                        r.request_focus();
                        value.retain(|c| c.is_ascii_digit());
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            go = true;
                        }
                        let (cur, max) = if by_line {
                            (cur_line, max_line)
                        } else {
                            (cur_pos, max_pos)
                        };
                        ui.label(RichText::new(format!("now {cur}, last {max}")).weak());
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Go").clicked() {
                            go = true;
                        }
                        if ui.button("Cancel").clicked() {
                            self.dialogs.goto = None;
                        }
                    });
                });
            if go {
                if let Ok(n) = value.trim().parse::<usize>() {
                    self.with_active(|d, v, _| {
                        if by_line {
                            let l = n.saturating_sub(1).min(d.line_count() - 1);
                            d.ensure_line_visible(l);
                            v.set_single(d.line_start(l));
                        } else {
                            let p = n.min(d.len_chars());
                            d.ensure_line_visible(d.char_to_line(p));
                            v.set_single(p);
                        }
                        v.center_caret = true;
                    });
                }
                self.dialogs.goto = None;
                self.request_focus_editor = true;
            } else if !open {
                self.dialogs.goto = None;
            } else if self.dialogs.goto.is_some() {
                self.dialogs.goto = Some((by_line, value));
            }
        }

        if self.dialogs.preferences {
            let mut open = true;
            egui::Window::new("Settings")
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| self.settings_ui(ui));
            if !open {
                self.dialogs.preferences = false;
                self.dialogs.shortcut_rows = None;
                crate::config::save_settings(&self.settings);
            }
        }

        if let Some(mut st) = self.dialogs.column_editor.clone() {
            let mut open = true;
            let mut ok = false;
            let mut cancel = false;
            egui::Window::new("Column Editor")
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new("Inserts on every line of the column selection or carets.")
                            .weak(),
                    );
                    ui.add_space(4.0);
                    ui.radio_value(&mut st.text_mode, true, "Text");
                    ui.indent("col_text", |ui| {
                        ui.add_enabled(
                            st.text_mode,
                            egui::TextEdit::singleline(&mut st.text).desired_width(240.0),
                        );
                    });
                    ui.radio_value(&mut st.text_mode, false, "Numbers");
                    ui.add_enabled_ui(!st.text_mode, |ui| {
                        egui::Grid::new("col_grid").num_columns(2).show(ui, |ui| {
                            let field = |ui: &mut Ui, v: &mut String| {
                                ui.add(egui::TextEdit::singleline(v).desired_width(80.0));
                            };
                            ui.label("Start at:");
                            field(ui, &mut st.initial);
                            ui.end_row();
                            ui.label("Step:");
                            field(ui, &mut st.increase);
                            ui.end_row();
                            ui.label("Repeat each:");
                            field(ui, &mut st.repeat);
                            ui.end_row();
                        });
                        ui.horizontal(|ui| {
                            ui.label("Pad with:");
                            ui.radio_value(&mut st.leading, LeadingMode::None, "None");
                            ui.radio_value(&mut st.leading, LeadingMode::Zeros, "Zeros");
                            ui.radio_value(&mut st.leading, LeadingMode::Spaces, "Spaces");
                        });
                        ui.horizontal(|ui| {
                            ui.label("Format:");
                            ui.radio_value(&mut st.format, NumFormat::Dec, "Dec");
                            ui.radio_value(&mut st.format, NumFormat::Hex, "Hex");
                            ui.radio_value(&mut st.format, NumFormat::Oct, "Oct");
                            ui.radio_value(&mut st.format, NumFormat::Bin, "Bin");
                        });
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Insert").clicked() {
                            ok = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if cancel {
                open = false;
            }
            if ok {
                let st2 = st.clone();
                self.with_active(|d, v, _| {
                    // Lines: the column selection, the multi-carets, or caret line to EOF.
                    let (first, last, col) = if let Some((a, h)) = v.rect {
                        (a.0.min(h.0), a.0.max(h.0), a.1.min(h.1))
                    } else if v.sels.len() > 1 {
                        let lines: Vec<usize> =
                            v.sels.iter().map(|s| d.char_to_line(s.start())).collect();
                        (
                            *lines.iter().min().unwrap(),
                            *lines.iter().max().unwrap(),
                            v.display_col(d, v.sels[0].start()),
                        )
                    } else {
                        let p = v.primary().start();
                        (d.char_to_line(p), d.line_count() - 1, v.display_col(d, p))
                    };
                    let n = last - first + 1;
                    let texts: Vec<String> = if st2.text_mode {
                        vec![st2.text.clone(); n]
                    } else {
                        textops::number_sequence(
                            n,
                            st2.initial.trim().parse().unwrap_or(0),
                            st2.increase.trim().parse().unwrap_or(1),
                            st2.repeat.trim().parse().unwrap_or(1),
                            st2.format,
                            st2.leading,
                            true,
                        )
                    };
                    crate::ops::column_insert(d, v, first, col, &texts);
                });
                self.dialogs.column_editor = None;
            } else if open {
                self.dialogs.column_editor = Some(st);
            } else {
                self.dialogs.column_editor = None;
            }
        }

        if self.dialogs.summary {
            let mut open = true;
            let info = self.active_id().and_then(|id| {
                let d = self.docs.get(&id)?;
                let v = self.views.get(&(self.focused, id));
                let text = d.text();
                let words = text
                    .split(|c: char| !crate::ops::is_word_char(c))
                    .filter(|w| !w.is_empty())
                    .count();
                let eol_chars: usize = text.chars().filter(|c| *c == '\n' || *c == '\r').count();
                let (sel_chars, sel_bytes, sel_lines) = v.map_or((0, 0, 0), |v| {
                    let t = v.selected_texts(d);
                    (
                        t.iter().map(|x| x.chars().count()).sum::<usize>(),
                        t.iter().map(String::len).sum::<usize>(),
                        t.iter().map(|x| x.lines().count()).sum::<usize>(),
                    )
                });
                let meta = d.path.as_ref().and_then(|p| std::fs::metadata(p).ok());
                let fmt_time = |t: Option<std::time::SystemTime>| {
                    t.map(|t| {
                        chrono::DateTime::<chrono::Local>::from(t)
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string()
                    })
                    .unwrap_or_default()
                };
                let mut rows: Vec<(&str, String)> = vec![("Path", d.full_path_or_name())];
                if let Some(m) = &meta {
                    rows.push(("Created", fmt_time(m.created().ok())));
                    rows.push(("Modified", fmt_time(m.modified().ok())));
                }
                rows.extend([
                    ("Lines", d.line_count().to_string()),
                    ("Words", words.to_string()),
                    (
                        "Characters",
                        format!("{} (without line endings)", d.len_chars() - eol_chars),
                    ),
                    (
                        "Size",
                        format!(
                            "{} bytes in memory, {} bytes as saved",
                            text.len(),
                            d.encoding.encode(&text).len()
                        ),
                    ),
                    (
                        "Selection",
                        format!("{sel_chars} characters ({sel_bytes} bytes) on {sel_lines} lines"),
                    ),
                ]);
                Some(rows)
            });
            egui::Window::new("Document Summary")
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    egui::Grid::new("summary_grid")
                        .num_columns(2)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            for (k, v) in info.unwrap_or_default() {
                                ui.label(RichText::new(k).weak());
                                ui.label(v);
                                ui.end_row();
                            }
                        });
                });
            if !open {
                self.dialogs.summary = false;
            }
        }

        if let Some(mut h) = self.dialogs.hash.clone() {
            let mut open = true;
            let mut copy = None;
            egui::Window::new(format!("{} Hash", h.algo.name()))
                .open(&mut open)
                .default_width(520.0)
                .show(ctx, |ui| {
                    if !h.from_files {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Text").strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.checkbox(&mut h.each_line, "Hash each line separately");
                                },
                            );
                        });
                        ui.add(
                            egui::TextEdit::multiline(&mut h.input)
                                .desired_rows(6)
                                .desired_width(f32::INFINITY),
                        );
                        h.output = if h.each_line {
                            h.input
                                .lines()
                                .map(|l| textops::hex_digest(h.algo, l.as_bytes()))
                                .collect::<Vec<_>>()
                                .join("\n")
                        } else if h.input.is_empty() {
                            String::new()
                        } else {
                            textops::hex_digest(h.algo, h.input.as_bytes())
                        };
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new(h.algo.name()).strong());
                    let mut o = h.output.clone();
                    ui.add(
                        egui::TextEdit::multiline(&mut o)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace),
                    );
                    if ui.button("Copy").clicked() {
                        copy = Some(h.output.clone());
                    }
                });
            if let Some(t) = copy {
                self.clip_set(&t);
            }
            self.dialogs.hash = open.then_some(h);
        }

        if self.dialogs.windows {
            let mut open = true;
            let mut action: Option<(&str, u64)> = None;
            egui::Window::new("All Open Documents")
                .open(&mut open)
                .default_size([640.0, 360.0])
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            egui::Grid::new("win_grid")
                                .striped(true)
                                .num_columns(4)
                                .show(ui, |ui| {
                                    ui.strong("Name");
                                    ui.strong("Folder");
                                    ui.strong("Language");
                                    ui.strong("Characters");
                                    ui.end_row();
                                    for (pi, p) in self.panes.iter().enumerate() {
                                        for id in &p.tabs {
                                            let Some(d) = self.docs.get(id) else { continue };
                                            let sel = self.dialogs.windows_selected == Some(*id);
                                            let r = ui.selectable_label(sel, App::doc_label(d));
                                            if r.clicked() || r.secondary_clicked() {
                                                action = Some(("select", *id));
                                            }
                                            if r.double_clicked() {
                                                action = Some(("activate", *id));
                                            }
                                            r.context_menu(|ui| {
                                                if ui.button("Go to Tab").clicked() {
                                                    action = Some(("activate", *id));
                                                }
                                                ui.separator();
                                                if let Some(op) = crate::panels::file_ops_menu(
                                                    ui,
                                                    d.path.as_deref(),
                                                ) {
                                                    action = Some((op.key(), *id));
                                                }
                                            });
                                            ui.label(
                                                d.path
                                                    .as_ref()
                                                    .and_then(|x| x.parent())
                                                    .map(|x| x.display().to_string())
                                                    .unwrap_or_default(),
                                            );
                                            ui.label(self.langs.langs[d.lang].display.clone());
                                            ui.label(format!(
                                                "{}{}",
                                                d.len_chars(),
                                                if pi == 1 { " (view 2)" } else { "" }
                                            ));
                                            ui.end_row();
                                        }
                                    }
                                });
                        });
                    ui.horizontal(|ui| {
                        if let Some(id) = self.dialogs.windows_selected {
                            if ui.button("Go to Tab").clicked() {
                                action = Some(("activate", id));
                            }
                            if ui.button("Save").clicked() {
                                action = Some(("save", id));
                            }
                            if ui.button("Close Tab").clicked() {
                                action = Some(("close", id));
                            }
                            ui.separator();
                            let saved = self
                                .docs
                                .get(&id)
                                .and_then(|d| d.path.as_ref())
                                .is_some_and(|p| p.exists());
                            if ui
                                .add_enabled(saved, egui::Button::new("Duplicate"))
                                .clicked()
                            {
                                action = Some(("duplicate", id));
                            }
                            if ui.button("Rename...").clicked() {
                                action = Some(("rename", id));
                            }
                            if ui
                                .add_enabled(saved, egui::Button::new("Delete..."))
                                .on_hover_text(crate::app::TRASH_LABEL)
                                .clicked()
                            {
                                action = Some(("delete", id));
                            }
                            ui.separator();
                        }
                        if ui.button("Sort Tabs by Name").clicked() {
                            out.push(Command::SortTabs(crate::commands::TabSort::NameAsc));
                        }
                    });
                });
            if let Some((a, id)) = action {
                let pane = self
                    .panes
                    .iter()
                    .position(|p| p.tabs.contains(&id))
                    .unwrap_or(0);
                match a {
                    "select" => self.dialogs.windows_selected = Some(id),
                    "activate" => self.activate(pane, id),
                    "save" => {
                        self.save_doc(id, false);
                    }
                    "close" => self.close_tab(pane, id, false),
                    other => self.doc_file_op(id, other),
                }
            }
            if !open {
                self.dialogs.windows = false;
            }
        }

        if let Some(mut cr) = self.dialogs.char_range.clone() {
            let mut open = true;
            let mut find = false;
            egui::Window::new("Find Characters in Range")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.radio_value(&mut cr.range, 0, "Non-ASCII (128-255)");
                    ui.radio_value(&mut cr.range, 1, "ASCII (0-127)");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut cr.range, 2, "Custom:");
                        ui.add(egui::TextEdit::singleline(&mut cr.from).desired_width(60.0));
                        ui.label("-");
                        ui.add(egui::TextEdit::singleline(&mut cr.to).desired_width(60.0));
                    });
                    ui.add_space(4.0);
                    ui.checkbox(&mut cr.backward, "Search backward");
                    ui.checkbox(&mut cr.wrap, "Wrap around");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Find Next").clicked() {
                            find = true;
                        }
                        ui.label(RichText::new(cr.status.clone()).weak());
                    });
                });
            if find {
                let range = match cr.range {
                    0 => CharRange::NonAscii,
                    1 => CharRange::Ascii,
                    _ => CharRange::Custom(
                        cr.from.trim().parse().unwrap_or(0),
                        cr.to.trim().parse().unwrap_or(255),
                    ),
                };
                let (back, wrap) = (cr.backward, cr.wrap);
                let found = self.with_active(|d, v, _| {
                    let len = d.len_chars();
                    let start = if back {
                        v.primary().start()
                    } else {
                        v.primary().end()
                    };
                    let check = |p: usize| {
                        d.char_at(p)
                            .is_some_and(|c| textops::char_in_range(c, range))
                    };
                    let hit = if back {
                        (0..start).rev().find(|&p| check(p)).or_else(|| {
                            if wrap {
                                (start..len).rev().find(|&p| check(p))
                            } else {
                                None
                            }
                        })
                    } else {
                        (start..len).find(|&p| check(p)).or_else(|| {
                            if wrap {
                                (0..start).find(|&p| check(p))
                            } else {
                                None
                            }
                        })
                    };
                    if let Some(p) = hit {
                        d.ensure_line_visible(d.char_to_line(p));
                        v.sels = vec![Selection::new(p, p + 1)];
                        v.scroll_to_caret = true;
                    }
                    hit.is_some()
                });
                cr.status = if found == Some(true) {
                    String::new()
                } else {
                    "Can't find any character in the range.".into()
                };
            }
            self.dialogs.char_range = open.then_some(cr);
        }

        if let Some((target, mut name)) = self.dialogs.rename.clone() {
            let mut open = true;
            let mut ok = false;
            let mut cancel = false;
            egui::Window::new("Rename")
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut name).desired_width(280.0))
                        .request_focus();
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Rename").clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            ok = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if cancel {
                open = false;
            }
            if ok {
                // A saved document renames its file; an unsaved one just
                // renames the tab.
                let file = match &target {
                    RenameTarget::File(p) => Some(p.clone()),
                    RenameTarget::Doc(id) => self
                        .docs
                        .get(id)
                        .and_then(|d| d.path.clone())
                        .filter(|p| p.exists()),
                };
                let mut err = None;
                if let Some(p) = file {
                    if let Err(e) = self.rename_file(&p, &name) {
                        err = Some(e);
                    }
                } else if let RenameTarget::Doc(id) = &target
                    && let Some(d) = self.docs.get_mut(id)
                {
                    d.untitled_name = name.clone();
                }
                match err {
                    // Keep the window open so the name can be fixed.
                    Some(e) => {
                        self.message("Rename", &e);
                        self.dialogs.rename = Some((target, name));
                    }
                    None => self.dialogs.rename = None,
                }
            } else {
                self.dialogs.rename = open.then_some((target, name));
            }
        }

        if let Some(mut st) = self.dialogs.datetime.take() {
            let mut open = true;
            let mut insert: Option<String> = None;
            let mut copy: Option<String> = None;
            egui::Window::new("Date and Unix Time")
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    let row = |ui: &mut Ui,
                               label: &str,
                               value: String,
                               insert: &mut Option<String>,
                               copy: &mut Option<String>| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(label).weak());
                            ui.monospace(&value);
                            if ui.small_button("Insert").clicked() {
                                *insert = Some(value.clone());
                            }
                            if ui.small_button("Copy").clicked() {
                                *copy = Some(value);
                            }
                        });
                    };
                    // Live clock.
                    ui.strong("Now");
                    let now = chrono::Local::now();
                    row(
                        ui,
                        "Local:",
                        now.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
                        &mut insert,
                        &mut copy,
                    );
                    row(
                        ui,
                        "UTC:  ",
                        now.with_timezone(&chrono::Utc)
                            .format("%Y-%m-%dT%H:%M:%SZ")
                            .to_string(),
                        &mut insert,
                        &mut copy,
                    );
                    row(
                        ui,
                        "Unix: ",
                        now.timestamp().to_string(),
                        &mut insert,
                        &mut copy,
                    );
                    ctx.request_repaint_after(std::time::Duration::from_millis(
                        1000 - u64::from(now.timestamp_subsec_millis()),
                    ));
                    ui.separator();

                    // Date picker -> Unix time.
                    ui.strong("Date to Unix time");
                    ui.horizontal(|ui| {
                        let [y, mo, d, h, mi, s] = &mut st.parts;
                        ui.add(egui::DragValue::new(y).range(1..=9999).prefix("Y "));
                        ui.add(egui::DragValue::new(mo).range(1..=12).prefix("M "));
                        ui.add(egui::DragValue::new(d).range(1..=31).prefix("D "));
                        ui.label(" ");
                        ui.add(egui::DragValue::new(h).range(0..=23).suffix(" h"));
                        ui.add(egui::DragValue::new(mi).range(0..=59).suffix(" m"));
                        ui.add(egui::DragValue::new(s).range(0..=59).suffix(" s"));
                    });
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut st.utc, false, "Local time");
                        ui.radio_value(&mut st.utc, true, "UTC");
                        if ui.small_button("Now").clicked() {
                            let utc = st.utc;
                            st.parts = DateTimeDialog::now().parts;
                            if utc {
                                use chrono::{Datelike, Timelike};
                                let n = chrono::Utc::now();
                                #[allow(clippy::cast_possible_wrap)]
                                {
                                    st.parts = [
                                        n.year(),
                                        n.month() as i32,
                                        n.day() as i32,
                                        n.hour() as i32,
                                        n.minute() as i32,
                                        n.second() as i32,
                                    ];
                                }
                            }
                        }
                    });
                    match st.picked_unix() {
                        Some(t) => {
                            row(ui, "Unix: ", t.to_string(), &mut insert, &mut copy);
                            row(ui, "ms:   ", (t * 1000).to_string(), &mut insert, &mut copy);
                        }
                        None => {
                            ui.label(RichText::new("Not a valid date or time").color(Color32::RED));
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Or type a date:").weak());
                        ui.add(
                            egui::TextEdit::singleline(&mut st.date_input)
                                .desired_width(200.0)
                                .hint_text("2026-10-30T14:05:00Z"),
                        );
                    });
                    if !st.date_input.trim().is_empty() {
                        match crate::datatools::date_to_timestamp(&st.date_input) {
                            Ok(t) => row(ui, "Unix: ", t, &mut insert, &mut copy),
                            Err(e) => {
                                ui.label(RichText::new(e).color(Color32::RED));
                            }
                        }
                    }
                    ui.separator();

                    // Unix time -> date.
                    ui.strong("Unix time to date");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut st.unix_input)
                                .desired_width(160.0)
                                .hint_text("seconds or ms"),
                        );
                        st.unix_input.retain(|c| c.is_ascii_digit() || c == '-');
                    });
                    match parse_unix(&st.unix_input) {
                        Some(t) => {
                            let local = t.with_timezone(&chrono::Local);
                            row(
                                ui,
                                "Local:",
                                local.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
                                &mut insert,
                                &mut copy,
                            );
                            if let Ok(utc) = crate::datatools::timestamp_to_date(&st.unix_input) {
                                row(ui, "UTC:  ", utc, &mut insert, &mut copy);
                            }
                        }
                        None if st.unix_input.trim().is_empty() => {}
                        None => {
                            ui.label(RichText::new("Not a valid Unix time").color(Color32::RED));
                        }
                    }
                });
            if let Some(t) = copy {
                ctx.copy_text(t);
            }
            if let Some(t) = insert {
                self.with_active(|d, v, _| {
                    crate::ops::insert_text(d, v, &t, crate::document::EditKind::Other)
                });
            }
            if open {
                self.dialogs.datetime = Some(st);
            }
        }

        if let Some(mut generator) = self.dialogs.password.take() {
            let mut open = true;
            let mut action = crate::password::PasswordAction::None;
            egui::Window::new("Password Generator")
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| action = generator.ui(ui));
            match action {
                crate::password::PasswordAction::Insert(p) => {
                    self.with_active(|d, v, _| {
                        crate::ops::insert_text(d, v, &p, crate::document::EditKind::Other)
                    });
                    self.request_focus_editor = true;
                }
                crate::password::PasswordAction::Close => {}
                crate::password::PasswordAction::None if open => {
                    self.dialogs.password = Some(generator);
                }
                crate::password::PasswordAction::None => {}
            }
        }

        if let Some(mut picker) = self.dialogs.color_picker.take() {
            let mut open = true;
            let mut action = crate::colorpick::PickerAction::None;
            picker.poll_sampler(ctx);
            egui::Window::new("Color Picker")
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| action = picker.ui(ui));
            match action {
                crate::colorpick::PickerAction::Insert(hex) => {
                    self.with_active(|d, v, _| {
                        crate::ops::insert_text(d, v, &hex, crate::document::EditKind::Other)
                    });
                    self.request_focus_editor = true;
                }
                crate::colorpick::PickerAction::Cancel => {}
                crate::colorpick::PickerAction::None if open => {
                    self.dialogs.color_picker = Some(picker);
                }
                crate::colorpick::PickerAction::None => {}
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    /// The Settings window: sections on the left, the chosen one on the right.
    fn settings_ui(&mut self, ui: &mut Ui) {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(170.0);
                for (i, t) in SETTINGS_SECTIONS.iter().enumerate() {
                    if ui
                        .selectable_label(self.dialogs.pref_tab == i, *t)
                        .clicked()
                    {
                        self.dialogs.pref_tab = i;
                    }
                }
                ui.add_space(20.0);
                ui.label(
                    RichText::new(format!(
                        "{APP_NAME} {}\nMIT License",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .weak()
                    .small(),
                );
            });
            ui.separator();
            ui.vertical(|ui| {
                // A fixed size, so the window doesn't jump between sections.
                ui.set_width(520.0);
                ui.set_min_height(380.0);
                ui.heading(
                    SETTINGS_SECTIONS[self.dialogs.pref_tab.min(SETTINGS_SECTIONS.len() - 1)],
                );
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| self.settings_section(ui));
            });
        });
    }

    #[allow(clippy::too_many_lines)]
    fn settings_section(&mut self, ui: &mut Ui) {
        let group = |ui: &mut Ui, title: &str| {
            ui.add_space(6.0);
            ui.label(RichText::new(title).strong());
        };
        match self.dialogs.pref_tab {
            settings_section::GENERAL => {
                let s = &mut self.settings;
                group(ui, "Window");
                ui.checkbox(&mut s.show_toolbar, "Show toolbar");
                ui.checkbox(&mut s.show_statusbar, "Show status bar");
                ui.checkbox(
                    &mut s.single_instance,
                    "Open files in the running window (single instance)",
                );
                group(ui, "Tabs");
                ui.checkbox(&mut s.show_tabbar, "Show tab bar");
                ui.checkbox(&mut s.tab_close_button, "Show a close button on each tab");
                ui.checkbox(
                    &mut s.double_click_close_tab,
                    "Double-click a tab to close it",
                );
                group(ui, "Updates");
                ui.checkbox(
                    &mut s.check_for_updates,
                    "Check for a new version when the app starts",
                );
                let checking = self.update.job.is_some();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!checking, egui::Button::new("Check Now"))
                        .clicked()
                    {
                        self.run(Command::CheckForUpdates);
                    }
                    if let Some(status) = &self.update.status {
                        ui.label(RichText::new(status).weak());
                    }
                });
                if let Some(u) = self.update.available.clone()
                    && ui.link(format!("Download version {}", u.version)).clicked()
                {
                    self.run(Command::OpenUpdatePage);
                }
                ui.label(
                    RichText::new(format!(
                        "This is version {}. A check asks GitHub which release is the latest, sending only the app's name and version.",
                        crate::update::current_version()
                    ))
                    .weak()
                    .small(),
                );
                let s = &mut self.settings;
                group(ui, "Other");
                ui.horizontal(|ui| {
                    ui.label("Recent files to remember:");
                    ui.add(egui::DragValue::new(&mut s.recent_max).range(0..=30));
                });
                ui.horizontal(|ui| {
                    ui.label("Web search URL:");
                    ui.add(egui::TextEdit::singleline(&mut s.search_engine).desired_width(320.0));
                });
                ui.label(RichText::new("$(CURRENT_WORD) is replaced by the selected text.").weak());
            }
            settings_section::APPEARANCE => self.appearance_settings(ui),
            settings_section::EDITING => {
                let s = &mut self.settings;
                group(ui, "Indentation");
                ui.horizontal(|ui| {
                    ui.label("Tab size:");
                    ui.add(egui::DragValue::new(&mut s.tab_size).range(1..=16));
                    ui.checkbox(&mut s.use_spaces, "Insert spaces instead of tabs");
                });
                ui.checkbox(&mut s.auto_indent, "Auto-indent new lines");
                group(ui, "Margins");
                ui.checkbox(&mut s.show_line_numbers, "Line numbers");
                ui.checkbox(&mut s.show_bookmark_margin, "Bookmarks");
                ui.checkbox(&mut s.show_change_history, "Change history");
                ui.checkbox(&mut s.show_fold_margin, "Folding");
                ui.horizontal(|ui| {
                    ui.label("Vertical edge at column (0 = none):");
                    ui.add(egui::DragValue::new(&mut s.edge_column).range(0..=500));
                });
                group(ui, "Highlighting");
                ui.checkbox(
                    &mut s.highlight_on_start,
                    "Turn on syntax highlighting when the app starts",
                );
                ui.checkbox(&mut s.highlight_current_line, "Current line");
                ui.checkbox(&mut s.brace_highlight, "Matching braces");
                ui.checkbox(
                    &mut s.smart_highlight,
                    "Other occurrences of the selected word",
                );
                ui.indent("smart", |ui| {
                    ui.add_enabled_ui(s.smart_highlight, |ui| {
                        ui.checkbox(&mut s.smart_match_case, "Match case");
                        ui.checkbox(&mut s.smart_whole_word, "Whole word only");
                    });
                });
                group(ui, "Caret and selection");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.caret_blink, "Blink");
                    ui.label("Width:");
                    ui.add(
                        egui::DragValue::new(&mut s.caret_width)
                            .range(1.0..=4.0)
                            .speed(0.25),
                    );
                });
                ui.checkbox(&mut s.multi_editing, "Ctrl/⌘ + click adds a caret");
                ui.checkbox(
                    &mut s.copy_line_without_selection,
                    "Copy and cut the whole line when nothing is selected",
                );
                ui.checkbox(&mut s.smooth_scrolling, "Smooth scrolling");
            }
            settings_section::NEW_DOCUMENTS => {
                let s = &mut self.settings;
                group(ui, "Line endings");
                ui.horizontal(|ui| {
                    use crate::encoding::Eol;
                    ui.radio_value(&mut s.default_eol, Eol::Crlf, "Windows (CR LF)");
                    ui.radio_value(&mut s.default_eol, Eol::Lf, "Unix and macOS (LF)");
                    ui.radio_value(&mut s.default_eol, Eol::Cr, "Classic Mac (CR)");
                });
                group(ui, "Encoding");
                for (k, l) in [
                    ("ansi", "ANSI"),
                    ("utf8", "UTF-8"),
                    ("utf8bom", "UTF-8 with BOM"),
                    ("utf16be", "UTF-16 Big Endian with BOM"),
                    ("utf16le", "UTF-16 Little Endian with BOM"),
                ] {
                    ui.radio_value(&mut s.default_encoding, k.to_string(), l);
                }
                group(ui, "Language");
                let cur = self
                    .langs
                    .by_name(&s.default_lang)
                    .map(|i| self.langs.langs[i].display.clone())
                    .unwrap_or_default();
                egui::ComboBox::from_id_salt("deflang")
                    .selected_text(cur)
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for l in &self.langs.langs {
                            ui.selectable_value(
                                &mut s.default_lang,
                                l.name.clone(),
                                l.display.clone(),
                            );
                        }
                    });
            }
            settings_section::COMPLETION => {
                let s = &mut self.settings;
                group(ui, "Suggestions");
                ui.checkbox(
                    &mut s.auto_complete,
                    "Suggest words and keywords while typing",
                );
                ui.add_enabled_ui(s.auto_complete, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("After");
                        ui.add(egui::DragValue::new(&mut s.auto_complete_min_chars).range(1..=9));
                        ui.label("characters");
                    });
                });
                group(ui, "Insert the closing pair");
                ui.checkbox(&mut s.auto_close_brackets, "Brackets: ( ) [ ] { }");
                ui.checkbox(&mut s.auto_close_quotes, "Quotes: \" \" ' '");
            }
            settings_section::FILES => {
                let s = &mut self.settings;
                group(ui, "Session");
                ui.checkbox(
                    &mut s.remember_session,
                    "Reopen the last session's files on start",
                );
                ui.checkbox(
                    &mut s.backup_unsaved,
                    "Keep unsaved changes between sessions (backup every 3 s)",
                );
                ui.checkbox(&mut s.backup_on_save, "Keep a .bak copy when saving");
                group(ui, "Changes made by other programs");
                ui.checkbox(&mut s.file_monitoring, "Detect them");
                ui.add_enabled_ui(s.file_monitoring, |ui| {
                    ui.checkbox(&mut s.update_silently, "Reload without asking");
                    ui.checkbox(
                        &mut s.scroll_to_end_on_update,
                        "Scroll to the end after reloading",
                    );
                });
                group(ui, "Inserted date and time");
                ui.horizontal(|ui| {
                    ui.label("Custom format:");
                    ui.text_edit_singleline(&mut s.date_time_format);
                });
                ui.label(
                    RichText::new(format!(
                        "Example: {}",
                        crate::config::format_date(&s.date_time_format, chrono::Local::now())
                    ))
                    .weak(),
                );
            }
            settings_section::FORMATTING => {
                use crate::formatters::{XmlQuotes, XmlWhitespace};
                let f = &mut self.settings.format;
                ui.label(RichText::new("Used by Tools > Format.").weak());
                group(ui, "All formats");
                ui.horizontal(|ui| {
                    ui.label("Indent width:");
                    ui.add(egui::DragValue::new(&mut f.tab_width).range(1..=8));
                    ui.checkbox(&mut f.use_tabs, "Indent with tabs");
                });
                ui.horizontal(|ui| {
                    ui.label("Line width:");
                    ui.add(egui::DragValue::new(&mut f.print_width).range(20..=400));
                });
                group(ui, "JSON");
                ui.checkbox(
                    &mut f.json_object_wrap_preserve,
                    "Keep one-line objects on one line",
                );
                group(ui, "XML");
                ui.horizontal(|ui| {
                    ui.label("Whitespace:");
                    ui.radio_value(&mut f.xml_whitespace, XmlWhitespace::Strict, "Keep");
                    ui.radio_value(&mut f.xml_whitespace, XmlWhitespace::Ignore, "Ignore");
                });
                ui.horizontal(|ui| {
                    ui.label("Attribute quotes:");
                    ui.radio_value(&mut f.xml_quotes, XmlQuotes::Preserve, "Keep");
                    ui.radio_value(&mut f.xml_quotes, XmlQuotes::Double, "Double");
                    ui.radio_value(&mut f.xml_quotes, XmlQuotes::Single, "Single");
                });
                ui.checkbox(&mut f.xml_self_closing_space, "Space before />");
                ui.checkbox(&mut f.xml_sort_attributes, "Sort attributes by name");
                ui.checkbox(&mut f.single_attribute_per_line, "One attribute per line");
                ui.checkbox(
                    &mut f.bracket_same_line,
                    "Keep > on the last attribute's line",
                );
                ui.add_space(6.0);
                if ui.button("Reset to Defaults").clicked() {
                    *f = crate::formatters::FormatOptions::default();
                }
            }
            _ => self.shortcut_settings(ui),
        }
    }

    /// Theme, fonts and interface size.
    fn appearance_settings(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        ui.label(RichText::new("Theme").strong());
        let mut follow = self.settings.follow_system_theme;
        ui.checkbox(&mut follow, "Match the system light/dark setting");
        if follow != self.settings.follow_system_theme {
            self.settings.follow_system_theme = follow;
            if !follow {
                // Keep showing what's on screen now.
                self.settings.theme = self.themes.themes[self.theme_idx].name.clone();
            }
            let dark = self.system_dark;
            self.follow_system_theme(dark);
        }
        let names: Vec<String> = self.themes.themes.iter().map(|t| t.name.clone()).collect();
        let combo = |ui: &mut Ui, id: &str, label: &str, current: &str| -> Option<usize> {
            let mut pick = None;
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(100.0, 18.0), egui::Sense::hover());
                ui.put(r, egui::Label::new(label).halign(egui::Align::LEFT));
                egui::ComboBox::from_id_salt(id)
                    .selected_text(current)
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            if ui.selectable_label(n == current, n).clicked() {
                                pick = Some(i);
                            }
                        }
                    });
            });
            pick
        };
        if self.settings.follow_system_theme {
            let (light, dark) = (
                self.settings.theme.clone(),
                self.settings.dark_theme.clone(),
            );
            if let Some(i) = combo(ui, "theme_light", "Light theme:", &light) {
                self.settings.theme = names[i].clone();
            }
            if let Some(i) = combo(ui, "theme_dark", "Dark theme:", &dark) {
                self.settings.dark_theme = names[i].clone();
            }
            let dark = self.system_dark;
            self.follow_system_theme(dark);
        } else {
            let current = names[self.theme_idx].clone();
            if let Some(i) = combo(ui, "theme", "Theme:", &current) {
                self.set_theme(i);
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Size").strong());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(100.0, 18.0), egui::Sense::hover());
            ui.put(r, egui::Label::new("Font size:").halign(egui::Align::LEFT));
            ui.add(
                egui::DragValue::new(&mut self.settings.font_size)
                    .range(6.0..=48.0)
                    .speed(0.5)
                    .suffix(" pt"),
            );
        });
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(100.0, 18.0), egui::Sense::hover());
            ui.put(r, egui::Label::new("Interface:").halign(egui::Align::LEFT));
            ui.add(
                egui::DragValue::new(&mut self.settings.ui_scale)
                    .range(0.6..=2.5)
                    .speed(0.05)
                    .suffix("×"),
            );
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Preview").strong());
        let g = &self.globals;
        egui::Frame::NONE
            .fill(g.bg)
            .inner_margin(8.0)
            .corner_radius(4.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("fn main() { println!(\"The quick brown fox\"); }")
                        .color(g.fg)
                        .monospace(),
                );
                ui.horizontal_wrapped(|ui| {
                    let swatch = |ui: &mut Ui, c: Color32, l: &str| {
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 2.0, c);
                        ui.label(RichText::new(l).color(g.fg).small());
                        ui.add_space(6.0);
                    };
                    swatch(ui, g.current_line_bg, "Current line");
                    swatch(ui, g.selection_bg, "Selection");
                    swatch(ui, g.caret, "Caret");
                    swatch(ui, g.smart_highlight, "Smart highlight");
                    swatch(ui, g.find_mark, "Find mark");
                    for (i, c) in g.mark_styles.iter().enumerate() {
                        swatch(ui, *c, &format!("Highlight {}", i + 1));
                    }
                });
            });
    }

    /// Every keyboard shortcut, named as in the menus, with a filter.
    fn shortcut_settings(&mut self, ui: &mut Ui) {
        if self.dialogs.shortcut_rows.is_none() {
            let entries = self.menu_entries();
            let mut rows: Vec<(String, String, String)> = self
                .shortcuts
                .iter()
                .map(|(c, sc)| match entries.iter().find(|e| e.cmd == *c) {
                    Some(e) => {
                        let label = e.label.trim();
                        // "1" under "Fold Level" reads as "Fold Level 1".
                        match e.path.rsplit_once(" > ") {
                            Some((parent, last)) if label.len() <= 2 => {
                                (format!("{last} {label}"), parent.to_string(), sc.label())
                            }
                            _ => (label.to_string(), e.path.clone(), sc.label()),
                        }
                    }
                    None => (format!("{c:?}"), String::new(), sc.label()),
                })
                .collect();
            rows.sort();
            self.dialogs.shortcut_rows = Some(rows);
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.dialogs.shortcut_filter)
                    .hint_text("Filter by name or key")
                    .desired_width(260.0),
            );
        });
        ui.add_space(4.0);
        let filter = self.dialogs.shortcut_filter.to_lowercase();
        egui::Grid::new("sc_grid")
            .striped(true)
            .num_columns(2)
            .min_col_width(110.0)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                for (name, path, key) in self.dialogs.shortcut_rows.iter().flatten() {
                    let hay = format!("{name} {path} {key}").to_lowercase();
                    if !filter.is_empty() && !hay.contains(&filter) {
                        continue;
                    }
                    ui.monospace(key);
                    ui.horizontal(|ui| {
                        ui.label(name);
                        if !path.is_empty() {
                            ui.label(RichText::new(format!("in {path}")).weak().small());
                        }
                    });
                    ui.end_row();
                }
            });
    }

    pub fn prompts_ui(&mut self, ctx: &Context) {
        if self.prompts.is_empty() {
            return;
        }
        let mut answer: Option<i32> = None;
        let prompt = &self.prompts[0];
        let (title, text, buttons): (String, String, Vec<(&str, i32)>) = match prompt {
            Prompt::SaveBeforeClose { doc, .. } => {
                let name = self
                    .docs
                    .get(doc)
                    .map(crate::document::Document::full_path_or_name)
                    .unwrap_or_default();
                (
                    "Save".into(),
                    format!("Save file \"{name}\" ?"),
                    vec![("Yes", 1), ("No", 0), ("Cancel", -1)],
                )
            }
            Prompt::Reload(id) => {
                let name = self
                    .docs
                    .get(id)
                    .map(crate::document::Document::full_path_or_name)
                    .unwrap_or_default();
                let dirty = self
                    .docs
                    .get(id)
                    .is_some_and(crate::document::Document::is_dirty);
                (
                    "Reload".into(),
                    if dirty {
                        format!(
                            "\"{name}\"\n\nThis file has been modified by another program.\nDo you want to reload it and lose the changes made in {APP_NAME}?"
                        )
                    } else {
                        format!(
                            "\"{name}\"\n\nThis file has been modified by another program.\nDo you want to reload it?"
                        )
                    },
                    vec![("Yes", 1), ("No", 0)],
                )
            }
            Prompt::Deleted(id) => {
                let name = self
                    .docs
                    .get(id)
                    .map(crate::document::Document::full_path_or_name)
                    .unwrap_or_default();
                (
                    "Keep non existing file".into(),
                    format!(
                        "The file \"{name}\" doesn't exist anymore.\nKeep this file in editor?"
                    ),
                    vec![("Yes", 1), ("No", 0)],
                )
            }
            Prompt::Message { title, text } => (title.clone(), text.clone(), vec![("OK", 1)]),
            Prompt::DeleteFile(p) => {
                let open = self.find_open(p).is_some_and(|(_, id)| {
                    self.docs
                        .get(&id)
                        .is_some_and(crate::document::Document::is_dirty)
                });
                (
                    "Delete File".into(),
                    format!(
                        "{} \"{}\"?{}",
                        crate::app::TRASH_LABEL,
                        p.display(),
                        if open {
                            "\n\nIts tab has unsaved changes, which will be lost."
                        } else {
                            ""
                        }
                    ),
                    vec![(crate::app::TRASH_LABEL, 1), ("Cancel", 0)],
                )
            }
        };
        egui::Modal::new(egui::Id::unique("prompt")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading(title);
            ui.add_space(6.0);
            ui.label(text);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                for (label, v) in buttons {
                    if ui.button(label).clicked() {
                        answer = Some(v);
                    }
                }
            });
        });
        if let Some(a) = answer {
            let p = self.prompts.remove(0);
            match p {
                Prompt::SaveBeforeClose { doc, then_exit } => {
                    let save = match a {
                        1 => Some(true),
                        0 => Some(false),
                        _ => None,
                    };
                    if save.is_none() {
                        self.prompts
                            .retain(|p| !matches!(p, Prompt::SaveBeforeClose { .. }));
                    }
                    self.finish_close(doc, save, then_exit);
                }
                Prompt::Reload(id) => self.answer_reload(id, a == 1),
                Prompt::Deleted(id) => self.answer_deleted(id, a == 1),
                Prompt::Message { .. } => {}
                Prompt::DeleteFile(p) => {
                    if a == 1
                        && let Err(e) = self.trash_file(&p)
                    {
                        self.message("Delete File", &e);
                    }
                }
            }
        }
    }
}
