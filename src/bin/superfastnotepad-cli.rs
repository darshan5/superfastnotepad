//! `superfastnotepad-cli`: the headless commands as a console program.
//!
//! The app itself is a GUI program on Windows, so cmd and PowerShell don't
//! wait for it or see its exit code. This one is a normal console program:
//! the shell waits, output goes to stdout (pipes and `>` work), and
//! `%ERRORLEVEL%` / `$LASTEXITCODE` hold the result. It contains no window
//! code, so it starts instantly. The commands are the same as
//! `superfastnotepad <command>` (src/cli.rs).

// The shared modules also hold editor-only helpers this program doesn't use.
#![allow(dead_code)]

#[path = "../cli.rs"]
mod cli;
#[path = "../cli_files.rs"]
mod cli_files;
#[path = "../cli_search.rs"]
mod cli_search;
#[path = "../cli_tools.rs"]
mod cli_tools;
#[path = "../datatools.rs"]
mod datatools;
#[path = "../encoding.rs"]
mod encoding;
#[path = "../formatters.rs"]
mod formatters;
#[path = "../formatters_web.rs"]
mod formatters_web;
#[path = "../password.rs"]
mod password;
#[path = "../search.rs"]
mod search;
#[path = "../textops.rs"]
mod textops;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("--version" | "-V" | "version") => {
            println!("superfastnotepad-cli {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("--help" | "-h") => {
            println!("{}", cli::help("superfastnotepad-cli"));
            0
        }
        None => {
            println!("{}", cli::help("superfastnotepad-cli"));
            2
        }
        Some(_) => cli::run(&args),
    };
    std::process::exit(code);
}
