//! Password generator: random passwords from the OS secure random source,
//! inserted at the caret or copied.

use egui::{Color32, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, pos2, vec2};

const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
pub const SYMBOLS: &str = "!@#$%^&*<>";

pub const MIN_LENGTH: usize = 4;
pub const MAX_LENGTH: usize = 64;

/// A uniformly random index below `n`, from the OS random source.
fn random_below(n: usize) -> Result<usize, String> {
    let n = u32::try_from(n).map_err(|e| e.to_string())?;
    // Reject the top partial range so every index is equally likely.
    let zone = u32::MAX - u32::MAX % n;
    loop {
        let r = getrandom::u32().map_err(|e| e.to_string())?;
        if r < zone {
            return Ok((r % n) as usize);
        }
    }
}

/// A password of `length` characters with upper and lower case letters,
/// plus digits and symbols when asked; each included set appears at least
/// once.
pub fn generate(length: usize, numbers: bool, symbols: bool) -> Result<String, String> {
    let mut sets = vec![LOWER, UPPER];
    if numbers {
        sets.push(DIGITS);
    }
    if symbols {
        sets.push(SYMBOLS);
    }
    let length = length.clamp(MIN_LENGTH.max(sets.len()), MAX_LENGTH);
    let all: Vec<char> = sets.iter().flat_map(|s| s.chars()).collect();
    let mut out: Vec<char> = Vec::with_capacity(length);
    for set in &sets {
        let chars: Vec<char> = set.chars().collect();
        out.push(chars[random_below(chars.len())?]);
    }
    while out.len() < length {
        out.push(all[random_below(all.len())?]);
    }
    // Fisher-Yates, so the required characters aren't always first.
    for i in (1..out.len()).rev() {
        out.swap(i, random_below(i + 1)?);
    }
    Ok(out.into_iter().collect())
}

pub struct PasswordGenerator {
    pub length: usize,
    pub numbers: bool,
    pub symbols: bool,
    pub password: String,
    pub error: Option<String>,
}

/// What the user did this frame.
pub enum PasswordAction {
    None,
    Insert(String),
    Close,
}

impl PasswordGenerator {
    pub fn new() -> Self {
        let mut g = Self {
            length: 16,
            numbers: true,
            symbols: true,
            password: String::new(),
            error: None,
        };
        g.regenerate();
        g
    }

    pub fn regenerate(&mut self) {
        match generate(self.length, self.numbers, self.symbols) {
            Ok(p) => {
                self.password = p;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> PasswordAction {
        let mut action = PasswordAction::None;
        let mut changed = false;
        ui.set_width(420.0);
        // The password, with copy and regenerate buttons.
        ui.horizontal(|ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(&self.password)
                        .monospace()
                        .size(18.0)
                        .strong(),
                )
                .truncate(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, Icon::Regenerate)
                    .on_hover_text("New password")
                    .clicked()
                {
                    changed = true;
                }
                if icon_button(ui, Icon::Copy).on_hover_text("Copy").clicked() {
                    ui.ctx().copy_text(self.password.clone());
                }
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Length");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(self.length.to_string()).weak());
            });
        });
        ui.spacing_mut().slider_width = ui.available_width();
        if ui
            .add(
                egui::Slider::new(&mut self.length, MIN_LENGTH..=MAX_LENGTH)
                    .show_value(false)
                    .trailing_fill(true),
            )
            .changed()
        {
            changed = true;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            changed |= toggle(ui, &mut self.numbers, "Numbers (0-9)");
            ui.add_space(16.0);
            changed |= toggle(ui, &mut self.symbols, &format!("Symbols ({SYMBOLS})"));
        });
        ui.label(
            RichText::new("Upper and lower case letters are always included.")
                .weak()
                .small(),
        );
        if let Some(e) = &self.error {
            ui.colored_label(Color32::from_rgb(0xD0, 0x30, 0x30), e);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Insert").clicked() {
                action = PasswordAction::Insert(self.password.clone());
            }
            if ui.button("Close").clicked() {
                action = PasswordAction::Close;
            }
        });
        if changed {
            self.regenerate();
        }
        action
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Copy,
    Regenerate,
}

fn icon_button(ui: &mut Ui, icon: Icon) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let v = ui.visuals();
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, v.widgets.hovered.weak_bg_fill);
    }
    let fg = v.text_color();
    let st = Stroke::new(1.5, fg);
    let p = ui.painter();
    let c = rect.center();
    match icon {
        Icon::Copy => {
            let back = Rect::from_center_size(c + vec2(-2.0, -2.0), vec2(10.0, 11.0));
            let front = back.translate(vec2(4.0, 4.0));
            p.rect_stroke(back, 2.0, st, egui::StrokeKind::Middle);
            p.rect_filled(front, 2.0, v.window_fill);
            p.rect_stroke(front, 2.0, st, egui::StrokeKind::Middle);
        }
        Icon::Regenerate => {
            // An open circle with an arrow head at its end.
            let r = 6.5;
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = -0.6 + (i as f32 / 20.0) * 5.2;
                    c + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            let tip = *pts.first().unwrap_or(&c);
            p.add(Shape::line(pts, st));
            p.add(Shape::convex_polygon(
                vec![
                    tip + vec2(-4.0, -3.5),
                    tip + vec2(2.5, -3.0),
                    tip + vec2(0.5, 3.0),
                ],
                fg,
                Stroke::NONE,
            ));
        }
    }
    resp
}

/// A switch: a pill with a knob on the right when on. Returns true if
/// clicked.
fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> bool {
    let size = vec2(34.0, 18.0);
    let resp = ui
        .horizontal(|ui| {
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
            let t = ui.ctx().animate_bool(resp.id, *on);
            let v = ui.visuals();
            let fill = if *on {
                v.selection.bg_fill
            } else {
                v.widgets.inactive.bg_fill
            };
            let p = ui.painter();
            p.rect_filled(rect, rect.height() / 2.0, fill);
            let x = egui::lerp((rect.left() + 9.0)..=(rect.right() - 9.0), t);
            p.circle_filled(pos2(x, rect.center().y), 6.5, Color32::WHITE);
            let label = ui.add(egui::Label::new(label).sense(Sense::click()));
            resp.union(label)
        })
        .inner;
    if resp.clicked() {
        *on = !*on;
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_have_the_requested_shape() {
        for _ in 0..200 {
            let p = generate(15, true, true).unwrap();
            assert_eq!(p.chars().count(), 15);
            assert!(p.chars().any(|c| c.is_ascii_lowercase()), "{p}");
            assert!(p.chars().any(|c| c.is_ascii_uppercase()), "{p}");
            assert!(p.chars().any(|c| c.is_ascii_digit()), "{p}");
            assert!(p.chars().any(|c| SYMBOLS.contains(c)), "{p}");
        }
        let letters = generate(40, false, false).unwrap();
        assert!(
            letters.chars().all(|c| c.is_ascii_alphabetic()),
            "{letters}"
        );
        assert_eq!(generate(1, true, true).unwrap().len(), MIN_LENGTH);
        assert_eq!(generate(500, true, false).unwrap().len(), MAX_LENGTH);
        // Two passwords in a row differ.
        assert_ne!(
            generate(32, true, true).unwrap(),
            generate(32, true, true).unwrap()
        );
    }

    #[test]
    fn random_index_is_in_range_and_covers_it() {
        let mut seen = [false; 7];
        for _ in 0..2000 {
            let i = random_below(7).unwrap();
            seen[i] = true;
        }
        assert!(seen.iter().all(|s| *s));
    }
}
