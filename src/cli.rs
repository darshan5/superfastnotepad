//! Headless commands (`search`, `tail`, `replace`, `hash`, ...), run by
//! `superfastnotepad-cli` and by the app's own program when its first
//! argument is a command name.
//!
//! Every command follows the same rules: `--out <file>` writes the result
//! there as UTF-8 instead of to stdout, `--help` prints its usage, and the
//! exit code is 0 = done / found, 1 = nothing found (or invalid, different),
//! 2 = error. Text input comes from `--text`, a file, or a pipe.

use std::io::{IsTerminal as _, Read as _, Write as _};
use std::path::{Path, PathBuf};

use crate::encoding::{Eol, TextEncoding};

pub struct Command {
    pub name: &'static str,
    pub summary: &'static str,
    pub usage: &'static str,
    run: fn(&[String]) -> Result<i32, String>,
}

fn search_cmd(a: &[String]) -> Result<i32, String> {
    Ok(crate::cli_search::run(a))
}

use crate::cli_files as f;
use crate::cli_tools as t;

pub const COMMANDS: &[Command] = &[
    Command {
        name: "search",
        summary: "find text or a regex in files (grep-style output)",
        usage: crate::cli_search::USAGE,
        run: search_cmd,
    },
    Command {
        name: "tail",
        summary: "last lines of a file; --follow prints new lines as they're written",
        usage: f::TAIL,
        run: f::tail,
    },
    Command {
        name: "head",
        summary: "first lines of a file",
        usage: f::HEAD,
        run: f::head,
    },
    Command {
        name: "lines",
        summary: "a range of lines, with line numbers",
        usage: f::LINES,
        run: f::lines,
    },
    Command {
        name: "append",
        summary: "add text to the end of a file (keeps its encoding and line endings)",
        usage: f::APPEND,
        run: f::append,
    },
    Command {
        name: "write",
        summary: "create or overwrite a file",
        usage: f::WRITE,
        run: f::write,
    },
    Command {
        name: "replace",
        summary: "find and replace in files (--dry-run to preview)",
        usage: f::REPLACE,
        run: f::replace,
    },
    Command {
        name: "info",
        summary: "size, date, encoding, line endings and line count of files",
        usage: f::INFO,
        run: f::info,
    },
    Command {
        name: "convert",
        summary: "change a file's encoding or line endings",
        usage: f::CONVERT,
        run: f::convert,
    },
    Command {
        name: "diff",
        summary: "differences between two files (unified diff)",
        usage: f::DIFF,
        run: f::diff,
    },
    Command {
        name: "files",
        summary: "list files by name pattern and date, newest first",
        usage: f::FILES,
        run: f::files,
    },
    Command {
        name: "format",
        summary: "pretty-print JSON, XML, HTML, CSS, JavaScript/TypeScript or Markdown",
        usage: t::FORMAT,
        run: t::format,
    },
    Command {
        name: "minify",
        summary: "minify JSON, XML or CSS",
        usage: t::MINIFY,
        run: t::minify,
    },
    Command {
        name: "validate",
        summary: "check that JSON, XML, CSV or TSV parses (error with line and column)",
        usage: t::VALIDATE,
        run: t::validate,
    },
    Command {
        name: "data",
        summary: "convert between CSV, TSV and JSON",
        usage: t::DATA,
        run: t::data,
    },
    Command {
        name: "base64",
        summary: "Base64 encode or decode",
        usage: t::BASE64,
        run: t::base64,
    },
    Command {
        name: "url",
        summary: "URL (percent) encode or decode",
        usage: t::URL,
        run: t::url,
    },
    Command {
        name: "qp",
        summary: "quoted-printable encode or decode",
        usage: t::QP,
        run: t::qp,
    },
    Command {
        name: "hash",
        summary: "MD5, SHA-1, SHA-256 or SHA-512 of files or text",
        usage: t::HASH,
        run: t::hash,
    },
    Command {
        name: "password",
        summary: "random passwords from the OS secure random source",
        usage: t::PASSWORD,
        run: t::password,
    },
    Command {
        name: "time",
        summary: "convert between Unix time, UTC and local time",
        usage: t::TIME,
        run: t::time,
    },
    Command {
        name: "uuid",
        summary: "random UUIDs (version 4)",
        usage: t::UUID,
        run: t::uuid,
    },
];

pub fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// Whether the app's first argument names a command (rather than a file).
pub fn is_command(arg: &str) -> bool {
    arg == "help" || find(arg).is_some()
}

/// The list of commands, for `--help`.
pub fn help(program: &str) -> String {
    let mut s = format!(
        "Usage: {program} <command> [options]\n\
         \n\
         Commands (`{program} <command> --help` for options):\n"
    );
    for c in COMMANDS {
        s.push_str(&format!("  {:<10}{}\n", c.name, c.summary));
    }
    s.push_str(
        "\nCommon options: --out <file> (write the result there, UTF-8), --text <s> \
         (input text instead of a file or a pipe).\n\
         Exit code: 0 = done or found, 1 = nothing found / invalid / different, 2 = error.",
    );
    s
}

/// Runs `args[0]` with the rest of `args`; returns the exit code.
pub fn run(args: &[String]) -> i32 {
    #[cfg(windows)]
    attach_console();
    let name = args.first().map_or("", String::as_str);
    if name == "help" {
        return match args.get(1) {
            None => {
                println!("{}", help("superfastnotepad-cli"));
                0
            }
            Some(c) => match find(c) {
                Some(c) => {
                    println!("{}", c.usage);
                    0
                }
                None => {
                    eprintln!("unknown command `{c}`\n\n{}", help("superfastnotepad-cli"));
                    2
                }
            },
        };
    }
    let Some(cmd) = find(name) else {
        eprintln!(
            "unknown command `{name}`\n\n{}",
            help("superfastnotepad-cli")
        );
        return 2;
    };
    let rest = &args[1..];
    if cmd.name != "search" && rest.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", cmd.usage);
        return 0;
    }
    match (cmd.run)(rest) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{}: {e}", cmd.name);
            // A Windows GUI build has nowhere else to show it.
            if let Some(p) = out_arg(rest) {
                let _ = std::fs::write(p, format!("error: {e}\n"));
            }
            2
        }
    }
}

fn out_arg(args: &[String]) -> Option<String> {
    args.iter().enumerate().find_map(|(i, a)| {
        a.strip_prefix("--out=")
            .map(str::to_string)
            .or_else(|| (a == "--out").then(|| args.get(i + 1).cloned()).flatten())
    })
}

/// A release Windows build of the app has no console of its own: borrow the
/// parent's (the cmd window it was started from) unless stdout is already
/// redirected. `superfastnotepad-cli` already has one, so this does nothing.
#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
    };
    // SAFETY: plain Win32 calls with constant arguments.
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h.is_null() || h == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

// ------------------------------------------------------------------ options

/// Parsed command line: positional arguments and `--options`.
#[derive(Debug, Default)]
pub struct Args {
    pub pos: Vec<String>,
    opts: Vec<(&'static str, Option<String>)>,
}

impl Args {
    /// `spec` lists the accepted options: `"--lines|-n="` is `--lines` (or
    /// `-n`) taking a value, `"--force"` a switch. `--out` and `--text` are
    /// given by the caller like any other. Values may follow as the next
    /// argument or after `=`. A lone `-` is positional (stdin), and so is
    /// everything after `--`.
    pub fn parse(args: &[String], spec: &[&'static str]) -> Result<Self, String> {
        let mut a = Self::default();
        let mut it = args.iter();
        let mut rest = false;
        while let Some(arg) = it.next() {
            let looks_numeric = arg.parse::<f64>().is_ok();
            if rest || arg == "-" || !arg.starts_with('-') || looks_numeric {
                a.pos.push(arg.clone());
                continue;
            }
            if arg == "--" {
                rest = true;
                continue;
            }
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
                _ => (arg.as_str(), None),
            };
            let entry = spec
                .iter()
                .find(|s| s.trim_end_matches('=').split('|').any(|n| n == flag))
                .ok_or_else(|| format!("unknown option: {arg}"))?;
            let name = entry
                .trim_end_matches('=')
                .split('|')
                .next()
                .unwrap_or(entry);
            let value = if entry.ends_with('=') {
                Some(
                    inline
                        .or_else(|| it.next().cloned())
                        .ok_or_else(|| format!("{flag} needs a value"))?,
                )
            } else if inline.is_some() {
                return Err(format!("{flag} takes no value"));
            } else {
                None
            };
            a.opts.push((name, value));
        }
        Ok(a)
    }

    pub fn has(&self, name: &str) -> bool {
        self.opts.iter().any(|(n, _)| *n == name)
    }

    /// The last value given for `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.opts
            .iter()
            .rev()
            .find(|(n, _)| *n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    pub fn all(&self, name: &str) -> Vec<&str> {
        self.opts
            .iter()
            .filter(|(n, _)| *n == name)
            .filter_map(|(_, v)| v.as_deref())
            .collect()
    }

    pub fn usize(&self, name: &str) -> Result<Option<usize>, String> {
        self.get(name)
            .map(|v| {
                v.parse::<usize>()
                    .map_err(|_| format!("{name}: not a whole number: {v}"))
            })
            .transpose()
    }

    pub fn out(&self) -> Option<PathBuf> {
        self.get("--out").map(PathBuf::from)
    }

    pub fn json(&self) -> Result<bool, String> {
        match self.get("--format") {
            None | Some("text") => Ok(false),
            Some("json") => Ok(true),
            Some(f) => Err(format!("--format: expected text or json, got {f}")),
        }
    }
}

// -------------------------------------------------------------------- input

/// Where a command's input comes from.
pub enum Source {
    Text(String),
    File(PathBuf),
    Stdin,
}

impl Source {
    /// `--text`, else the positional argument `index` (`-` = stdin), else a
    /// pipe. Waiting on a terminal for typed input would look like a hang.
    pub fn from_args(a: &Args, index: usize) -> Result<Self, String> {
        if let Some(t) = a.get("--text") {
            let t = if a.has("--escapes") {
                crate::search::unescape_extended(t)
            } else {
                t.to_string()
            };
            return Ok(Self::Text(t));
        }
        match a.pos.get(index).map(String::as_str) {
            Some("-") => Ok(Self::Stdin),
            Some(p) => Ok(Self::File(PathBuf::from(p))),
            None if !std::io::stdin().is_terminal() => Ok(Self::Stdin),
            None => Err("give --text, a file, or pipe the input in".into()),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Self::Text(_) => "(text)".into(),
            Self::File(p) => p.display().to_string(),
            Self::Stdin => "-".into(),
        }
    }

    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Text(t) => Ok(t.as_bytes().to_vec()),
            Self::File(p) => std::fs::read(p).map_err(|e| format!("{}: {e}", p.display())),
            Self::Stdin => {
                let mut b = Vec::new();
                std::io::stdin()
                    .read_to_end(&mut b)
                    .map_err(|e| format!("reading stdin: {e}"))?;
                Ok(b)
            }
        }
    }

    /// The input as text, decoded from whatever encoding it's in.
    pub fn text(&self) -> Result<String, String> {
        match self {
            Self::Text(t) => Ok(t.clone()),
            _ => Ok(TextEncoding::detect_and_decode(self.bytes()?).1),
        }
    }
}

// ------------------------------------------------------------------- output

/// Writes the result to `--out` (UTF-8) or stdout.
pub fn emit(a: &Args, text: &str) -> Result<(), String> {
    emit_bytes(a, text.as_bytes())
}

pub fn emit_bytes(a: &Args, bytes: &[u8]) -> Result<(), String> {
    match a.out() {
        Some(p) => std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display())),
        None => {
            let mut so = std::io::stdout().lock();
            so.write_all(bytes)
                .and_then(|()| so.flush())
                .map_err(|e| format!("writing output: {e}"))
        }
    }
}

/// `s` with a line ending added if it has none.
pub fn line(mut s: String) -> String {
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

// -------------------------------------------------------------------- files

/// A file's text and how it's stored.
pub struct TextFile {
    pub encoding: TextEncoding,
    pub eol: Option<Eol>,
    pub text: String,
}

pub fn read_text_file(p: &Path) -> Result<TextFile, String> {
    let bytes = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let (encoding, text) = TextEncoding::detect_and_decode(bytes);
    Ok(TextFile {
        encoding,
        eol: Eol::detect(&text),
        text,
    })
}

/// Saves `text` to `p` through a temporary file, so a failed write never
/// leaves half a file. `backup` keeps the old file as `<name>.bak`.
pub fn write_text_file(
    p: &Path,
    encoding: TextEncoding,
    text: &str,
    backup: bool,
) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", p.display());
    if backup && p.exists() {
        std::fs::copy(p, bak_path(p)).map_err(err)?;
    }
    let name = p
        .file_name()
        .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
    let tmp = p.with_file_name(format!(".{name}.sfn-tmp"));
    std::fs::write(&tmp, encoding.encode(text)).map_err(err)?;
    std::fs::rename(&tmp, p).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        err(e)
    })
}

pub fn bak_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(".bak");
    PathBuf::from(s)
}

pub fn parse_encoding(s: &str) -> Result<TextEncoding, String> {
    let k = s.to_ascii_lowercase().replace(['-', '_', ' '], "");
    Ok(match k.as_str() {
        "utf8" => TextEncoding::Utf8,
        "utf8bom" => TextEncoding::Utf8Bom,
        "utf16" | "utf16le" | "utf16lebom" => TextEncoding::Utf16LeBom,
        "utf16be" | "utf16bebom" => TextEncoding::Utf16BeBom,
        "ansi" | "windows1252" | "cp1252" | "latin1" => TextEncoding::Ansi,
        _ => {
            return Err(format!(
                "unknown encoding {s} (utf-8, utf-8-bom, utf-16le, utf-16be, ansi)"
            ));
        }
    })
}

pub fn parse_eol(s: &str) -> Result<Eol, String> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "crlf" | "windows" | "dos" => Eol::Crlf,
        "lf" | "unix" => Eol::Lf,
        "cr" | "mac" => Eol::Cr,
        _ => return Err(format!("unknown line ending {s} (crlf, lf, cr)")),
    })
}

/// `text` with every line ending made `eol`.
pub fn with_eol(text: &str, eol: Eol) -> String {
    let unix = text.replace("\r\n", "\n").replace('\r', "\n");
    match eol {
        Eol::Lf => unix,
        _ => unix.replace('\n', eol.as_str()),
    }
}

pub fn looks_binary(bytes: &[u8]) -> bool {
    let enc = TextEncoding::detect(&bytes[..bytes.len().min(8000)]);
    !matches!(enc, TextEncoding::Utf16LeBom | TextEncoding::Utf16BeBom)
        && bytes.iter().take(8000).any(|&b| b == 0)
}

/// Settings folder of the app (same rule as `config::config_dir`, which a
/// test there checks).
pub fn app_config_dir() -> PathBuf {
    if let Ok(p) = std::env::var("SUPERFASTNOTEPAD_CONFIG_DIR") {
        return PathBuf::from(p);
    }
    directories::ProjectDirs::from("org", "superfastnotepad", "superfastnotepad").map_or_else(
        || PathBuf::from(".superfastnotepad"),
        |d| d.config_dir().to_path_buf(),
    )
}

/// Refuses to change a file that's open in the app with unsaved edits
/// (from the session the app keeps up to date while it runs), unless
/// `--force`. Writing it anyway would be undone when the edits are saved.
pub fn guard_unsaved(p: &Path, force: bool) -> Result<(), String> {
    if force || !unsaved_in_app(&app_config_dir().join("session.json"), p) {
        return Ok(());
    }
    Err(format!(
        "{} is open in superfastnotepad with unsaved changes; save or close it there, or add --force",
        p.display()
    ))
}

fn unsaved_in_app(session: &Path, p: &Path) -> bool {
    let Ok(s) = std::fs::read_to_string(session) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else {
        return false;
    };
    let Some(target) = p.canonicalize().ok() else {
        return false;
    };
    v["files"].as_array().is_some_and(|files| {
        files.iter().any(|f| {
            f["dirty"].as_bool() == Some(true)
                && f["path"]
                    .as_str()
                    .and_then(|s| Path::new(s).canonicalize().ok())
                    .is_some_and(|q| q == target)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn parses_options_and_positionals() {
        let a = Args::parse(
            &s(&[
                "f.txt",
                "-n",
                "5",
                "--out=o.txt",
                "--follow",
                "--",
                "--x",
                "-3",
            ]),
            &["--lines|-n=", "--out=", "--follow"],
        )
        .unwrap();
        assert_eq!(a.pos, s(&["f.txt", "--x", "-3"]));
        assert_eq!(a.usize("--lines").unwrap(), Some(5));
        assert_eq!(a.get("--out"), Some("o.txt"));
        assert!(a.has("--follow"));
        assert!(Args::parse(&s(&["--bogus"]), &[]).is_err());
        assert!(Args::parse(&s(&["--out"]), &["--out="]).is_err());
        // Negative numbers are values, not options.
        assert_eq!(Args::parse(&s(&["-5"]), &[]).unwrap().pos, s(&["-5"]));
    }

    #[test]
    fn line_endings_and_encodings() {
        assert_eq!(with_eol("a\r\nb\nc\rd", Eol::Crlf), "a\r\nb\r\nc\r\nd");
        assert_eq!(with_eol("a\r\nb", Eol::Lf), "a\nb");
        assert_eq!(
            parse_encoding("UTF-16LE").unwrap(),
            TextEncoding::Utf16LeBom
        );
        assert_eq!(parse_encoding("utf8").unwrap(), TextEncoding::Utf8);
        assert!(parse_encoding("ebcdic").is_err());
        assert_eq!(parse_eol("CRLF").unwrap(), Eol::Crlf);
    }

    #[test]
    fn finds_files_with_unsaved_edits() {
        let dir = std::env::temp_dir().join(format!("sfn-cli-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let other = dir.join("b.txt");
        std::fs::write(&other, "x").unwrap();
        let session = dir.join("session.json");
        let doc = serde_json::json!({"files": [
            {"path": file.display().to_string(), "dirty": true},
            {"path": other.display().to_string(), "dirty": false},
        ]});
        std::fs::write(&session, doc.to_string()).unwrap();
        assert!(unsaved_in_app(&session, &file));
        assert!(!unsaved_in_app(&session, &other));
        assert!(!unsaved_in_app(&dir.join("none.json"), &file));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_command_has_usage() {
        for c in COMMANDS {
            assert!(c.usage.contains(c.name), "{}", c.name);
        }
    }
}
