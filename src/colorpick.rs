//! Color picker: a saturation/brightness square with a hue strip, RGB, hex
//! and CMYK fields, and an eyedropper that samples any pixel on screen.
//! Picking inserts the color as `#RRGGBB` at the caret.

use egui::{Color32, Mesh, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, pos2, vec2};

/// Hue (0..360), saturation and brightness (0..1) to RGB.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [to(r), to(g), to(b)]
}

/// RGB to hue (0..360), saturation and brightness (0..1). Grays have hue 0.
pub fn rgb_to_hsv([r, g, b]: [u8; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

pub fn to_hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// Parses `#RRGGBB`, `RRGGBB`, `#RGB` or `RGB`.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    match s.len() {
        6 => Some([byte(0)?, byte(2)?, byte(4)?]),
        3 => {
            let d = |i: usize| u8::from_str_radix(&s[i..=i], 16).ok().map(|v| v * 17);
            Some([d(0)?, d(1)?, d(2)?])
        }
        _ => None,
    }
}

/// CMYK percentages (simple conversion, no color profile).
pub fn to_cmyk([r, g, b]: [u8; 3]) -> [u8; 4] {
    let (r, g, b) = (
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 {
        return [0, 0, 0, 100];
    }
    let pct = |f: f32| (f * 100.0).round() as u8;
    let ink = |c: f32| pct((1.0 - c - k) / (1.0 - k));
    [ink(r), ink(g), ink(b), pct(k)]
}

pub struct ColorPicker {
    pub hue: f32,
    pub sat: f32,
    pub val: f32,
    pub hex: String,
    /// The eyedropper is sampling the screen.
    pub sampling: bool,
}

/// What the user did in the picker this frame.
pub enum PickerAction {
    None,
    Insert(String),
    Cancel,
}

impl ColorPicker {
    pub fn new(start: [u8; 3]) -> Self {
        let (hue, sat, val) = rgb_to_hsv(start);
        Self {
            hue,
            sat,
            val,
            hex: to_hex(start)[1..].to_string(),
            sampling: false,
        }
    }

    /// The picked ("new") color.
    pub fn rgb(&self) -> [u8; 3] {
        hsv_to_rgb(self.hue, self.sat, self.val)
    }

    pub fn set_rgb(&mut self, rgb: [u8; 3]) {
        let (h, s, v) = rgb_to_hsv(rgb);
        // Keep the hue for grays so the square doesn't jump to red.
        if s > 0.0 {
            self.hue = h;
        }
        self.sat = s;
        self.val = v;
    }

    fn sync_hex(&mut self) {
        self.hex = to_hex(self.rgb())[1..].to_string();
    }

    pub fn ui(&mut self, ui: &mut Ui) -> PickerAction {
        let mut action = PickerAction::None;
        let mut changed = false;
        ui.horizontal_top(|ui| {
            changed |= self.square(ui);
            changed |= self.hue_strip(ui);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                self.swatch(ui);
                ui.add_space(4.0);
                changed |= self.fields(ui);
            });
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.spacing_mut().button_padding = vec2(6.0, 3.0);
                let w = 96.0;
                if ui
                    .add_sized([w, 22.0], egui::Button::new("Insert"))
                    .clicked()
                {
                    action = PickerAction::Insert(to_hex(self.rgb()));
                }
                if ui
                    .add_sized([w, 22.0], egui::Button::new("Cancel"))
                    .clicked()
                {
                    action = PickerAction::Cancel;
                }
                if ui
                    .add_sized([w, 22.0], egui::Button::new("Copy Hex"))
                    .clicked()
                {
                    ui.ctx().copy_text(to_hex(self.rgb()));
                }
                ui.add_space(6.0);
                self.cmyk(ui);
                ui.add_space(6.0);
                self.eyedropper_button(ui);
            });
        });
        if changed {
            self.sync_hex();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !self.sampling {
            action = PickerAction::Cancel;
        }
        action
    }

    /// Polls the eyedropper; call every frame while the picker is open.
    pub fn poll_sampler(&mut self, ctx: &egui::Context) {
        if !self.sampling {
            return;
        }
        match crate::platform::eyedropper::poll() {
            crate::platform::eyedropper::Sample::Pending(live) => {
                if let Some(rgb) = live {
                    self.set_rgb(rgb);
                    self.sync_hex();
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
            }
            crate::platform::eyedropper::Sample::Done(picked) => {
                self.sampling = false;
                if let Some(rgb) = picked {
                    self.set_rgb(rgb);
                    self.sync_hex();
                }
            }
        }
    }

    /// Saturation (left to right) and brightness (bottom to top) for the
    /// current hue.
    fn square(&mut self, ui: &mut Ui) -> bool {
        const SIZE: f32 = 170.0;
        const N: usize = 16;
        let (rect, resp) = ui.allocate_exact_size(vec2(SIZE, SIZE), Sense::click_and_drag());
        let mut mesh = Mesh::default();
        let color = |s: f32, v: f32| {
            let [r, g, b] = hsv_to_rgb(self.hue, s, v);
            Color32::from_rgb(r, g, b)
        };
        for row in 0..=N {
            for col in 0..=N {
                let (s, v) = (col as f32 / N as f32, 1.0 - row as f32 / N as f32);
                mesh.colored_vertex(rect.min + vec2(s * SIZE, (1.0 - v) * SIZE), color(s, v));
            }
        }
        let w = (N + 1) as u32;
        for row in 0..N as u32 {
            for col in 0..N as u32 {
                let i = row * w + col;
                mesh.add_triangle(i, i + 1, i + w);
                mesh.add_triangle(i + 1, i + w + 1, i + w);
            }
        }
        let p = ui.painter();
        p.add(Shape::mesh(mesh));
        let mut changed = false;
        if let Some(pos) = resp.interact_pointer_pos()
            && (resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on())
        {
            self.sat = ((pos.x - rect.left()) / SIZE).clamp(0.0, 1.0);
            self.val = 1.0 - ((pos.y - rect.top()) / SIZE).clamp(0.0, 1.0);
            changed = true;
        }
        let at = pos2(
            rect.left() + self.sat * SIZE,
            rect.top() + (1.0 - self.val) * SIZE,
        );
        let ring = if self.val > 0.55 && self.sat < 0.5 {
            Color32::BLACK
        } else {
            Color32::WHITE
        };
        p.circle_stroke(at, 6.0, Stroke::new(1.5, ring));
        changed
    }

    /// Vertical hue strip: red at the top and bottom.
    fn hue_strip(&mut self, ui: &mut Ui) -> bool {
        const W: f32 = 16.0;
        const H: f32 = 170.0;
        let (outer, resp) = ui.allocate_exact_size(vec2(W + 14.0, H), Sense::click_and_drag());
        let rect = Rect::from_center_size(outer.center(), vec2(W, H));
        let mut mesh = Mesh::default();
        let n = 36;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let [r, g, b] = hsv_to_rgb(360.0 * (1.0 - t), 1.0, 1.0);
            let c = Color32::from_rgb(r, g, b);
            let y = rect.top() + t * H;
            mesh.colored_vertex(pos2(rect.left(), y), c);
            mesh.colored_vertex(pos2(rect.right(), y), c);
        }
        for i in 0..n {
            let k = i * 2;
            mesh.add_triangle(k, k + 1, k + 2);
            mesh.add_triangle(k + 1, k + 3, k + 2);
        }
        let p = ui.painter();
        p.add(Shape::mesh(mesh));
        let mut changed = false;
        if let Some(pos) = resp.interact_pointer_pos()
            && (resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on())
        {
            self.hue = 360.0 * (1.0 - ((pos.y - rect.top()) / H).clamp(0.0, 1.0));
            changed = true;
        }
        let y = rect.top() + (1.0 - self.hue / 360.0) * H;
        let fg = ui.visuals().text_color();
        let tri = |tip: Pos2, dir: f32| {
            Shape::convex_polygon(
                vec![
                    tip,
                    tip + vec2(-7.0 * dir, -5.0),
                    tip + vec2(-7.0 * dir, 5.0),
                ],
                Color32::WHITE,
                Stroke::new(1.0, fg),
            )
        };
        p.add(tri(pos2(rect.left() - 1.0, y), 1.0));
        p.add(tri(pos2(rect.right() + 1.0, y), -1.0));
        changed
    }

    /// The picked color.
    fn swatch(&self, ui: &mut Ui) {
        let (r, _) = ui.allocate_exact_size(vec2(96.0, 40.0), Sense::hover());
        let [red, green, blue] = self.rgb();
        let p = ui.painter();
        p.rect_filled(r, 2.0, Color32::from_rgb(red, green, blue));
        p.rect_stroke(
            r,
            2.0,
            Stroke::new(1.0, ui.visuals().weak_text_color()),
            egui::StrokeKind::Outside,
        );
    }

    /// C, M, Y and K percentages (read only), two per row.
    fn cmyk(&self, ui: &mut Ui) {
        let [c, m, y, k] = to_cmyk(self.rgb());
        egui::Grid::new("color_cmyk")
            .num_columns(2)
            .min_col_width(45.0)
            .spacing([6.0, 3.0])
            .show(ui, |ui| {
                for row in [[("C", c), ("M", m)], [("Y", y), ("K", k)]] {
                    for (name, v) in row {
                        ui.label(format!("{name} {v}%"));
                    }
                    ui.end_row();
                }
            });
    }

    /// A large eyedropper icon that samples a color from the screen.
    fn eyedropper_button(&mut self, ui: &mut Ui) {
        use crate::platform::eyedropper;
        let size = vec2(96.0, 38.0);
        let enabled = eyedropper::SUPPORTED && !self.sampling;
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let v = ui.visuals();
        let bg = if self.sampling {
            v.selection.bg_fill
        } else if resp.hovered() && enabled {
            v.widgets.hovered.weak_bg_fill
        } else {
            v.widgets.inactive.weak_bg_fill
        };
        let fg = if enabled || self.sampling {
            v.text_color()
        } else {
            v.weak_text_color()
        };
        let p = ui.painter();
        p.rect_filled(rect, 6.0, bg);
        // A pipette pointing down-left: bulb, collar, glass tube and tip.
        let c = rect.center();
        let dir = vec2(-1.0, 1.0).normalized();
        let n = vec2(dir.y, -dir.x);
        let bulb = c - dir * 9.0;
        let collar = c - dir * 3.0;
        let tip = c + dir * 15.0;
        p.circle_filled(bulb, 6.0, fg);
        p.line_segment([collar - n * 6.0, collar + n * 6.0], Stroke::new(3.0, fg));
        p.line_segment([collar, tip - dir * 3.0], Stroke::new(5.0, fg));
        p.line_segment([tip - dir * 4.0, tip], Stroke::new(2.0, fg));
        let tube_fill = Color32::from_rgb(self.rgb()[0], self.rgb()[1], self.rgb()[2]);
        p.line_segment(
            [collar + dir * 2.0, tip - dir * 4.0],
            Stroke::new(2.0, tube_fill),
        );
        let tip_text = if self.sampling {
            "Picking..."
        } else {
            eyedropper::HINT
        };
        if resp
            .on_hover_text(format!("Pick from Screen: {tip_text}"))
            .clicked()
            && enabled
        {
            self.sampling = eyedropper::start();
        }
        if self.sampling {
            ui.label(RichText::new("Picking...").weak());
        }
    }

    fn fields(&mut self, ui: &mut Ui) -> bool {
        let mut changed = false;
        let rgb = self.rgb();
        egui::Grid::new("color_fields")
            .num_columns(2)
            .spacing([6.0, 3.0])
            .show(ui, |ui| {
                let num = |ui: &mut Ui, v: &mut i32| {
                    ui.add(egui::DragValue::new(v).range(0..=255).speed(0.5))
                        .changed()
                };
                // RGB.
                let mut channels = rgb.map(i32::from);
                let names = ["R:", "G:", "B:"];
                for (i, name) in names.iter().enumerate() {
                    ui.label(*name);
                    if num(ui, &mut channels[i]) {
                        let picked = channels.map(|v| v.clamp(0, 255) as u8);
                        self.set_rgb(picked);
                        changed = true;
                    }
                    ui.end_row();
                }
            });
        // Hex, outside the grid so the field isn't squeezed to its column.
        ui.horizontal(|ui| {
            ui.label("#");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.hex)
                    .desired_width(64.0)
                    .char_limit(7)
                    .font(egui::TextStyle::Monospace),
            );
            if resp.changed()
                && let Some(rgb) = parse_hex(&self.hex)
            {
                self.set_rgb(rgb);
            }
            if resp.lost_focus() {
                changed = true;
            }
        });
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_round_trip() {
        for rgb in [
            [0, 0, 0],
            [255, 255, 255],
            [255, 0, 0],
            [0, 128, 255],
            [18, 52, 86],
            [250, 128, 114],
        ] {
            let (h, s, v) = rgb_to_hsv(rgb);
            assert_eq!(hsv_to_rgb(h, s, v), rgb, "{rgb:?}");
        }
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), [0, 255, 0]);
        assert_eq!(hsv_to_rgb(360.0, 1.0, 1.0), [255, 0, 0]);
    }

    #[test]
    fn hex_and_cmyk() {
        assert_eq!(to_hex([255, 255, 255]), "#FFFFFF");
        assert_eq!(to_hex([0, 10, 171]), "#000AAB");
        assert_eq!(parse_hex("#ffffff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("1a2B3c"), Some([0x1a, 0x2b, 0x3c]));
        assert_eq!(parse_hex("#fa0"), Some([0xff, 0xaa, 0x00]));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#ggg"), None);
        assert_eq!(parse_hex("€€€"), None);
        assert_eq!(to_cmyk([0, 0, 0]), [0, 0, 0, 100]);
        assert_eq!(to_cmyk([255, 0, 0]), [0, 100, 100, 0]);
        assert_eq!(to_cmyk([255, 255, 255]), [0, 0, 0, 0]);
    }

    #[test]
    fn picker_keeps_hue_for_grays() {
        let mut p = ColorPicker::new([255, 0, 0]);
        p.hue = 200.0;
        p.set_rgb([128, 128, 128]);
        assert!((p.hue - 200.0).abs() < f32::EPSILON);
        assert_eq!(p.rgb(), [128, 128, 128]);
        assert_eq!(ColorPicker::new([255, 255, 255]).hex, "FFFFFF");
    }
}
