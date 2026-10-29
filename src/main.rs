//! superfastnotepad — a fast text editor in pure Rust for Windows and macOS.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod commands;
mod compare;
mod compare_ui;
mod config;
mod dialogs;
mod document;
mod editor;
mod encoding;
mod formatters;
mod function_list;
mod icons;
mod langs;
mod lexer;
mod ops;
mod panels;
mod platform;
mod search;
mod textops;
mod theme;
mod ui;

use std::sync::Arc;

use app::{App, StartupArgs};

fn parse_args() -> (StartupArgs, bool, Vec<String>) {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut a = StartupArgs {
        files: Vec::new(),
        line: None,
        column: None,
        lang: None,
        no_session: false,
        read_only: false,
        always_on_top: false,
    };
    let mut multi = false;
    for arg in &raw {
        match arg.as_str() {
            "-multiInst" => multi = true,
            "-nosession" => a.no_session = true,
            "-ro" => a.read_only = true,
            "-alwaysOnTop" => a.always_on_top = true,
            "--help" | "-h" => {
                println!(
                    "{} [--help] [-multiInst] [-nosession] [-ro] [-alwaysOnTop] [-l<Language>] [-n<line>] [-c<column>] [filePath ...]",
                    ui::APP_NAME
                );
                std::process::exit(0);
            }
            s if s.starts_with("-n") => a.line = s[2..].parse().ok(),
            s if s.starts_with("-c") => a.column = s[2..].parse().ok(),
            s if s.starts_with("-l") => a.lang = Some(s[2..].to_lowercase()),
            s if s.starts_with('-') => {}
            s => a.files.push(s.to_string()),
        }
    }
    (a, multi, raw)
}

/// Loads the one font the whole UI uses: the platform's built-in monospace
/// face (Menlo on macOS, Consolas on Windows). No fonts are bundled.
fn setup_fonts(ctx: &egui::Context) -> Result<(), String> {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/Menlo.ttc",
            "/System/Library/Fonts/Monaco.ttf",
        ]
    } else if cfg!(windows) {
        &[
            "C:\\Windows\\Fonts\\consola.ttf",
            "C:\\Windows\\Fonts\\cour.ttf",
        ]
    } else {
        &["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"]
    };
    let bytes = candidates
        .iter()
        .find_map(|p| std::fs::read(p).ok())
        .ok_or_else(|| format!("no system font found (tried {})", candidates.join(", ")))?;
    let mut fonts = egui::FontDefinitions::empty();
    fonts
        .font_data
        .insert("system".into(), Arc::new(egui::FontData::from_owned(bytes)));
    for fam in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
        fonts.families.insert(fam, vec!["system".into()]);
    }
    ctx.set_fonts(fonts);
    Ok(())
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let scale = self.settings.ui_scale.clamp(0.6, 2.5);
        if (ui.ctx().zoom_factor() - scale).abs() > 0.001 {
            ui.ctx().set_zoom_factor(scale);
        }
        let start = std::time::Instant::now();
        self.frame(ui);
        if let Some(p) = &mut self.profile {
            p.record(start.elapsed(), &ui.ctx().repaint_causes());
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        // Periodic backup of the session and unsaved content.
        if !self.exit_requested {
            self.save_all_state();
        }
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(7)
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if !self.allow_close {
            self.save_all_state();
        }
        platform::release_instance_lock();
    }
}

fn app_icon() -> egui::IconData {
    // A simple generated icon: green notepad with lines.
    let (w, h) = (64u32, 64u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let inside = (6..58).contains(&x) && (4..60).contains(&y);
            let (r, g, b, a) = if !inside {
                (0, 0, 0, 0)
            } else if y < 14 {
                (0x3c, 0x9a, 0x3c, 255)
            } else if (y - 14) % 8 == 6 && (12..52).contains(&x) {
                (0x70, 0x90, 0xb0, 255)
            } else {
                (0xf4, 0xf8, 0xf0, 255)
            };
            rgba[i..i + 4].copy_from_slice(&[r, g, b, a]);
        }
    }
    egui::IconData {
        rgba,
        width: w,
        height: h,
    }
}

fn main() -> eframe::Result<()> {
    let (args, multi, raw) = parse_args();
    let settings = config::load_settings();
    if settings.single_instance && !multi && platform::forward_to_running_instance(&raw) {
        return Ok(());
    }
    let always_on_top = args.always_on_top;
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(ui::APP_NAME)
        .with_inner_size([1100.0, 760.0])
        .with_min_inner_size([420.0, 300.0])
        .with_drag_and_drop(true)
        .with_icon(app_icon());
    if always_on_top {
        viewport = viewport.with_always_on_top();
    }
    let options = eframe::NativeOptions {
        viewport,
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native(
        ui::APP_NAME,
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx)?;
            let mut app = App::new(args);
            if app.settings.single_instance && !multi {
                let (tx, rx) = std::sync::mpsc::channel();
                let ctx = cc.egui_ctx.clone();
                platform::start_instance_server(tx, move || ctx.request_repaint());
                app.ipc_rx = Some(rx);
            }
            Ok(Box::new(app))
        }),
    )
}
