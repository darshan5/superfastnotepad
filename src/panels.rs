//! Docking panels: search results, incremental search, document list,
//! folder-as-workspace, function list, document map, character panel and
//! clipboard history.

use std::path::{Path, PathBuf};

use egui::{Color32, CornerRadius, RichText, Sense, Ui, text::LayoutJob};

use crate::app::App;
use crate::commands::Command;
use crate::document::{EditKind, Selection};

const ASCII_NAMES: [&str; 33] = [
    "NUL", "SOH", "STX", "ETX", "EOT", "ENQ", "ACK", "BEL", "BS", "TAB", "LF", "VT", "FF", "CR",
    "SO", "SI", "DLE", "DC1", "DC2", "DC3", "DC4", "NAK", "SYN", "ETB", "CAN", "EM", "SUB", "ESC",
    "FS", "GS", "RS", "US", "Space",
];

impl App {
    pub fn results_panel(&mut self, ui: &mut Ui) {
        let mut goto: Option<(usize, usize)> = None;
        let mut close = false;
        let Some(results) = &self.results else { return };
        let hits = results.total_hits();
        let header = format!(
            "Search \"{}\" ({hits} hits in {} file{}{})",
            results.query,
            results.files.len(),
            if results.files.len() == 1 { "" } else { "s" },
            if results.searched_files > 0 {
                format!(" of {} searched", results.searched_files)
            } else {
                String::new()
            }
        );
        let hl = Color32::from_rgb(0xFF, 0xD0, 0x40);
        let fg = ui.visuals().text_color();
        let cursor = self.results_cursor;
        ui.horizontal(|ui| {
            ui.strong(header);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("x").on_hover_text("Close").clicked() {
                    close = true;
                }
                if self.fif.is_some() {
                    ui.spinner();
                }
            });
        });
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (fi, f) in results.files.iter().enumerate() {
                    let n: usize = f.hits.iter().map(|h| h.ranges.len()).sum();
                    egui::CollapsingHeader::new(
                        RichText::new(format!(
                            "{} ({n} hit{})",
                            f.title,
                            if n == 1 { "" } else { "s" }
                        ))
                        .color(Color32::from_rgb(0x40, 0x90, 0x40)),
                    )
                    .id_salt(("res", fi, &f.title))
                    .default_open(true)
                    .show(ui, |ui| {
                        for (hi, h) in f.hits.iter().enumerate().take(5000) {
                            let mut job = LayoutJob::default();
                            let mono = egui::FontId::monospace(12.0);
                            job.append(
                                &format!("Line {:>5}: ", h.line + 1),
                                0.0,
                                egui::TextFormat::simple(mono.clone(), Color32::GRAY),
                            );
                            let mut last = 0;
                            let text = &h.text;
                            for &(s, e) in &h.ranges {
                                let (s, e) = (s.min(text.len()), e.min(text.len()));
                                if s < last
                                    || !text.is_char_boundary(s)
                                    || !text.is_char_boundary(e)
                                {
                                    continue;
                                }
                                job.append(
                                    &text[last..s],
                                    0.0,
                                    egui::TextFormat::simple(mono.clone(), fg),
                                );
                                job.append(
                                    &text[s..e],
                                    0.0,
                                    egui::TextFormat {
                                        font_id: mono.clone(),
                                        color: Color32::BLACK,
                                        background: hl,
                                        ..Default::default()
                                    },
                                );
                                last = e;
                            }
                            if last <= text.len() {
                                job.append(
                                    &text[last..],
                                    0.0,
                                    egui::TextFormat::simple(mono.clone(), fg),
                                );
                            }
                            let selected = cursor == Some((fi, hi));
                            let r = ui.add(
                                egui::Button::selectable(selected, job).frame_when_inactive(false),
                            );
                            if r.double_clicked() || r.clicked() {
                                goto = Some((fi, hi));
                            }
                        }
                    });
                }
            });
        if let Some((fi, hi)) = goto {
            self.goto_result(fi, hi);
        }
        if close {
            self.results_visible = false;
        }
    }

    pub fn incremental_bar(&mut self, ui: &mut Ui) {
        let mut changed = false;
        let mut step: Option<bool> = None;
        let mut close = false;
        ui.horizontal(|ui| {
            if ui.small_button("x").clicked() {
                close = true;
            }
            ui.label("Find :");
            let mut te =
                egui::TextEdit::singleline(&mut self.incremental.text).desired_width(260.0);
            if self.incremental.not_found {
                te = te
                    .background_color(Color32::from_rgb(0xFF, 0x90, 0x90))
                    .text_color(Color32::BLACK);
            }
            let r = ui.add(te);
            if std::mem::take(&mut self.incremental.focus) {
                r.request_focus();
            }
            if r.changed() {
                changed = true;
            }
            if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                step = Some(!ui.input(|i| i.modifiers.shift));
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                r.request_focus();
                step = Some(!ui.input(|i| i.modifiers.shift));
            }
            if ui.button("<").clicked() {
                step = Some(false);
            }
            if ui.button(">").clicked() {
                step = Some(true);
            }
            if ui
                .checkbox(&mut self.incremental.highlight_all, "Highlight all")
                .changed()
            {
                changed = true;
            }
            if ui
                .checkbox(&mut self.incremental.match_case, "Match case")
                .changed()
            {
                changed = true;
            }
            if self.incremental.not_found && !self.incremental.text.is_empty() {
                ui.label(RichText::new("Phrase not found").color(Color32::RED));
            }
        });
        if close {
            self.incremental.open = false;
            self.incremental.text.clear();
            self.incremental_update(None);
            self.request_focus_editor = true;
        } else if let Some(fwd) = step {
            self.incremental_update(Some(fwd));
        } else if changed {
            self.incremental_update(None);
        }
    }

    pub fn left_panels(&mut self, ui: &mut Ui) {
        let both = self.settings.show_doc_list && self.settings.show_folder_tree;
        if self.settings.show_doc_list {
            ui.horizontal(|ui| {
                ui.strong("Document List");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("x").clicked() {
                        self.settings.show_doc_list = false;
                    }
                });
            });
            let mut act = None;
            let h = if both {
                ui.available_height() * 0.4
            } else {
                ui.available_height()
            };
            egui::ScrollArea::vertical()
                .id_salt("doclist")
                .max_height(h)
                .show(ui, |ui| {
                    egui::Grid::new("doclist_grid")
                        .striped(true)
                        .num_columns(2)
                        .show(ui, |ui| {
                            for (pi, p) in self.panes.iter().enumerate() {
                                for (ti, id) in p.tabs.iter().enumerate() {
                                    let Some(d) = self.docs.get(id) else { continue };
                                    let active = pi == self.focused && ti == p.active;
                                    let r = ui
                                        .selectable_label(active, App::doc_label(d))
                                        .on_hover_text(d.full_path_or_name());
                                    if r.clicked() {
                                        act = Some((pi, *id));
                                    }
                                    let ext = d
                                        .display_name()
                                        .rsplit_once('.')
                                        .map(|x| x.1.to_string())
                                        .unwrap_or_default();
                                    ui.label(RichText::new(ext).weak());
                                    ui.end_row();
                                }
                            }
                        });
                });
            if let Some((p, id)) = act {
                self.activate(p, id);
            }
            if both {
                ui.separator();
            }
        }
        if self.settings.show_folder_tree {
            let mut open_file = None;
            let mut remove = None;
            ui.horizontal(|ui| {
                ui.strong("Folder as Workspace");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("x").clicked() {
                        self.settings.show_folder_tree = false;
                    }
                    if ui.small_button("+").on_hover_text("Add folder").clicked()
                        && let Some(d) = rfd::FileDialog::new().pick_folder()
                    {
                        self.add_workspace_folder(d);
                    }
                });
            });
            egui::ScrollArea::both()
                .id_salt("foldertree")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (i, root) in self.settings.workspace_folders.clone().iter().enumerate() {
                        let p = PathBuf::from(root);
                        let name = p
                            .file_name()
                            .map_or_else(|| root.clone(), |n| n.to_string_lossy().to_string());
                        let r =
                            egui::CollapsingHeader::new(RichText::new(name.to_string()).strong())
                                .id_salt(("root", root))
                                .default_open(true)
                                .show(ui, |ui| self.dir_tree(ui, &p, 0, &mut open_file));
                        r.header_response.context_menu(|ui| {
                            if ui.button("Remove").clicked() {
                                remove = Some(i);
                                ui.close();
                            }
                            if ui
                                .button(if cfg!(target_os = "macos") {
                                    "Reveal in Finder"
                                } else {
                                    "Explore"
                                })
                                .clicked()
                            {
                                crate::platform::open_with_default_app(root);
                                ui.close();
                            }
                            if ui.button("Find in Files...").clicked() {
                                self.find.fif_dir = root.clone();
                                self.find.open = true;
                                self.find.tab = crate::app::FindTab::FindInFiles;
                                ui.close();
                            }
                        });
                    }
                });
            if let Some(i) = remove {
                self.settings.workspace_folders.remove(i);
            }
            if let Some(f) = open_file {
                self.open_path(&f, self.focused);
            }
        }
    }

    fn dir_tree(&self, ui: &mut Ui, dir: &Path, depth: usize, open_file: &mut Option<PathBuf>) {
        if depth > 24 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<(bool, String, PathBuf)> = rd
            .flatten()
            .map(|e| {
                let p = e.path();
                (p.is_dir(), e.file_name().to_string_lossy().to_string(), p)
            })
            .filter(|(_, n, _)| !n.starts_with('.'))
            .collect();
        entries.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
        });
        for (is_dir, name, path) in entries.into_iter().take(2000) {
            if is_dir {
                egui::CollapsingHeader::new(format!("{name}/"))
                    .id_salt(&path)
                    .default_open(false)
                    .show(ui, |ui| self.dir_tree(ui, &path, depth + 1, open_file));
            } else {
                let r = ui.add(egui::Label::new(format!("   {name}")).sense(Sense::click()));
                if r.double_clicked() {
                    *open_file = Some(path.clone());
                }
                r.on_hover_text(path.display().to_string());
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    pub fn right_panels(&mut self, ui: &mut Ui, _out: &mut [Command]) {
        let sections = [
            self.settings.show_function_list,
            self.settings.show_doc_map,
            self.settings.show_char_panel,
            self.settings.show_clipboard_history,
        ];
        let count = sections.iter().filter(|x| **x).count().max(1);
        let each = (ui.available_height() / count as f32) - 30.0;
        if self.settings.show_function_list {
            self.function_list_panel(ui, each);
            ui.separator();
        }
        if self.settings.show_doc_map {
            self.doc_map_panel(ui, each);
            ui.separator();
        }
        if self.settings.show_char_panel {
            ui.horizontal(|ui| {
                ui.strong("ASCII Codes Insertion Panel");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("x").clicked() {
                        self.settings.show_char_panel = false;
                    }
                });
            });
            let mut insert: Option<char> = None;
            egui::ScrollArea::vertical()
                .id_salt("charpanel")
                .max_height(each)
                .show(ui, |ui| {
                    egui::Grid::new("chars")
                        .striped(true)
                        .num_columns(4)
                        .show(ui, |ui| {
                            ui.strong("Value");
                            ui.strong("Hex");
                            ui.strong("Character");
                            ui.strong("HTML");
                            ui.end_row();
                            for v in 0u32..256 {
                                let c = char::from_u32(v).unwrap_or(' ');
                                let shown = if v <= 32 {
                                    ASCII_NAMES[v as usize].to_string()
                                } else if v == 127 {
                                    "DEL".into()
                                } else {
                                    c.to_string()
                                };
                                let r =
                                    ui.add(egui::Label::new(v.to_string()).sense(Sense::click()));
                                ui.label(format!("{v:02X}"));
                                let r2 = ui.add(
                                    egui::Label::new(RichText::new(shown).monospace())
                                        .sense(Sense::click()),
                                );
                                let html = match v {
                                    34 => "&quot;".to_string(),
                                    38 => "&amp;".to_string(),
                                    60 => "&lt;".to_string(),
                                    62 => "&gt;".to_string(),
                                    160..=255 => format!("&#{v};"),
                                    _ => String::new(),
                                };
                                ui.label(html);
                                ui.end_row();
                                if r.double_clicked() || r2.double_clicked() {
                                    insert = Some(c);
                                }
                            }
                        });
                });
            if let Some(c) = insert {
                self.with_active(|d, v, _| {
                    crate::ops::insert_text(d, v, &c.to_string(), EditKind::Other)
                });
                self.request_focus_editor = true;
            }
            ui.separator();
        }
        if self.settings.show_clipboard_history {
            ui.horizontal(|ui| {
                ui.strong("Clipboard History");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("x").clicked() {
                        self.settings.show_clipboard_history = false;
                    }
                });
            });
            let mut paste: Option<String> = None;
            egui::ScrollArea::vertical()
                .id_salt("cliphist")
                .max_height(each)
                .show(ui, |ui| {
                    for h in &self.clip.history {
                        let preview: String = h
                            .chars()
                            .take(80)
                            .map(|c| if c == '\n' || c == '\r' { '⏎' } else { c })
                            .collect();
                        let r = ui.add(
                            egui::Label::new(RichText::new(preview).monospace())
                                .sense(Sense::click()),
                        );
                        if r.double_clicked() {
                            paste = Some(h.clone());
                        }
                        r.on_hover_text("Double-click to paste");
                    }
                });
            if let Some(t) = paste {
                self.with_active(|d, v, _| crate::ops::insert_text(d, v, &t, EditKind::Other));
                self.request_focus_editor = true;
            }
        }
    }

    fn function_list_panel(&mut self, ui: &mut Ui, height: f32) {
        ui.horizontal(|ui| {
            ui.strong("Function List");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("x").clicked() {
                    self.settings.show_function_list = false;
                }
                if ui.small_button("Reload").on_hover_text("Reload").clicked() {
                    self.func_cache = None;
                }
            });
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.func_filter)
                .hint_text("Search")
                .desired_width(f32::INFINITY),
        );
        let Some(id) = self.active_id() else { return };
        let Some(doc) = self.docs.get(&id) else {
            return;
        };
        let lang = self.langs.langs[doc.lang].name.clone();
        let stale =
            !matches!(&self.func_cache, Some((cid, v, _)) if *cid == id && *v == doc.version);
        if stale {
            let text = doc.text();
            let items = if self.function_lists.has_parser(&lang) && text.len() < 20_000_000 {
                self.function_lists.parse(&lang, &text)
            } else {
                Vec::new()
            };
            self.func_cache = Some((id, doc.version, items));
        }
        let Some((_, _, items)) = &self.func_cache else {
            return;
        };
        let caret_line = self
            .views
            .get(&(self.focused, id))
            .map_or(0, |v| doc.char_to_line(v.primary().head));
        let current = items.iter().rposition(|i| i.line <= caret_line);
        let filter = self.func_filter.to_lowercase();
        let mut jump = None;
        egui::ScrollArea::vertical()
            .id_salt("funclist")
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if items.is_empty() {
                    ui.label(
                        RichText::new(if self.function_lists.has_parser(&lang) {
                            "No functions found"
                        } else {
                            "No function list parser for this language"
                        })
                        .weak(),
                    );
                }
                let mut classes: Vec<Option<String>> = Vec::new();
                for it in items {
                    if !classes.contains(&it.class) {
                        classes.push(it.class.clone());
                    }
                }
                for class in classes {
                    let mut body = |ui: &mut Ui| {
                        for (i, it) in items.iter().enumerate().filter(|(_, it)| it.class == class)
                        {
                            if !filter.is_empty() && !it.name.to_lowercase().contains(&filter) {
                                continue;
                            }
                            let r =
                                ui.selectable_label(current == Some(i), format!("ƒ {}", it.name));
                            if r.clicked() {
                                jump = Some(it.line);
                            }
                        }
                    };
                    match &class {
                        Some(c) => {
                            egui::CollapsingHeader::new(RichText::new(c.to_string()).strong())
                                .id_salt(("fl", c))
                                .default_open(true)
                                .show(ui, body);
                        }
                        None => body(ui),
                    }
                }
            });
        if let Some(l) = jump {
            self.with_active(|d, v, _| {
                d.ensure_line_visible(l);
                v.set_single(d.line_start(l));
                v.center_caret = true;
            });
            self.request_focus_editor = true;
        }
    }

    fn doc_map_panel(&mut self, ui: &mut Ui, height: f32) {
        ui.horizontal(|ui| {
            ui.strong("Document Map");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("x").clicked() {
                    self.settings.show_doc_map = false;
                }
            });
        });
        let pane = self.focused;
        let Some(id) = self.active_id() else { return };
        let lang = self.docs.get(&id).map_or(0, |d| d.lang);
        let styles = self.styles_for(lang).clone();
        let g = self.globals.clone();
        let Some(doc) = self.docs.get_mut(&id) else {
            return;
        };
        let Some(vs) = self.views.get_mut(&(pane, id)) else {
            return;
        };
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height.max(80.0)),
            Sense::click_and_drag(),
        );
        let p = ui.painter_at(rect);
        p.rect_filled(rect, CornerRadius::ZERO, g.bg);
        let n = doc.line_count();
        let lh = 2.0f32;
        let max_lines = (rect.height() / lh) as usize;
        // Scroll the map so the viewport stays visible.
        let top = vs.top_row as usize;
        let first = if n <= max_lines {
            0
        } else {
            top.saturating_sub(max_lines / 3).min(n - max_lines)
        };
        let cw = 1.2f32;
        for i in 0..max_lines.min(n - first) {
            let l = first + i;
            let text = doc.line_text(l);
            if text.trim().is_empty() {
                continue;
            }
            let spans = doc.line_spans(l, &text);
            let y = rect.min.y + i as f32 * lh;
            let indent = text
                .chars()
                .take_while(|c| c.is_whitespace())
                .map(|c| if c == '\t' { 4 } else { 1 })
                .sum::<usize>();
            let x0 = rect.min.x + 2.0 + indent as f32 * cw;
            let len = text
                .trim_end()
                .chars()
                .count()
                .saturating_sub(indent)
                .min(200);
            let color = spans
                .iter()
                .find(|s| s.2 != crate::lexer::TokenKind::Default)
                .map_or(g.fg, |s| styles.get(s.2).fg);
            let c = crate::editor::translucent(color, 150);
            p.rect_filled(
                egui::Rect::from_min_size(egui::pos2(x0, y), egui::vec2(len as f32 * cw, lh - 0.5)),
                CornerRadius::ZERO,
                c,
            );
        }
        // Viewport rectangle.
        let vis = vs.visible_rows as f32;
        let vy = rect.min.y + (top as f32 - first as f32) * lh;
        let vr = egui::Rect::from_min_size(
            egui::pos2(rect.min.x, vy),
            egui::vec2(rect.width(), vis * lh),
        );
        p.rect_filled(vr, CornerRadius::ZERO, crate::editor::translucent(g.fg, 30));
        p.rect_stroke(
            vr,
            CornerRadius::ZERO,
            egui::Stroke::new(1.0, crate::editor::translucent(g.fg, 90)),
            egui::StrokeKind::Inside,
        );
        if (resp.clicked() || resp.dragged())
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let l = first as f32 + (pos.y - rect.min.y) / lh;
            vs.top_row = f64::from((l - vis / 2.0).max(0.0));
        }
        let _ = Selection::default();
    }
}
