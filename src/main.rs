//! superfastnotepad — a fast text editor in pure Rust for Windows and macOS.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod calltip;
mod colorpick;
mod commands;
mod compare;
mod compare_ui;
mod config;
mod datatools;
mod dialogs;
mod document;
mod editor;
mod encoding;
mod filter;
mod formatters;
mod formatters_web;
mod function_list;
mod icons;
mod json_tree;
mod langs;
mod lexer;
mod ops;
mod panels;
mod password;
mod platform;
mod search;
mod textops;
mod theme;
mod ui;
mod update;

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
        take: None,
    };
    let mut multi = false;
    let mut args = raw.iter();
    while let Some(arg) = args.next() {
        if let Some(dir) = arg.strip_prefix("--session-dir=") {
            config::set_session_dir(dir.into());
            continue;
        }
        if arg == "--take" {
            a.take = args.next().map(Into::into);
            continue;
        }
        if arg == "--session-dir" {
            if let Some(dir) = args.next() {
                config::set_session_dir(dir.into());
            }
            continue;
        }
        match arg.as_str() {
            "-multiInst" => multi = true,
            "-nosession" => a.no_session = true,
            "-ro" => a.read_only = true,
            "-alwaysOnTop" => a.always_on_top = true,
            "--help" | "-h" => {
                println!(
                    "{} [--help] [-multiInst] [-nosession] [--session-dir <folder>] [-ro] [-alwaysOnTop] [-l<Language>] [-n<line>] [-c<column>] [filePath ...]",
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
    // Dialog titles use the heading size; the default (18) looks oversized
    // next to 13 pt body text.
    ctx.all_styles_mut(|s| {
        if let Some(f) = s.text_styles.get_mut(&egui::TextStyle::Heading) {
            f.size = 15.0;
        }
    });
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

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if !self.allow_close {
            self.save_all_state();
        }
        platform::release_instance_lock();
    }
}

fn app_icon() -> egui::IconData {
    // Same artwork as the macOS bundle icon (scripts/make-icon.py).
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap_or_default()
}

fn main() -> eframe::Result<()> {
    #[cfg(target_os = "macos")]
    platform::finder::install();
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
    // Restore the window where the session left it. Extra windows (opened
    // without the session) start a little down and to the right of it.
    if let Some(g) = config::load_session().window
        && g.width >= 200.0
        && g.height >= 150.0
    {
        let offset = if args.no_session { 30.0 } else { 0.0 };
        viewport = viewport
            .with_position([g.x + offset, g.y + offset])
            .with_inner_size([g.width, g.height])
            .with_maximized(g.maximized && !args.no_session);
    }
    if always_on_top {
        viewport = viewport.with_always_on_top();
    }
    let options = eframe::NativeOptions {
        viewport,
        // The session stores the window geometry itself (see above).
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        ui::APP_NAME,
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx)?;
            #[cfg(target_os = "macos")]
            {
                let ctx = cc.egui_ctx.clone();
                platform::finder::set_wake(move || ctx.request_repaint());
            }
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
