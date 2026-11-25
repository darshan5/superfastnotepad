//! File commands: tail, head, lines, append, write, replace, info, convert,
//! diff and files.

use std::fmt::Write as _;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::cli::{self, Args, Source};
use crate::cli_search::{FileFilter, cut, list_files, parse_date, path_for_output};
use crate::encoding::{Eol, TextEncoding};
use crate::search::{self, SearchMode, SearchOptions};

// --------------------------------------------------------------- tail/head

pub const TAIL: &str = "\
Usage: tail <file> [-n <lines>] [--follow] [options]

  -n, --lines <n>        how many lines (default 10)
  -f, --follow           keep printing lines as they're added (Ctrl+C stops)
  --until <regex>        with --follow: stop at the first new line that matches
                         (exit 0), e.g. waiting for \"Export finished\"
  --timeout <seconds>    with --follow: stop after this long (exit 1 if --until
                         never matched)
  --grep <regex>         only lines that match; -i ignores case
  --numbers              print line numbers
  --max-line-chars <n>   cut long lines
  --out <file>           write to a file (UTF-8) instead of stdout

A file that's cut shorter while followed (log rotation) is read again from the start.";

pub const HEAD: &str = "\
Usage: head <file> [-n <lines>] [--numbers] [--grep <regex>] [-i] [--max-line-chars <n>] [--out <file>]

  The first lines of a file (default 10).";

const TAIL_SPEC: &[&str] = &[
    "--lines|-n=",
    "--follow|-f",
    "--until=",
    "--timeout=",
    "--grep=",
    "--ignore-case|-i",
    "--numbers",
    "--max-line-chars=",
    "--out=",
];

struct LineOpts {
    filter: Option<fancy_regex::Regex>,
    numbers: bool,
    max_chars: usize,
}

impl LineOpts {
    fn from(a: &Args) -> Result<Self, String> {
        Ok(Self {
            filter: a
                .get("--grep")
                .map(|g| regex(g, a.has("--ignore-case")))
                .transpose()?,
            numbers: a.has("--numbers"),
            max_chars: a.usize("--max-line-chars")?.unwrap_or(0),
        })
    }

    fn keep(&self, l: &str) -> bool {
        self.filter
            .as_ref()
            .is_none_or(|re| re.is_match(l).unwrap_or(false))
    }

    fn show(&self, n: usize, l: &str, width: usize) -> String {
        let l = cut(l, self.max_chars);
        if self.numbers {
            format!("{n:>width$}: {l}\n")
        } else {
            format!("{l}\n")
        }
    }
}

fn regex(pattern: &str, ignore_case: bool) -> Result<fancy_regex::Regex, String> {
    let o = SearchOptions {
        mode: SearchMode::Regex,
        match_case: !ignore_case,
        ..Default::default()
    };
    search::build_regex(pattern, &o).map_err(|e| format!("invalid regular expression: {e}"))
}

fn one_file(a: &Args) -> Result<PathBuf, String> {
    match a.pos.as_slice() {
        [f] => Ok(PathBuf::from(f)),
        [] => Err("give a file".into()),
        _ => Err("give one file".into()),
    }
}

/// Lines of `text` (any line endings), 1-based numbers.
fn numbered(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim_end_matches('\r')))
}

fn width_of(n: usize) -> usize {
    n.max(1).to_string().len()
}

pub fn head(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(args, TAIL_SPEC)?;
    let p = one_file(&a)?;
    let n = a.usize("--lines")?.unwrap_or(10);
    let o = LineOpts::from(&a)?;
    let text = cli::read_text_file(&p)?.text;
    let picked: Vec<(usize, &str)> = numbered(&text).filter(|(_, l)| o.keep(l)).take(n).collect();
    let w = width_of(picked.last().map_or(0, |p| p.0));
    let out: String = picked.iter().map(|(i, l)| o.show(*i, l, w)).collect();
    cli::emit(&a, &out)?;
    Ok(i32::from(picked.is_empty() && o.filter.is_some()))
}

/// Files up to this size are read whole; larger UTF-8/ANSI ones from the end.
const WHOLE_FILE: u64 = 16 * 1024 * 1024;

pub fn tail(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(args, TAIL_SPEC)?;
    let p = one_file(&a)?;
    let n = a.usize("--lines")?.unwrap_or(10);
    let o = LineOpts::from(&a)?;
    let follow = a.has("--follow");
    let until = a
        .get("--until")
        .map(|u| regex(u, a.has("--ignore-case")))
        .transpose()?;
    let timeout = a
        .get("--timeout")
        .map(|t| {
            t.parse::<f64>()
                .ok()
                .filter(|s| *s >= 0.0)
                .map(Duration::from_secs_f64)
                .ok_or_else(|| format!("--timeout: not a number of seconds: {t}"))
        })
        .transpose()?;
    if (until.is_some() || timeout.is_some()) && !follow {
        return Err("--until and --timeout go with --follow".into());
    }

    let mut file = std::fs::File::open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut head = vec![0u8; 4096.min(size as usize)];
    file.read_exact(&mut head).map_err(|e| e.to_string())?;
    let enc = TextEncoding::detect(&head);
    let utf16 = matches!(enc, TextEncoding::Utf16LeBom | TextEncoding::Utf16BeBom);

    // The last `n` (matching) lines, and the number of the first of them.
    let (picked, total) = if size <= WHOLE_FILE || utf16 || o.numbers {
        let mut bytes = Vec::with_capacity(size as usize);
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let text = enc.decode(&bytes);
        let all: Vec<(usize, String)> = numbered(&text)
            .filter(|(_, l)| o.keep(l))
            .map(|(i, l)| (i, l.to_string()))
            .collect();
        let total = text.lines().count();
        (all[all.len().saturating_sub(n)..].to_vec(), total)
    } else {
        (last_lines_from_end(&mut file, size, enc, n, &o)?, 0)
    };
    // Room for the numbers of lines still to come when following.
    let last = total.max(picked.last().map_or(0, |p| p.0));
    let w = width_of(if follow { last + 1000 } else { last });
    let out: String = picked.iter().map(|(i, l)| o.show(*i, l, w)).collect();
    cli::emit(&a, &out)?;
    if !follow {
        return Ok(i32::from(picked.is_empty() && o.filter.is_some()));
    }
    follow_file(&a, &p, size, total, enc, &o, until.as_ref(), timeout, w)
}

/// For a large UTF-8/ANSI file: read blocks from the end until there are
/// enough lines. Line numbers aren't known this way (`--numbers` reads all).
fn last_lines_from_end(
    file: &mut std::fs::File,
    size: u64,
    enc: TextEncoding,
    n: usize,
    o: &LineOpts,
) -> Result<Vec<(usize, String)>, String> {
    const BLOCK: u64 = 1024 * 1024;
    let mut start = size;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let from = start.saturating_sub(BLOCK);
        let mut block = vec![0u8; (start - from) as usize];
        file.seek(SeekFrom::Start(from))
            .map_err(|e| e.to_string())?;
        file.read_exact(&mut block).map_err(|e| e.to_string())?;
        block.extend_from_slice(&buf);
        buf = block;
        start = from;
        // Drop the first (maybe partial) line unless we're at the start.
        let body = if start == 0 {
            &buf[..]
        } else {
            match buf.iter().position(|&b| b == b'\n') {
                Some(i) => &buf[i + 1..],
                None => continue,
            }
        };
        let text = enc.decode(body);
        let lines: Vec<&str> = text
            .lines()
            .map(|l| l.trim_end_matches('\r'))
            .filter(|l| o.keep(l))
            .collect();
        if lines.len() >= n || start == 0 {
            return Ok(lines[lines.len().saturating_sub(n)..]
                .iter()
                .map(|l| (0, (*l).to_string()))
                .collect());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn follow_file(
    a: &Args,
    p: &Path,
    mut pos: u64,
    mut line_no: usize,
    enc: TextEncoding,
    o: &LineOpts,
    until: Option<&fancy_regex::Regex>,
    timeout: Option<Duration>,
    width: usize,
) -> Result<i32, String> {
    let started = Instant::now();
    let mut pending: Vec<u8> = Vec::new();
    let mut out_file = match a.out() {
        Some(path) => Some(
            std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?,
        ),
        None => None,
    };
    loop {
        if timeout.is_some_and(|t| started.elapsed() >= t) {
            return Ok(1);
        }
        std::thread::sleep(Duration::from_millis(250));
        let Ok(meta) = std::fs::metadata(p) else {
            continue; // rotated away for a moment
        };
        let len = meta.len();
        if len < pos {
            eprintln!(
                "tail: {} was cut shorter; reading it from the start",
                p.display()
            );
            pos = 0;
            line_no = 0;
            pending.clear();
        }
        if len == pos {
            continue;
        }
        let mut f = std::fs::File::open(p).map_err(|e| e.to_string())?;
        f.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        let mut chunk = Vec::new();
        std::io::Read::by_ref(&mut f)
            .take(len - pos)
            .read_to_end(&mut chunk)
            .map_err(|e| e.to_string())?;
        pos += chunk.len() as u64;
        pending.extend_from_slice(&chunk);
        // Only whole lines: up to the last line feed in this encoding.
        let cut_at = match enc {
            TextEncoding::Utf16LeBom => (0..pending.len().saturating_sub(1))
                .rev()
                .find(|&i| i % 2 == 0 && pending[i] == b'\n' && pending[i + 1] == 0)
                .map(|i| i + 2),
            TextEncoding::Utf16BeBom => (0..pending.len().saturating_sub(1))
                .rev()
                .find(|&i| i % 2 == 0 && pending[i] == 0 && pending[i + 1] == b'\n')
                .map(|i| i + 2),
            _ => pending.iter().rposition(|&b| b == b'\n').map(|i| i + 1),
        };
        let Some(cut_at) = cut_at else { continue };
        let whole: Vec<u8> = pending.drain(..cut_at).collect();
        let text = enc.decode(&whole);
        let mut out = String::new();
        let mut matched = false;
        for l in text.lines() {
            line_no += 1;
            let l = l.trim_end_matches('\r');
            if o.keep(l) {
                out.push_str(&o.show(line_no, l, width));
            }
            if until.is_some_and(|re| re.is_match(l).unwrap_or(false)) {
                matched = true;
                break;
            }
        }
        if !out.is_empty() {
            match &mut out_file {
                Some(f) => f.write_all(out.as_bytes()).map_err(|e| e.to_string())?,
                None => {
                    let mut so = std::io::stdout().lock();
                    so.write_all(out.as_bytes()).map_err(|e| e.to_string())?;
                    so.flush().map_err(|e| e.to_string())?;
                }
            }
        }
        if matched {
            return Ok(0);
        }
    }
}

// -------------------------------------------------------------------- lines

pub const LINES: &str = "\
Usage: lines <file> (<from>-<to> | --from <n> [--to <n> | --count <n>] | --around <n> [--context <n>])

  12-40                  lines 12 to 40; 120- is line 120 to the end, -40 is
                         line 1 to 40
  --from <n>             first line (default 1, also when only --to is given)
  --to <n>               last line (or `end`)
  --count <n>            how many lines

Without an end (no --to, --count or `-<n>`), 50 lines are printed, and a note
on stderr says how many lines the file has and which range comes next.
  --around <n>           the lines around line n (a `search` hit), with
  --context <n>          this many lines before and after (default 10)
  --no-numbers           leave out the line numbers
  --max-line-chars <n>   cut long lines
  --out <file>           write to a file (UTF-8) instead of stdout";

pub fn lines(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(
        args,
        &[
            "--from=",
            "--to=",
            "--count=",
            "--around=",
            "--context=",
            "--no-numbers",
            "--max-line-chars=",
            "--out=",
        ],
    )?;
    let (file, range) = match a.pos.as_slice() {
        [f] => (f, None),
        [f, r] => (f, Some(r.as_str())),
        [] => return Err("give a file".into()),
        _ => return Err("give one file and at most one range".into()),
    };
    const DEFAULT_COUNT: usize = 50;
    // `to` = usize::MAX means the end of the file; `implicit` that no end was given.
    let mut implicit = false;
    let (from, to) = if let Some(r) = range {
        let (x, y) = r
            .split_once('-')
            .ok_or_else(|| format!("expected a range like 12-40, 120- or -40, got {r}"))?;
        let num = |s: &str| {
            s.trim()
                .parse::<usize>()
                .map_err(|_| format!("expected a range like 12-40, 120- or -40, got {r}"))
        };
        let to = if y.trim().is_empty() {
            usize::MAX
        } else {
            num(y)?
        };
        let from = if x.trim().is_empty() { 1 } else { num(x)? };
        (from.max(1), to)
    } else if let Some(c) = a.usize("--around")? {
        let ctx = a.usize("--context")?.unwrap_or(10);
        (c.saturating_sub(ctx).max(1), c + ctx)
    } else {
        let from = a.usize("--from")?.unwrap_or(1).max(1);
        let to = match (a.get("--to"), a.usize("--count")?) {
            (Some("end"), _) => usize::MAX,
            (Some(_), _) => a.usize("--to")?.unwrap_or(from),
            (None, Some(c)) => from + c.max(1) - 1,
            (None, None) => {
                implicit = true;
                from + DEFAULT_COUNT - 1
            }
        };
        (from, to)
    };
    if to < from {
        return Err(format!("the range ends ({to}) before it starts ({from})"));
    }
    let o = LineOpts {
        filter: None,
        numbers: !a.has("--no-numbers"),
        max_chars: a.usize("--max-line-chars")?.unwrap_or(0),
    };
    let text = cli::read_text_file(Path::new(file))?.text;
    let total = text.lines().count();
    let w = width_of(to.min(total));
    let mut out: String = numbered(&text)
        .skip(from - 1)
        .take(to.saturating_sub(from).saturating_add(1))
        .map(|(i, l)| o.show(i, l, w))
        .collect();
    if from > total {
        let _ = writeln!(out, "(the file has {total} lines)");
        cli::emit(&a, &out)?;
        return Ok(1);
    }
    cli::emit(&a, &out)?;
    if implicit && to < total {
        let next = to + 1;
        eprintln!(
            "(lines {from}-{to} of {total}; next: {next}-{}, or {next}- for the rest)",
            (next + DEFAULT_COUNT - 1).min(total)
        );
    }
    Ok(0)
}

// ------------------------------------------------------------- append/write

pub const APPEND: &str = "\
Usage: append <file> (--text <s> | --from <file> | -) [options]

  --text <s>             the text to add (--escapes turns \\n \\t \\\\ into
                         a new line, a tab, a backslash)
  --from <file>          add the contents of this file
  -                      add what's piped in
  --no-newline           don't end the added text with a line ending
  --encoding <e>         for a new file: utf-8 (default), utf-8-bom, utf-16le,
                         utf-16be, ansi
  --eol <crlf|lf|cr>     for a new file (default: this system's)
  --force                even if the file has unsaved changes in the app

An existing file keeps its encoding and line endings; the added text gets the
file's line endings, and starts on a new line.";

pub const WRITE: &str = "\
Usage: write <file> (--text <s> | --from <file> | -) [options]

  --force                replace an existing file (and ignore unsaved changes
                         to it in the app)
  --backup               keep the old file as <file>.bak
  --encoding <e>         utf-8 (default for a new file), utf-8-bom, utf-16le,
                         utf-16be, ansi; an existing file keeps its own
  --eol <crlf|lf|cr>     line endings (default: the existing file's, or this
                         system's)
  --escapes              in --text, \\n \\t \\\\ are a new line, a tab, a backslash
  --no-newline           don't add a final line ending";

const WRITE_SPEC: &[&str] = &[
    "--text=",
    "--from=",
    "--escapes",
    "--no-newline",
    "--encoding=",
    "--eol=",
    "--force",
    "--backup",
    "--out=",
];

/// The text to add or write: `--text`, `--from`, or stdin (`-` or a pipe).
fn content(a: &Args) -> Result<String, String> {
    if let Some(f) = a.get("--from") {
        return Source::File(PathBuf::from(f)).text();
    }
    if a.get("--text").is_some() {
        return Source::from_args(a, usize::MAX)?.text();
    }
    match a.pos.get(1).map(String::as_str) {
        Some("-") | None => Source::from_args(a, 1)?.text(),
        Some(x) => Err(format!("unexpected argument: {x}")),
    }
}

fn target(a: &Args) -> Result<PathBuf, String> {
    a.pos
        .first()
        .map(PathBuf::from)
        .ok_or_else(|| "give the file to write".into())
}

fn new_file_format(a: &Args) -> Result<(TextEncoding, Eol), String> {
    Ok((
        a.get("--encoding")
            .map_or(Ok(TextEncoding::Utf8), cli::parse_encoding)?,
        a.get("--eol")
            .map_or(Ok(Eol::platform_default()), cli::parse_eol)?,
    ))
}

pub fn append(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(args, WRITE_SPEC)?;
    let p = target(&a)?;
    let text = content(&a)?;
    let (new_enc, new_eol) = new_file_format(&a)?;
    let existing = if p.exists() {
        cli::guard_unsaved(&p, a.has("--force"))?;
        Some(cli::read_text_file(&p)?)
    } else {
        None
    };
    let eol = existing.as_ref().and_then(|f| f.eol).unwrap_or(new_eol);
    let mut add = cli::with_eol(&text, eol);
    if !a.has("--no-newline") && !add.is_empty() && !add.ends_with(['\n', '\r']) {
        add.push_str(eol.as_str());
    }
    let added_lines = add.lines().count();
    match existing {
        Some(f) => {
            if !f.text.is_empty() && !f.text.ends_with(['\n', '\r']) {
                add.insert_str(0, eol.as_str());
            }
            // Encoded without the byte order mark the file already has.
            let mut bytes = f.encoding.encode(&add);
            let bom = match f.encoding {
                TextEncoding::Utf8Bom => 3,
                TextEncoding::Utf16LeBom | TextEncoding::Utf16BeBom => 2,
                _ => 0,
            };
            bytes.drain(..bom.min(bytes.len()));
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&p)
                .map_err(|e| format!("{}: {e}", p.display()))?;
            file.write_all(&bytes)
                .map_err(|e| format!("{}: {e}", p.display()))?;
        }
        None => cli::write_text_file(&p, new_enc, &add, false)?,
    }
    report(
        &a,
        &format!("appended {added_lines} line(s) to {}", p.display()),
    )?;
    Ok(0)
}

pub fn write(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(args, WRITE_SPEC)?;
    let p = target(&a)?;
    let text = content(&a)?;
    let (mut enc, mut eol) = new_file_format(&a)?;
    if p.exists() {
        if !a.has("--force") {
            return Err(format!("{} exists; add --force to replace it", p.display()));
        }
        cli::guard_unsaved(&p, true)?;
        let old = cli::read_text_file(&p)?;
        if a.get("--encoding").is_none() {
            enc = old.encoding;
        }
        if a.get("--eol").is_none() {
            eol = old.eol.unwrap_or(eol);
        }
    }
    let mut body = cli::with_eol(&text, eol);
    if !a.has("--no-newline") && !body.is_empty() && !body.ends_with(['\n', '\r']) {
        body.push_str(eol.as_str());
    }
    cli::write_text_file(&p, enc, &body, a.has("--backup"))?;
    report(
        &a,
        &format!(
            "wrote {} line(s) to {} ({}, {})",
            body.lines().count(),
            p.display(),
            enc.label(),
            eol.label()
        ),
    )?;
    Ok(0)
}

/// A one-line confirmation, to `--out` or stdout.
fn report(a: &Args, msg: &str) -> Result<(), String> {
    cli::emit(a, &format!("{msg}\n"))
}

// ------------------------------------------------------------------ replace

pub const REPLACE: &str = "\
Usage: replace (<file>... | --dir <folder> [--glob <p>] [-r]) (--text <s> | --regex <re>) --with <s> [options]

  --text <s> / --regex <re>  what to find; in --with, $1 / ${name} are groups
  --with <s>             the replacement (may be empty: --with \"\")
  -i, --ignore-case      ignore upper/lower case
  --word                 whole words only
  --escapes              \\n \\t \\\\ in --text and --with
  --glob <pattern>       with --dir: only files whose name matches; repeatable
  -r, --recursive        with --dir: also sub-folders
  --dry-run              show what would change (a unified diff), change nothing
  --backup               keep each changed file as <file>.bak
  --force                also files with unsaved changes in the app
  --out <file>           write the report to a file

Files keep their encoding and line endings. Exit code 0 = replaced (or would),
1 = nothing matched, 2 = error.";

pub fn replace(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(
        args,
        &[
            "--text=",
            "--regex=",
            "--with=",
            "--ignore-case|-i",
            "--word",
            "--escapes",
            "--dir=",
            "--glob=",
            "--recursive|-r",
            "--dry-run",
            "--backup",
            "--force",
            "--out=",
        ],
    )?;
    let (pattern, mode) = match (a.get("--text"), a.get("--regex")) {
        (Some(t), None) if a.has("--escapes") => (t, SearchMode::Extended),
        (Some(t), None) => (t, SearchMode::Normal),
        (None, Some(r)) => (r, SearchMode::Regex),
        _ => return Err("give --text or --regex (one of them)".into()),
    };
    if pattern.is_empty() {
        return Err("the text to find is empty".into());
    }
    let with = a
        .get("--with")
        .ok_or("give --with (the replacement; --with \"\" deletes)")?;
    let opts = SearchOptions {
        mode,
        match_case: !a.has("--ignore-case"),
        whole_word: a.has("--word"),
        ..Default::default()
    };
    let re = search::build_regex(pattern, &opts)
        .map_err(|e| format!("invalid regular expression: {e}"))?;
    let globs: Vec<String> = a.all("--glob").iter().map(|s| (*s).to_string()).collect();
    let out_path = a.out();
    let files: Vec<PathBuf> = match a.get("--dir") {
        Some(d) => list_files(&FileFilter {
            dir: Path::new(d),
            globs: &globs,
            recursive: a.has("--recursive"),
            newest: None,
            modified_since: None,
            skip: out_path.as_deref(),
        })?
        .into_iter()
        .map(|f| f.0)
        .collect(),
        None if a.pos.is_empty() => return Err("give files, or --dir".into()),
        None => a.pos.iter().map(PathBuf::from).collect(),
    };
    let dry = a.has("--dry-run");
    let mut report = String::new();
    let (mut total, mut changed_files, mut searched) = (0, 0, 0);
    for p in &files {
        let Ok(bytes) = std::fs::read(p) else {
            let _ = writeln!(report, "{}: can't read, skipped", path_for_output(p));
            continue;
        };
        if cli::looks_binary(&bytes) {
            continue;
        }
        searched += 1;
        let (enc, text) = TextEncoding::detect_and_decode(bytes);
        let (new, n) = search::replace_all_in(&re, &text, with, mode);
        if n == 0 {
            continue;
        }
        let shown = path_for_output(p);
        if dry {
            let diff = unified(&shown, &shown, &text, &new, 0, |s| s.to_string());
            report.push_str(&diff);
        } else {
            if let Err(e) = cli::guard_unsaved(p, a.has("--force")) {
                let _ = writeln!(report, "{shown}: skipped: {e}");
                continue;
            }
            cli::write_text_file(p, enc, &new, a.has("--backup"))?;
        }
        let _ = writeln!(report, "{shown}: {n} replacement(s)");
        total += n;
        changed_files += 1;
    }
    let _ = writeln!(
        report,
        "{total} replacement(s) in {changed_files} of {searched} file(s){}",
        if dry {
            " (dry run: nothing changed)"
        } else {
            ""
        }
    );
    cli::emit(&a, &report)?;
    Ok(i32::from(total == 0))
}

// --------------------------------------------------------------------- info

pub const INFO: &str = "\
Usage: info <file>... [--format text|json] [--out <file>]

  Size, modified time, encoding, line endings, line count, longest line and
  whether the file ends with a line ending.";

pub fn info(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(args, &["--format=", "--out="])?;
    if a.pos.is_empty() {
        return Err("give one or more files".into());
    }
    let json = a.json()?;
    let mut docs = Vec::new();
    let mut text_out = String::new();
    let mut failed = false;
    for f in &a.pos {
        let p = Path::new(f);
        let (meta, bytes) = match (std::fs::metadata(p), std::fs::read(p)) {
            (Ok(m), Ok(b)) => (m, b),
            (Err(e), _) | (_, Err(e)) => {
                failed = true;
                let _ = writeln!(text_out, "{f}: {e}");
                docs.push(serde_json::json!({"path": f, "error": e.to_string()}));
                continue;
            }
        };
        let modified = meta.modified().ok().map(|t| {
            chrono::DateTime::<chrono::Local>::from(t)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
        });
        if cli::looks_binary(&bytes) {
            let _ = writeln!(
                text_out,
                "{f}\n  size:       {}\n  modified:   {}\n  binary file\n",
                size_label(meta.len()),
                modified.clone().unwrap_or_default()
            );
            docs.push(serde_json::json!({"path": f, "bytes": meta.len(), "modified": modified, "binary": true}));
            continue;
        }
        let (enc, text) = TextEncoding::detect_and_decode(bytes);
        let [crlf, lf, cr] = Eol::count(&text);
        let kinds = [crlf, lf, cr].iter().filter(|&&n| n > 0).count();
        let eol = match (kinds, Eol::detect(&text)) {
            (0, _) | (_, None) => "none (one line)".to_string(),
            (1, Some(e)) => e.label().to_string(),
            _ => format!("Mixed (CR LF {crlf}, LF {lf}, CR {cr})"),
        };
        let lines = text.lines().count();
        let longest = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
        let final_newline = text.ends_with(['\n', '\r']);
        let _ = writeln!(
            text_out,
            "{f}\n  size:       {}\n  modified:   {}\n  encoding:   {}\n  line ends:  {eol}\n  lines:      {lines}\n  longest:    {longest} characters\n  ends with a line ending: {}\n",
            size_label(meta.len()),
            modified.clone().unwrap_or_default(),
            enc.label(),
            if final_newline { "yes" } else { "no" }
        );
        docs.push(serde_json::json!({
            "path": f, "bytes": meta.len(), "modified": modified, "binary": false,
            "encoding": enc.label(), "line_endings": eol,
            "crlf": crlf, "lf": lf, "cr": cr, "lines": lines,
            "longest_line": longest, "final_newline": final_newline,
        }));
    }
    if json {
        cli::emit(
            &a,
            &cli::line(serde_json::to_string_pretty(&docs).unwrap_or_default()),
        )?;
    } else {
        cli::emit(&a, text_out.trim_end_matches('\n'))?;
        if a.out().is_none() {
            println!();
        }
    }
    Ok(if failed { 2 } else { 0 })
}

fn size_label(n: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let f = n as f64;
    if n < 1024 {
        format!("{n} bytes")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB ({n} bytes)", f / 1024.0)
    } else if n < 1024 * 1024 * 1024 {
        format!("{:.1} MB ({n} bytes)", f / 1024.0 / 1024.0)
    } else {
        format!("{:.2} GB ({n} bytes)", f / 1024.0 / 1024.0 / 1024.0)
    }
}

// ------------------------------------------------------------------ convert

pub const CONVERT: &str = "\
Usage: convert <file> [--encoding <e>] [--eol <crlf|lf|cr>] [--out <file>] [--backup] [--force]

  --encoding <e>         utf-8, utf-8-bom, utf-16le, utf-16be, ansi
  --eol <crlf|lf|cr>     make every line ending this
  --out <file>           write the converted file there (default: in place)
  --backup               when converting in place, keep <file>.bak
  --force                even if the file has unsaved changes in the app

Example: convert export.csv --encoding utf-8 --eol crlf   (fixes a UTF-16 file
written by PowerShell's `>`).";

pub fn convert(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(
        args,
        &["--encoding=", "--eol=", "--out=", "--backup", "--force"],
    )?;
    let p = one_file(&a)?;
    if a.get("--encoding").is_none() && a.get("--eol").is_none() {
        return Err("give --encoding, --eol or both".into());
    }
    let f = cli::read_text_file(&p)?;
    let enc = a
        .get("--encoding")
        .map_or(Ok(f.encoding), cli::parse_encoding)?;
    let text = match a.get("--eol") {
        Some(e) => cli::with_eol(&f.text, cli::parse_eol(e)?),
        None => f.text.clone(),
    };
    let eol_label = |t: &str| Eol::detect(t).map_or("no line endings", Eol::label);
    let dest = a.out().unwrap_or_else(|| p.clone());
    if dest == p {
        cli::guard_unsaved(&p, a.has("--force"))?;
    }
    cli::write_text_file(&dest, enc, &text, a.has("--backup") && dest == p)?;
    // The report goes to stdout: --out is the converted file here.
    println!(
        "{}: {}, {} -> {}: {}, {}",
        p.display(),
        f.encoding.label(),
        eol_label(&f.text),
        dest.display(),
        enc.label(),
        eol_label(&text)
    );
    Ok(0)
}

// --------------------------------------------------------------------- diff

pub const DIFF: &str = "\
Usage: diff <old file> <new file> [options]

  --context <n>          unchanged lines shown around each change (default 3)
  -i, --ignore-case      treat upper and lower case as equal
  -w, --ignore-whitespace  ignore spaces and tabs inside lines
  --ignore-eol           treat CR LF and LF files as equal (always on: lines
                         are compared without their line endings)
  --out <file>           write the diff to a file

Exit code (like diff): 0 = the same, 1 = different, 2 = error.";

pub fn diff(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(
        args,
        &[
            "--context=",
            "--ignore-case|-i",
            "--ignore-whitespace|-w",
            "--ignore-eol",
            "--out=",
        ],
    )?;
    let [old, new] = a.pos.as_slice() else {
        return Err("give two files".into());
    };
    let ctx = a.usize("--context")?.unwrap_or(3);
    let ic = a.has("--ignore-case");
    let iw = a.has("--ignore-whitespace");
    let l = cli::read_text_file(Path::new(old))?.text;
    let r = cli::read_text_file(Path::new(new))?.text;
    let norm = move |s: &str| {
        let s = if iw {
            s.split_whitespace().collect::<Vec<_>>().join(" ")
        } else {
            s.to_string()
        };
        if ic { s.to_lowercase() } else { s }
    };
    let out = unified(old, new, &l, &r, ctx, norm);
    if out.is_empty() {
        cli::emit(&a, "")?;
        return Ok(0);
    }
    cli::emit(&a, &out)?;
    Ok(1)
}

/// A unified diff of `old` and `new` (lines compared after `norm`, shown
/// as they are); empty when they're the same.
fn unified(
    old_name: &str,
    new_name: &str,
    old: &str,
    new: &str,
    context: usize,
    norm: impl Fn(&str) -> String,
) -> String {
    let a: Vec<&str> = old.lines().map(|l| l.trim_end_matches('\r')).collect();
    let b: Vec<&str> = new.lines().map(|l| l.trim_end_matches('\r')).collect();
    let na: Vec<String> = a.iter().map(|l| norm(l)).collect();
    let nb: Vec<String> = b.iter().map(|l| norm(l)).collect();
    let ops = similar::capture_diff_slices(similar::Algorithm::Myers, &na, &nb);
    let groups = similar::group_diff_ops(ops, context);
    if groups.is_empty() {
        return String::new();
    }
    let mut out = format!("--- {old_name}\n+++ {new_name}\n");
    for g in groups {
        let (Some(first), Some(last)) = (g.first(), g.last()) else {
            continue;
        };
        let (os, ns) = (first.old_range().start, first.new_range().start);
        let (oe, ne) = (last.old_range().end, last.new_range().end);
        let span = |s: usize, e: usize| {
            if e - s == 1 {
                format!("{}", s + 1)
            } else {
                format!("{},{}", if e == s { s } else { s + 1 }, e - s)
            }
        };
        let _ = writeln!(out, "@@ -{} +{} @@", span(os, oe), span(ns, ne));
        for op in &g {
            let (tag, or, nr) = op.as_tag_tuple();
            match tag {
                similar::DiffTag::Equal => {
                    for i in or {
                        let _ = writeln!(out, " {}", a[i]);
                    }
                }
                similar::DiffTag::Delete => {
                    for i in or {
                        let _ = writeln!(out, "-{}", a[i]);
                    }
                }
                similar::DiffTag::Insert => {
                    for i in nr {
                        let _ = writeln!(out, "+{}", b[i]);
                    }
                }
                similar::DiffTag::Replace => {
                    for i in or {
                        let _ = writeln!(out, "-{}", a[i]);
                    }
                    for i in nr {
                        let _ = writeln!(out, "+{}", b[i]);
                    }
                }
            }
        }
    }
    out
}

// -------------------------------------------------------------------- files

pub const FILES: &str = "\
Usage: files [--dir <folder>] [--glob <pattern>]... [options]

  --dir <folder>         where to look (default: current folder)
  --glob <pattern>       only names that match (* and ?); repeatable
  -r, --recursive        also sub-folders
  --newest <n>           only the n most recently modified
  --modified-since <d>   only files modified on/after YYYY-MM-DD
  --format text|json     output format
  --out <file>           write the list to a file

Lists `modified  size  path`, newest first. Exit code 1 = no files.";

pub fn files(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(
        args,
        &[
            "--dir=",
            "--glob=",
            "--recursive|-r",
            "--newest=",
            "--modified-since=",
            "--format=",
            "--out=",
        ],
    )?;
    let json = a.json()?;
    let globs: Vec<String> = a.all("--glob").iter().map(|s| (*s).to_string()).collect();
    let dir = PathBuf::from(a.get("--dir").unwrap_or("."));
    let out_path = a.out();
    let mut list = list_files(&FileFilter {
        dir: &dir,
        globs: &globs,
        recursive: a.has("--recursive"),
        newest: a.usize("--newest")?,
        modified_since: a.get("--modified-since").map(parse_date).transpose()?,
        skip: out_path.as_deref(),
    })?;
    list.sort_by_key(|f| std::cmp::Reverse(f.1));
    let when = |t: std::time::SystemTime| {
        chrono::DateTime::<chrono::Local>::from(t)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    };
    let out = if json {
        let v: Vec<serde_json::Value> = list
            .iter()
            .map(|(p, t, s)| {
                serde_json::json!({"path": path_for_output(p), "modified": when(*t), "bytes": s})
            })
            .collect();
        cli::line(serde_json::to_string_pretty(&v).unwrap_or_default())
    } else {
        let mut s = String::new();
        for (p, t, size) in &list {
            let _ = writeln!(s, "{}  {:>10}  {}", when(*t), size, path_for_output(p));
        }
        let _ = writeln!(s, "{} file(s)", list.len());
        s
    };
    cli::emit(&a, &out)?;
    Ok(i32::from(list.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sfn-cli-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn run(f: fn(&[String]) -> Result<i32, String>, args: &[&str]) -> Result<i32, String> {
        f(&args.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
    }

    fn s(p: &Path) -> String {
        p.display().to_string()
    }

    #[test]
    fn head_tail_and_lines() {
        let d = dir("ht");
        let f = d.join("log.txt");
        let body: String = (1..=30).map(|i| format!("line {i}\r\n")).collect();
        std::fs::write(&f, body).unwrap();
        let out = d.join("out.txt");
        let read = || std::fs::read_to_string(&out).unwrap();
        run(tail, &[&s(&f), "-n", "3", "--out", &s(&out)]).unwrap();
        assert_eq!(read(), "line 28\nline 29\nline 30\n");
        run(tail, &[&s(&f), "-n", "2", "--numbers", "--out", &s(&out)]).unwrap();
        assert!(read().ends_with("30: line 30\n"), "{}", read());
        run(head, &[&s(&f), "-n", "2", "--out", &s(&out)]).unwrap();
        assert_eq!(read(), "line 1\nline 2\n");
        assert_eq!(
            run(head, &[&s(&f), "--grep", "^line 2[05]$", "--out", &s(&out)]),
            Ok(0)
        );
        assert_eq!(read(), "line 20\nline 25\n");
        run(lines, &[&s(&f), "9-11", "--out", &s(&out)]).unwrap();
        assert_eq!(read(), " 9: line 9\n10: line 10\n11: line 11\n");
        run(
            lines,
            &[&s(&f), "--around", "5", "--context", "1", "--out", &s(&out)],
        )
        .unwrap();
        assert_eq!(read(), "4: line 4\n5: line 5\n6: line 6\n");
        assert_eq!(run(lines, &[&s(&f), "40-45", "--out", &s(&out)]), Ok(1));
        run(lines, &[&s(&f), "29-", "--out", &s(&out)]).unwrap();
        assert_eq!(read(), "29: line 29\n30: line 30\n");
        run(
            lines,
            &[&s(&f), "--from", "28", "--to", "end", "--out", &s(&out)],
        )
        .unwrap();
        assert_eq!(read().lines().count(), 3);
        run(lines, &[&s(&f), "-2", "--out", &s(&out)]).unwrap();
        assert_eq!(read(), "1: line 1\n2: line 2\n");
        run(lines, &[&s(&f), "--to", "2", "--out", &s(&out)]).unwrap();
        assert_eq!(read(), "1: line 1\n2: line 2\n");
        run(lines, &[&s(&f), "--from", "3", "--out", &s(&out)]).unwrap();
        assert_eq!(read().lines().count(), 28);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tail_reads_large_files_from_the_end() {
        let d = dir("big");
        let f = d.join("big.log");
        let mut body = String::new();
        let mut i = 0;
        while body.len() < (WHOLE_FILE as usize) + 100_000 {
            i += 1;
            let _ = writeln!(body, "entry {i} {}", "x".repeat(60));
        }
        std::fs::write(&f, &body).unwrap();
        let out = d.join("out.txt");
        run(tail, &[&s(&f), "-n", "2", "--out", &s(&out)]).unwrap();
        let got = std::fs::read_to_string(&out).unwrap();
        assert!(got.starts_with(&format!("entry {} ", i - 1)), "{got}");
        assert_eq!(got.lines().count(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn follow_stops_at_until() {
        let d = dir("follow");
        let f = d.join("app.log");
        std::fs::write(&f, "start\n").unwrap();
        let out = d.join("out.txt");
        let writer = {
            let f = f.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(400));
                let mut h = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
                h.write_all(b"working\nExport finished\nafter\n").unwrap();
            })
        };
        let code = run(
            tail,
            &[
                &s(&f),
                "-f",
                "--until",
                "finished",
                "--timeout",
                "5",
                "--out",
                &s(&out),
            ],
        );
        writer.join().unwrap();
        assert_eq!(code, Ok(0));
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "start\nworking\nExport finished\n"
        );
        assert_eq!(
            run(
                tail,
                &[
                    &s(&f),
                    "-f",
                    "--until",
                    "never",
                    "--timeout",
                    "0.3",
                    "--out",
                    &s(&out)
                ]
            ),
            Ok(1)
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn append_keeps_encoding_and_line_endings() {
        let d = dir("append");
        let f = d.join("notes.txt");
        // UTF-16 LE with CR LF and no final line ending.
        std::fs::write(&f, TextEncoding::Utf16LeBom.encode("one\r\ntwo")).unwrap();
        let out = d.join("out.txt");
        run(
            append,
            &[
                &s(&f),
                "--text",
                "three\\nfour",
                "--escapes",
                "--out",
                &s(&out),
            ],
        )
        .unwrap();
        let back = cli::read_text_file(&f).unwrap();
        assert_eq!(back.encoding, TextEncoding::Utf16LeBom);
        assert_eq!(back.text, "one\r\ntwo\r\nthree\r\nfour\r\n");
        // A new file.
        let g = d.join("new.txt");
        run(
            append,
            &[&s(&g), "--text", "hi", "--eol", "lf", "--out", &s(&out)],
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&g).unwrap(), "hi\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn write_needs_force_to_replace() {
        let d = dir("write");
        let f = d.join("a.txt");
        let out = d.join("out.txt");
        run(
            write,
            &[&s(&f), "--text", "x", "--eol", "crlf", "--out", &s(&out)],
        )
        .unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"x\r\n");
        assert!(run(write, &[&s(&f), "--text", "y", "--out", &s(&out)]).is_err());
        run(
            write,
            &[
                &s(&f),
                "--text",
                "y\nz",
                "--force",
                "--backup",
                "--out",
                &s(&out),
            ],
        )
        .unwrap();
        // The existing file's line endings are kept.
        assert_eq!(std::fs::read(&f).unwrap(), b"y\r\nz\r\n");
        assert_eq!(std::fs::read(cli::bak_path(&f)).unwrap(), b"x\r\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn replace_dry_run_and_real() {
        let d = dir("replace");
        let a = d.join("a.txt");
        std::fs::write(&a, "id=1\r\nid=22\r\nname=x\r\n").unwrap();
        let b = d.join("b.txt");
        std::fs::write(&b, "nothing here\n").unwrap();
        let out = d.join("report.txt");
        let args = [
            "--dir",
            &s(&d),
            "--glob",
            "?.txt",
            "--regex",
            r"id=(\d+)",
            "--with",
            "ID-$1",
        ];
        let o = s(&out);
        let mut dry: Vec<&str> = args.to_vec();
        dry.extend(["--dry-run", "--out", o.as_str()]);
        assert_eq!(run(replace, &dry), Ok(0));
        let rep = std::fs::read_to_string(&out).unwrap();
        assert!(rep.contains("-id=1\n-id=22\n+ID-1\n+ID-22\n"), "{rep}");
        assert!(
            rep.contains("2 replacement(s) in 1 of 2 file(s) (dry run"),
            "{rep}"
        );
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "id=1\r\nid=22\r\nname=x\r\n"
        );
        let mut real: Vec<&str> = args.to_vec();
        real.extend(["--out", o.as_str()]);
        assert_eq!(run(replace, &real), Ok(0));
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "ID-1\r\nID-22\r\nname=x\r\n"
        );
        assert_eq!(run(replace, &real), Ok(1));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn convert_info_and_diff() {
        let d = dir("conv");
        let f = d.join("ps.txt");
        std::fs::write(&f, TextEncoding::Utf16LeBom.encode("a\r\nb\r\n")).unwrap();
        run(convert, &[&s(&f), "--encoding", "utf-8", "--eol", "lf"]).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"a\nb\n");
        let out = d.join("out.txt");
        run(info, &[&s(&f), "--format", "json", "--out", &s(&out)]).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(v[0]["lines"], 2);
        assert_eq!(v[0]["encoding"], "UTF-8");
        let g = d.join("g.txt");
        std::fs::write(&g, "a\r\nB\r\n").unwrap();
        assert_eq!(run(diff, &[&s(&f), &s(&g), "--out", &s(&out)]), Ok(1));
        assert!(std::fs::read_to_string(&out).unwrap().contains("-b\n+B\n"));
        assert_eq!(run(diff, &[&s(&f), &s(&g), "-i", "--out", &s(&out)]), Ok(0));
        let _ = std::fs::remove_dir_all(&d);
    }
}
