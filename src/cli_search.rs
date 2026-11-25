//! `superfastnotepad search ...`: a headless, grep-style search over a
//! folder that writes plain text (or JSON) and exits. It runs before any
//! settings, session or window code, so it opens no window and leaves a
//! running instance alone.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::search::{self, SearchMode, SearchOptions};

pub const USAGE: &str = "\
Usage: superfastnotepad-cli search --dir <folder> (--text <s> | --regex <re>) [options]
       (or `superfastnotepad search ...`: the same search from the app's own
       program, which on Windows doesn't wait or give an exit code in a shell)

  --dir <folder>          folder to search (default: current folder)
  --text <s>              find this literal text
  --regex <re>            find this regular expression
  --ignore-case, -i       ignore upper/lower case
  --glob <pattern>        only files whose name matches (* and ?); repeatable
  --recursive, -r         also search sub-folders
  --newest <n>            only the n most recently modified matching files
  --modified-since <date> only files modified on/after YYYY-MM-DD
  --context <n>           also print n lines before and after each hit
  --max-line-chars <n>    cut printed lines at n characters (0 = full)
  --decode-entities       print &quot; &amp; &#34; ... as the real characters
  --fields                print each hit as `column: value` rows, using the
                          file's first line as the CSV header
  --format text|json      output format (default: text)
  --out <file>            write results to a file instead of stdout

Output: `path:line: text`, one hit per line, then `N hits in M files of K searched`.
Exit code: 0 = hits, 1 = no hits, 2 = error.";

#[derive(Debug, Default)]
struct Opts {
    dir: PathBuf,
    pattern: Option<String>,
    regex: bool,
    ignore_case: bool,
    globs: Vec<String>,
    recursive: bool,
    newest: Option<usize>,
    modified_since: Option<SystemTime>,
    context: usize,
    max_line_chars: usize,
    decode_entities: bool,
    fields: bool,
    json: bool,
    out: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        dir: PathBuf::from("."),
        ..Default::default()
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        // Accept both `--flag value` and `--flag=value`.
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (a.as_str(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| it.next().cloned())
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        let number = |v: String| {
            v.parse::<usize>()
                .map_err(|_| format!("{flag}: not a number: {v}"))
        };
        match flag {
            "--dir" => o.dir = PathBuf::from(value()?),
            "--text" => {
                o.pattern = Some(value()?);
                o.regex = false;
            }
            "--regex" => {
                o.pattern = Some(value()?);
                o.regex = true;
            }
            "--ignore-case" | "-i" => o.ignore_case = true,
            "--glob" => o.globs.push(value()?),
            "--recursive" | "-r" => o.recursive = true,
            "--newest" => o.newest = Some(number(value()?)?),
            "--modified-since" => o.modified_since = Some(parse_date(&value()?)?),
            "--context" => o.context = number(value()?)?,
            "--max-line-chars" => o.max_line_chars = number(value()?)?,
            "--decode-entities" => o.decode_entities = true,
            "--fields" => o.fields = true,
            "--format" => match value()?.as_str() {
                "text" => o.json = false,
                "json" => o.json = true,
                f => return Err(format!("--format: expected text or json, got {f}")),
            },
            "--out" => o.out = Some(PathBuf::from(value()?)),
            _ => return Err(format!("unknown option: {a}")),
        }
    }
    if o.pattern.as_deref().is_none_or(str::is_empty) {
        return Err("give --text or --regex".into());
    }
    Ok(o)
}

/// Local midnight at the start of `YYYY-MM-DD`.
fn parse_date(s: &str) -> Result<SystemTime, String> {
    use chrono::TimeZone as _;
    let d = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| format!("--modified-since: expected YYYY-MM-DD, got {s}"))?;
    let local = chrono::Local
        .from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap_or_default())
        .earliest()
        .ok_or_else(|| format!("--modified-since: no such local time: {s}"))?;
    Ok(local.into())
}

/// One printed hit.
#[derive(serde::Serialize)]
struct Hit {
    path: String,
    line: usize,
    text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    before: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    after: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fields: Vec<(String, String)>,
}

struct Outcome {
    hits: Vec<Hit>,
    files_with_hits: usize,
    searched: usize,
}

/// Files under `o.dir` that pass the name, date and `--newest` filters.
fn candidate_files(o: &Opts) -> Result<Vec<PathBuf>, String> {
    if !o.dir.is_dir() {
        return Err(format!("not a folder: {}", o.dir.display()));
    }
    let mut files: Vec<(PathBuf, SystemTime)> = Vec::new();
    // Never search the results file itself.
    let out = o.out.as_ref().and_then(|p| p.canonicalize().ok());
    let walker = walkdir::WalkDir::new(&o.dir).max_depth(if o.recursive { usize::MAX } else { 1 });
    for entry in walker.into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if !o.globs.is_empty() && !o.globs.iter().any(|g| search::glob_match(g, &name)) {
            continue;
        }
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if o.modified_since.is_some_and(|since| mtime < since) {
            continue;
        }
        if out.is_some() && entry.path().canonicalize().ok() == out {
            continue;
        }
        files.push((entry.into_path(), mtime));
    }
    if let Some(n) = o.newest {
        files.sort_by_key(|f| std::cmp::Reverse(f.1));
        files.truncate(n);
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files.into_iter().map(|(p, _)| p).collect())
}

fn search(o: &Opts) -> Result<Outcome, String> {
    let opts = SearchOptions {
        mode: if o.regex {
            SearchMode::Regex
        } else {
            SearchMode::Normal
        },
        match_case: !o.ignore_case,
        ..Default::default()
    };
    let re = search::build_regex(o.pattern.as_deref().unwrap_or_default(), &opts)
        .map_err(|e| format!("invalid regular expression: {e}"))?;
    let clean = |s: &str| {
        let s = if o.decode_entities {
            crate::textops::decode_html_entities(s)
        } else {
            s.to_string()
        };
        cut(&s, o.max_line_chars)
    };
    let mut out = Outcome {
        hits: Vec::new(),
        files_with_hits: 0,
        searched: 0,
    };
    for path in candidate_files(o)? {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes.iter().take(8000).any(|&b| b == 0) {
            continue; // binary
        }
        out.searched += 1;
        let text = crate::encoding::TextEncoding::detect(&bytes).decode(&bytes);
        let matches = search::find_all(&re, &text);
        if matches.is_empty() {
            continue;
        }
        out.files_with_hits += 1;
        let lines: Vec<&str> = text.lines().collect();
        let header = if o.fields {
            lines
                .first()
                .map(|h| crate::datatools::split_csv_line(h.trim_start_matches('\u{feff}')))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let shown = path_for_output(&path);
        for h in search::hits_by_line(&text, &matches) {
            let around = |r: std::ops::Range<usize>| {
                r.filter_map(|i| lines.get(i).map(|l| clean(l)))
                    .collect::<Vec<_>>()
            };
            let fields = if o.fields {
                let values = crate::datatools::split_csv_line(&h.text);
                (0..header.len().max(values.len()))
                    .map(|i| {
                        let name = header
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| format!("column_{}", i + 1));
                        (name, clean(values.get(i).map_or("", String::as_str)))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            out.hits.push(Hit {
                path: shown.clone(),
                line: h.line + 1,
                text: clean(&h.text),
                before: around(h.line.saturating_sub(o.context)..h.line),
                after: around(h.line + 1..h.line + 1 + o.context),
                fields,
            });
        }
    }
    Ok(out)
}

fn path_for_output(p: &Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix("./")
        .or_else(|| s.strip_prefix(".\\"))
        .unwrap_or(&s)
        .to_string()
}

/// `s` cut to `max` characters (0 = no limit), marked with `…`.
fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) if max > 0 => format!("{}…", &s[..i]),
        _ => s.to_string(),
    }
}

fn summary(r: &Outcome) -> String {
    let s = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    format!(
        "{} in {} of {} searched",
        s(r.hits.len(), "hit", "hits"),
        s(r.files_with_hits, "file", "files"),
        r.searched
    )
}

fn render_text(r: &Outcome) -> String {
    let mut out = String::new();
    for h in &r.hits {
        let first = h.line - h.before.len();
        for (i, l) in h.before.iter().enumerate() {
            let _ = writeln!(out, "{}-{}- {l}", h.path, first + i);
        }
        if h.fields.is_empty() {
            let _ = writeln!(out, "{}:{}: {}", h.path, h.line, h.text);
        } else {
            let _ = writeln!(out, "{}:{}:", h.path, h.line);
            for (k, v) in &h.fields {
                let _ = writeln!(out, "  {k}: {v}");
            }
        }
        for (i, l) in h.after.iter().enumerate() {
            let _ = writeln!(out, "{}-{}- {l}", h.path, h.line + 1 + i);
        }
        if !h.before.is_empty() || !h.after.is_empty() {
            out.push_str("--\n");
        }
    }
    out.push_str(&summary(r));
    out.push('\n');
    out
}

fn render_json(r: &Outcome) -> String {
    let hits: Vec<serde_json::Value> = r
        .hits
        .iter()
        .map(|h| {
            let mut v = serde_json::to_value(h).unwrap_or_default();
            // Fields as an object keeps the header order readable.
            if let Some(obj) = v.as_object_mut()
                && !h.fields.is_empty()
            {
                let map: serde_json::Map<String, serde_json::Value> = h
                    .fields
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect();
                obj.insert("fields".into(), serde_json::Value::Object(map));
            }
            v
        })
        .collect();
    let doc = serde_json::json!({
        "hits": hits,
        "total_hits": r.hits.len(),
        "files_with_hits": r.files_with_hits,
        "files_searched": r.searched,
        "summary": summary(r),
    });
    let mut s = serde_json::to_string_pretty(&doc).unwrap_or_default();
    s.push('\n');
    s
}

/// A release Windows build has no console of its own: borrow the parent's
/// (the cmd window it was started from) unless stdout is already redirected.
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

/// Runs `search` with the arguments after the word `search`; returns the
/// process exit code.
pub fn run(args: &[String]) -> i32 {
    #[cfg(windows)]
    attach_console();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return 0;
    }
    let opts = match parse(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("search: {e}\n\n{USAGE}");
            write_error(args, &e);
            return 2;
        }
    };
    let result = match search(&opts) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("search: {e}");
            if let Some(p) = &opts.out {
                let _ = std::fs::write(p, format!("error: {e}\n"));
            }
            return 2;
        }
    };
    let text = if opts.json {
        render_json(&result)
    } else {
        render_text(&result)
    };
    let written = match &opts.out {
        Some(p) => std::fs::write(p, &text).map_err(|e| format!("{}: {e}", p.display())),
        None => {
            let mut so = std::io::stdout().lock();
            so.write_all(text.as_bytes())
                .and_then(|()| so.flush())
                .map_err(|e| e.to_string())
        }
    };
    if let Err(e) = written {
        eprintln!("search: can't write results: {e}");
        return 2;
    }
    i32::from(result.hits.is_empty())
}

/// On a bad command line, still put the error in `--out` when one was
/// given: a Windows GUI build has nowhere else to show it.
fn write_error(args: &[String], e: &str) {
    let out = args.iter().enumerate().find_map(|(i, a)| {
        a.strip_prefix("--out=")
            .map(str::to_string)
            .or_else(|| (a == "--out").then(|| args.get(i + 1).cloned()).flatten())
    });
    if let Some(p) = out {
        let _ = std::fs::write(p, format!("error: {e}\n\n{USAGE}\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(ToString::to_string).collect()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sfn-cli-search-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parses_options() {
        let o = parse(&args(&[
            "--dir",
            "x",
            "--glob",
            "*.txt",
            "--glob=*.log",
            "--regex",
            "^4",
            "-i",
            "--context=2",
            "--format",
            "json",
        ]))
        .unwrap();
        assert_eq!(o.globs, vec!["*.txt", "*.log"]);
        assert!(o.regex && o.ignore_case && o.json);
        assert_eq!(o.context, 2);
        assert!(parse(&args(&["--dir", "x"])).is_err());
        assert!(parse(&args(&["--text", "a", "--bogus"])).is_err());
        assert!(parse(&args(&["--text", "a", "--newest", "x"])).is_err());
    }

    #[test]
    fn searches_files_and_prints_fields() {
        let d = temp_dir("fields");
        let header = "CheckNumber,Status,Response";
        std::fs::write(
            d.join("export-1.txt"),
            format!("{header}\n46411,Failed,\"{{&quot;error&quot;:&quot;bad, total&quot;}}\"\n"),
        )
        .unwrap();
        std::fs::write(d.join("export-2.txt"), format!("{header}\n1,Ok,\n")).unwrap();
        std::fs::write(d.join("other.log"), "46411,\n").unwrap();
        let o = parse(&args(&[
            "--dir",
            d.to_str().unwrap(),
            "--glob",
            "export-*.txt",
            "--regex",
            "^46411,",
            "--fields",
            "--decode-entities",
        ]))
        .unwrap();
        let r = search(&o).unwrap();
        assert_eq!((r.hits.len(), r.files_with_hits, r.searched), (1, 1, 2));
        let text = render_text(&r);
        assert!(text.contains(":2:\n  CheckNumber: 46411\n"), "{text}");
        assert!(
            text.contains("  Response: {\"error\":\"bad, total\"}\n"),
            "{text}"
        );
        assert!(text.ends_with("1 hit in 1 file of 2 searched\n"), "{text}");
        let json: serde_json::Value = serde_json::from_str(&render_json(&r)).unwrap();
        assert_eq!(json["hits"][0]["fields"]["Status"], "Failed");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn context_newest_and_exit_codes() {
        let d = temp_dir("ctx");
        std::fs::write(d.join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        let o = parse(&args(&[
            "--dir",
            d.to_str().unwrap(),
            "--text",
            "THREE",
            "-i",
            "--context",
            "1",
            "--max-line-chars",
            "3",
        ]))
        .unwrap();
        let text = render_text(&search(&o).unwrap());
        assert!(text.contains("a.txt-2- two\n"), "{text}");
        assert!(text.contains("a.txt:3: thr…\n"), "{text}");
        assert!(text.contains("a.txt-4- fou…\n"), "{text}");

        let out = d.join("hits.txt");
        let base = ["--dir", d.to_str().unwrap(), "--out", out.to_str().unwrap()];
        let mut a = args(&base);
        a.extend(args(&["--text", "two"]));
        assert_eq!(run(&a), 0);
        assert!(std::fs::read_to_string(&out).unwrap().contains(":2: two"));
        let mut a = args(&base);
        a.extend(args(&["--text", "nope"]));
        assert_eq!(run(&a), 1);
        let mut a = args(&base);
        a.extend(args(&["--regex", "("]));
        assert_eq!(run(&a), 2);
        assert!(std::fs::read_to_string(&out).unwrap().starts_with("error:"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
