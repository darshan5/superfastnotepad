//! `superfastnotepad-cli`: the headless commands as a console program.
//!
//! The app itself is a GUI program on Windows, so cmd and PowerShell don't
//! wait for it or see its exit code. This one is a normal console program:
//! the shell waits, output goes to stdout (pipes and `>` work), and
//! `%ERRORLEVEL%` / `$LASTEXITCODE` hold the result. It contains no window
//! code, so it starts instantly.

// The shared modules also hold editor-only helpers this program doesn't use.
#![allow(dead_code)]

#[path = "../cli_search.rs"]
mod cli_search;
#[path = "../datatools.rs"]
mod datatools;
#[path = "../encoding.rs"]
mod encoding;
#[path = "../search.rs"]
mod search;
#[path = "../textops.rs"]
mod textops;

const HELP: &str = "\
superfastnotepad-cli: headless commands for scripts and AI agents.

Usage:
  superfastnotepad-cli search [options]    search files (see `search --help`)
  superfastnotepad-cli --version";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("search") => cli_search::run(&args[1..]),
        Some("--version" | "-V") => {
            println!("superfastnotepad-cli {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("--help" | "-h" | "help") | None => {
            println!("{HELP}");
            i32::from(args.is_empty()) * 2
        }
        Some(other) => {
            eprintln!("superfastnotepad-cli: unknown command `{other}`\n\n{HELP}");
            2
        }
    };
    std::process::exit(code);
}
