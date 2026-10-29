//! Toolbar icons drawn as vector shapes, so they render identically on every
//! platform without depending on emoji fonts.

use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    New,
    Open,
    Save,
    SaveAll,
    Close,
    Cut,
    Copy,
    Paste,
    Undo,
    Redo,
    Find,
    Replace,
    ZoomIn,
    ZoomOut,
    SyncV,
    SyncH,
    Wrap,
    AllChars,
    IndentGuide,
    DocMap,
    DocList,
    FunctionList,
    Folder,
    Monitor,
}

fn page(p: &Painter, r: Rect, fg: Color32, fill: Color32) {
    let fold = r.width() * 0.32;
    let pts = vec![
        r.left_top(),
        pos2(r.right() - fold, r.top()),
        pos2(r.right(), r.top() + fold),
        r.right_bottom(),
        r.left_bottom(),
    ];
    p.add(Shape::convex_polygon(pts, fill, Stroke::new(1.0, fg)));
    p.line_segment(
        [
            pos2(r.right() - fold, r.top()),
            pos2(r.right() - fold, r.top() + fold),
        ],
        Stroke::new(1.0, fg),
    );
    p.line_segment(
        [
            pos2(r.right() - fold, r.top() + fold),
            pos2(r.right(), r.top() + fold),
        ],
        Stroke::new(1.0, fg),
    );
}

fn floppy(p: &Painter, r: Rect, fg: Color32, body: Color32) {
    p.rect_filled(r, CornerRadius::same(1), body);
    p.rect_stroke(
        r,
        CornerRadius::same(1),
        Stroke::new(1.0, fg),
        StrokeKind::Inside,
    );
    let shutter = Rect::from_min_size(
        r.min + vec2(r.width() * 0.25, 0.0),
        vec2(r.width() * 0.5, r.height() * 0.35),
    );
    p.rect_filled(shutter, CornerRadius::ZERO, Color32::from_gray(230));
    let label = Rect::from_min_size(
        pos2(r.left() + r.width() * 0.18, r.top() + r.height() * 0.55),
        vec2(r.width() * 0.64, r.height() * 0.4),
    );
    p.rect_filled(label, CornerRadius::ZERO, Color32::WHITE);
}

fn magnifier(p: &Painter, c: Pos2, rad: f32, fg: Color32) {
    p.circle_stroke(c, rad, Stroke::new(1.8, fg));
    let d = rad * 0.75;
    p.line_segment(
        [c + vec2(d, d), c + vec2(d * 2.2, d * 2.2)],
        Stroke::new(2.4, fg),
    );
}

fn arrow_head(p: &Painter, tip: Pos2, dir: egui::Vec2, size: f32, color: Color32) {
    let d = dir.normalized();
    let n = vec2(-d.y, d.x);
    let base = tip - d * size;
    p.add(Shape::convex_polygon(
        vec![tip, base + n * size * 0.6, base - n * size * 0.6],
        color,
        Stroke::NONE,
    ));
}

#[allow(clippy::too_many_lines)]
pub fn draw(p: &Painter, rect: Rect, icon: Icon, fg: Color32) {
    let r = rect.shrink(4.0);
    let c = r.center();
    let w = r.width();
    let st = Stroke::new(1.5, fg);
    let blue = Color32::from_rgb(0x3a, 0x78, 0xd8);
    let paper = Color32::from_rgb(0xfa, 0xfa, 0xf6);
    let yellow = Color32::from_rgb(0xf0, 0xc0, 0x40);
    let green = Color32::from_rgb(0x3c, 0xa0, 0x4c);
    let red = Color32::from_rgb(0xd8, 0x3c, 0x3c);
    match icon {
        Icon::New => {
            page(
                p,
                Rect::from_center_size(c, vec2(w * 0.7, w * 0.9)),
                fg,
                paper,
            );
            let s = c + vec2(w * 0.28, w * 0.28);
            p.line_segment(
                [s - vec2(3.5, 0.0), s + vec2(3.5, 0.0)],
                Stroke::new(2.0, green),
            );
            p.line_segment(
                [s - vec2(0.0, 3.5), s + vec2(0.0, 3.5)],
                Stroke::new(2.0, green),
            );
        }
        Icon::Open | Icon::Folder => {
            let body = Rect::from_min_max(
                pos2(r.left(), r.top() + w * 0.25),
                pos2(r.right(), r.bottom() - w * 0.08),
            );
            let tab =
                Rect::from_min_size(pos2(r.left(), r.top() + w * 0.12), vec2(w * 0.42, w * 0.2));
            p.rect_filled(tab, CornerRadius::same(1), yellow);
            p.rect_filled(body, CornerRadius::same(1), yellow);
            p.rect_stroke(
                body,
                CornerRadius::same(1),
                Stroke::new(1.0, fg),
                StrokeKind::Inside,
            );
        }
        Icon::Save => floppy(
            p,
            Rect::from_center_size(c, vec2(w * 0.85, w * 0.85)),
            fg,
            blue,
        ),
        Icon::SaveAll => {
            floppy(
                p,
                Rect::from_min_size(r.min + vec2(w * 0.25, 0.0), vec2(w * 0.7, w * 0.7)),
                fg,
                blue,
            );
            floppy(
                p,
                Rect::from_min_size(r.min + vec2(0.0, w * 0.3), vec2(w * 0.7, w * 0.7)),
                fg,
                blue,
            );
        }
        Icon::Close => {
            page(
                p,
                Rect::from_center_size(c, vec2(w * 0.7, w * 0.9)),
                fg,
                paper,
            );
            let s = c + vec2(w * 0.22, w * 0.22);
            let d = 3.5;
            p.line_segment([s - vec2(d, d), s + vec2(d, d)], Stroke::new(2.0, red));
            p.line_segment([s + vec2(-d, d), s + vec2(d, -d)], Stroke::new(2.0, red));
        }
        Icon::Cut => {
            let y = c.y + w * 0.22;
            p.circle_stroke(pos2(c.x - w * 0.22, y), w * 0.15, st);
            p.circle_stroke(pos2(c.x + w * 0.22, y), w * 0.15, st);
            p.line_segment(
                [
                    pos2(c.x - w * 0.12, y - w * 0.1),
                    pos2(c.x + w * 0.2, r.top()),
                ],
                st,
            );
            p.line_segment(
                [
                    pos2(c.x + w * 0.12, y - w * 0.1),
                    pos2(c.x - w * 0.2, r.top()),
                ],
                st,
            );
        }
        Icon::Copy => {
            page(
                p,
                Rect::from_min_size(r.min, vec2(w * 0.6, w * 0.75)),
                fg,
                paper,
            );
            page(
                p,
                Rect::from_min_size(r.min + vec2(w * 0.38, w * 0.25), vec2(w * 0.6, w * 0.75)),
                fg,
                paper,
            );
        }
        Icon::Paste => {
            let board = Rect::from_min_size(r.min + vec2(0.0, w * 0.1), vec2(w * 0.7, w * 0.9));
            p.rect_filled(
                board,
                CornerRadius::same(2),
                Color32::from_rgb(0xb0, 0x80, 0x50),
            );
            p.rect_filled(
                Rect::from_center_size(
                    pos2(board.center().x, board.top()),
                    vec2(w * 0.3, w * 0.16),
                ),
                CornerRadius::same(1),
                Color32::from_gray(120),
            );
            page(
                p,
                Rect::from_min_size(r.min + vec2(w * 0.38, w * 0.35), vec2(w * 0.6, w * 0.65)),
                fg,
                paper,
            );
        }
        Icon::Undo | Icon::Redo => {
            let left = icon == Icon::Undo;
            let rad = w * 0.36;
            let center = c + vec2(0.0, w * 0.1);
            let n = 14;
            let (a0, a1) = (std::f32::consts::PI * 1.05, std::f32::consts::PI * 2.0);
            let pts: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let a = a0 + (a1 - a0) * i as f32 / n as f32;
                    let x = if left { -a.cos() } else { a.cos() };
                    center + vec2(x * rad, a.sin() * rad)
                })
                .collect();
            p.add(Shape::line(pts.clone(), Stroke::new(2.0, blue)));
            let tip = *pts.last().unwrap();
            arrow_head(p, tip + vec2(0.0, 4.0), vec2(0.0, 1.0), 5.0, blue);
        }
        Icon::Find => magnifier(p, c - vec2(w * 0.12, w * 0.12), w * 0.3, fg),
        Icon::Replace => {
            magnifier(p, c - vec2(w * 0.2, w * 0.2), w * 0.24, fg);
            p.line_segment(
                [
                    pos2(c.x, r.bottom() - 2.0),
                    pos2(r.right(), r.bottom() - 2.0),
                ],
                Stroke::new(2.0, red),
            );
            arrow_head(
                p,
                pos2(r.right() + 1.0, r.bottom() - 2.0),
                vec2(1.0, 0.0),
                4.0,
                red,
            );
        }
        Icon::ZoomIn | Icon::ZoomOut => {
            let mc = c - vec2(w * 0.1, w * 0.1);
            magnifier(p, mc, w * 0.3, fg);
            let s = Stroke::new(1.6, fg);
            p.line_segment([mc - vec2(w * 0.15, 0.0), mc + vec2(w * 0.15, 0.0)], s);
            if icon == Icon::ZoomIn {
                p.line_segment([mc - vec2(0.0, w * 0.15), mc + vec2(0.0, w * 0.15)], s);
            }
        }
        Icon::SyncV => {
            p.line_segment([pos2(c.x - 3.0, r.top()), pos2(c.x - 3.0, r.bottom())], st);
            p.line_segment([pos2(c.x + 3.0, r.top()), pos2(c.x + 3.0, r.bottom())], st);
            arrow_head(p, pos2(c.x - 3.0, r.top() - 1.0), vec2(0.0, -1.0), 4.0, fg);
            arrow_head(
                p,
                pos2(c.x + 3.0, r.bottom() + 1.0),
                vec2(0.0, 1.0),
                4.0,
                fg,
            );
        }
        Icon::SyncH => {
            p.line_segment([pos2(r.left(), c.y - 3.0), pos2(r.right(), c.y - 3.0)], st);
            p.line_segment([pos2(r.left(), c.y + 3.0), pos2(r.right(), c.y + 3.0)], st);
            arrow_head(p, pos2(r.right() + 1.0, c.y - 3.0), vec2(1.0, 0.0), 4.0, fg);
            arrow_head(p, pos2(r.left() - 1.0, c.y + 3.0), vec2(-1.0, 0.0), 4.0, fg);
        }
        Icon::Wrap => {
            p.line_segment(
                [
                    pos2(r.left(), r.top() + 2.0),
                    pos2(r.right(), r.top() + 2.0),
                ],
                st,
            );
            let pts = vec![
                pos2(r.left(), c.y),
                pos2(r.right() - 2.0, c.y),
                pos2(r.right() - 2.0, r.bottom() - 2.0),
                pos2(c.x, r.bottom() - 2.0),
            ];
            p.add(Shape::line(pts, st));
            arrow_head(
                p,
                pos2(c.x - 3.0, r.bottom() - 2.0),
                vec2(-1.0, 0.0),
                4.0,
                fg,
            );
        }
        Icon::AllChars => {
            p.text(
                c,
                egui::Align2::CENTER_CENTER,
                "¶",
                egui::FontId::proportional(w * 1.1),
                fg,
            );
        }
        Icon::IndentGuide => {
            for (i, x) in [r.left() + 2.0, c.x, r.right() - 2.0].iter().enumerate() {
                let mut y = r.top() + i as f32 * 2.0;
                while y < r.bottom() {
                    p.line_segment([pos2(*x, y), pos2(*x, y + 1.5)], Stroke::new(1.0, fg));
                    y += 3.0;
                }
            }
        }
        Icon::DocMap => {
            p.rect_stroke(
                r,
                CornerRadius::same(1),
                Stroke::new(1.0, fg),
                StrokeKind::Inside,
            );
            for i in 0..5 {
                let y = r.top() + 3.0 + i as f32 * (w - 6.0) / 5.0;
                let len = [0.7, 0.5, 0.8, 0.4, 0.6][i];
                p.line_segment(
                    [
                        pos2(r.left() + 3.0, y),
                        pos2(r.left() + 3.0 + (w - 6.0) * len, y),
                    ],
                    Stroke::new(1.0, blue),
                );
            }
            p.rect_filled(
                Rect::from_min_size(pos2(r.left() + 1.0, c.y - 3.0), vec2(w - 2.0, 6.0)),
                CornerRadius::ZERO,
                Color32::from_rgba_unmultiplied(0x3a, 0x78, 0xd8, 60),
            );
        }
        Icon::DocList => {
            for i in 0..4 {
                let y = r.top() + 2.0 + i as f32 * (w - 4.0) / 3.0;
                p.rect_filled(
                    Rect::from_center_size(pos2(r.left() + 2.5, y), vec2(3.0, 3.0)),
                    CornerRadius::ZERO,
                    blue,
                );
                p.line_segment([pos2(r.left() + 6.0, y), pos2(r.right(), y)], st);
            }
        }
        Icon::FunctionList => {
            p.text(
                c,
                egui::Align2::CENTER_CENTER,
                "ƒ",
                egui::FontId::proportional(w * 1.1),
                fg,
            );
        }
        Icon::Monitor => {
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / 20.0;
                    c + vec2(a.cos() * w * 0.5, a.sin() * w * 0.28)
                })
                .collect();
            p.add(Shape::line(pts, st));
            p.circle_filled(c, w * 0.16, fg);
        }
    }
}
