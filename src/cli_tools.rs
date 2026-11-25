//! Tool commands: format, minify, validate, data (CSV/TSV/JSON), base64,
//! url, qp, hash, password, time and uuid.

use std::fmt::Write as _;
#[cfg(test)]
use std::path::Path;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone as _, Utc};

use crate::cli::{self, Args, Source};
use crate::formatters::{self as fm, FormatOptions};
use crate::formatters_web as web;

// ------------------------------------------------------- format / validate

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lang {
    Json,
    Xml,
    Html,
    Css,
    Js,
    Ts,
    Markdown,
    Csv,
    Tsv,
}

impl Lang {
    fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().trim_start_matches('.') {
            "json" | "geojson" | "jsonc" | "har" => Self::Json,
            "xml" | "svg" | "xsd" | "xsl" | "xslt" | "config" | "csproj" | "vbproj" | "plist"
            | "rss" | "atom" | "wsdl" | "xaml" | "resx" | "nuspec" | "props" | "targets" => {
                Self::Xml
            }
            "html" | "htm" | "xhtml" => Self::Html,
            "css" => Self::Css,
            "js" | "mjs" | "cjs" | "jsx" | "javascript" => Self::Js,
            "ts" | "tsx" | "mts" | "cts" | "typescript" => Self::Ts,
            "md" | "markdown" | "mdown" => Self::Markdown,
            "csv" => Self::Csv,
            "tsv" | "tab" => Self::Tsv,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Xml => "XML",
            Self::Html => "HTML",
            Self::Css => "CSS",
            Self::Js => "JavaScript",
            Self::Ts => "TypeScript",
            Self::Markdown => "Markdown",
            Self::Csv => "CSV",
            Self::Tsv => "TSV",
        }
    }

    /// `--lang`, else the file's extension, else a guess from the text.
    fn pick(a: &Args, src: &Source, text: &str) -> Result<Self, String> {
        if let Some(l) = a.get("--lang") {
            return Self::parse(l).ok_or_else(|| {
                format!("unknown --lang {l} (json, xml, html, css, js, ts, md, csv, tsv)")
            });
        }
        if let Source::File(p) = src
            && let Some(l) = p
                .extension()
                .and_then(|e| Self::parse(&e.to_string_lossy()))
        {
            return Ok(l);
        }
        let t = text.trim_start_matches('\u{feff}').trim_start();
        let lower = t.get(..t.len().min(200)).unwrap_or(t).to_ascii_lowercase();
        if t.starts_with('{') || t.starts_with('[') {
            Ok(Self::Json)
        } else if lower.starts_with("<!doctype html") || lower.starts_with("<html") {
            Ok(Self::Html)
        } else if t.starts_with('<') {
            Ok(Self::Xml)
        } else {
            Err(
                "can't tell the format: add --lang (json, xml, html, css, js, ts, md, csv, tsv)"
                    .into(),
            )
        }
    }
}

pub const FORMAT: &str = "\
Usage: format (<file> | --text <s> | -) [options]

  --lang <l>             json, xml, html, css, js, ts, md (default: from the
                         file's extension, or guessed from the text)
  --indent <n>           spaces per level (default 2)
  --tabs                 indent with tabs
  --width <n>            wrap long lines at n characters (default 80)
  --in-place             rewrite the file (keeps its encoding and line endings)
  --backup               with --in-place: keep <file>.bak
  --force                with --in-place: even with unsaved changes in the app
  --out <file>           write the result there (default: stdout)

The same formatters as Tools > Format in the app.";

pub const MINIFY: &str = "\
Usage: minify (<file> | --text <s> | -) [--lang json|xml|css] [--in-place [--backup]] [--out <file>]

  Removes indentation and line breaks from JSON, XML or CSS.";

pub const VALIDATE: &str = "\
Usage: validate (<file> | --text <s> | -) [--lang json|xml|csv|tsv] [--out <file>]

  Prints `valid JSON` (exit 0) or the error with its line and column (exit 1).
  CSV/TSV: quotes must be closed and no row may have more fields than the header.";

const FORMAT_SPEC: &[&str] = &[
    "--text=",
    "--escapes",
    "--lang=",
    "--indent=",
    "--tabs",
    "--width=",
    "--in-place",
    "--backup",
    "--force",
    "--out=",
];

fn format_options(a: &Args) -> Result<FormatOptions, String> {
    let mut o = FormatOptions::default();
    if let Some(n) = a.usize("--indent")? {
        o.tab_width = n.clamp(1, 16);
    }
    o.use_tabs = a.has("--tabs");
    if let Some(w) = a.usize("--width")? {
        o.print_width = w.max(20);
    }
    Ok(o)
}

/// Sends the result to `--out`/stdout, or back into the file.
fn finish(a: &Args, src: &Source, result: &str) -> Result<i32, String> {
    if a.has("--in-place") {
        let Source::File(p) = src else {
            return Err("--in-place needs a file".into());
        };
        cli::guard_unsaved(p, a.has("--force"))?;
        let f = cli::read_text_file(p)?;
        let text = cli::with_eol(result, f.eol.unwrap_or(crate::encoding::Eol::Lf));
        cli::write_text_file(p, f.encoding, &cli::line(text), a.has("--backup"))?;
        println!("{}: done", p.display());
        return Ok(0);
    }
    cli::emit(a, &cli::line(result.to_string()))?;
    Ok(0)
}

pub fn format(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(args, FORMAT_SPEC)?;
    let src = Source::from_args(&a, 0)?;
    let text = src.text()?;
    let lang = Lang::pick(&a, &src, &text)?;
    let o = format_options(&a)?;
    let out = match lang {
        Lang::Json => fm::json_format(&text, &o),
        Lang::Xml => fm::xml_format(&text, &o),
        Lang::Html => web::html_format(&text, &o),
        Lang::Css => web::css_format(&text, &o),
        Lang::Js | Lang::Ts => web::js_format(&text, &o),
        Lang::Markdown => web::markdown_format(&text, &o),
        Lang::Csv | Lang::Tsv => {
            return Err(format!(
                "no formatter for {} (try `data` or `validate`)",
                lang.name()
            ));
        }
    }?;
    finish(&a, &src, &out)
}

pub fn minify(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(args, FORMAT_SPEC)?;
    let src = Source::from_args(&a, 0)?;
    let text = src.text()?;
    let lang = Lang::pick(&a, &src, &text)?;
    let out = match lang {
        Lang::Json => fm::json_minify(&text),
        Lang::Xml => fm::xml_minify(&text, &FormatOptions::default()),
        Lang::Css => web::css_minify(&text),
        _ => {
            return Err(format!(
                "no minifier for {} (JSON, XML and CSS can be minified)",
                lang.name()
            ));
        }
    }?;
    finish(&a, &src, &out)
}

pub fn validate(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(args, &["--text=", "--escapes", "--lang=", "--out="])?;
    let src = Source::from_args(&a, 0)?;
    let text = src.text()?;
    let lang = Lang::pick(&a, &src, &text)?;
    let result = match lang {
        Lang::Json => fm::json_validate(&text),
        Lang::Xml => fm::xml_validate(&text),
        Lang::Csv => crate::datatools::csv_to_json(&text, ',').map(drop),
        Lang::Tsv => crate::datatools::csv_to_json(&text, '\t').map(drop),
        _ => {
            return Err(format!(
                "no validator for {} (JSON, XML, CSV and TSV can be checked)",
                lang.name()
            ));
        }
    };
    let name = src.name();
    match result {
        Ok(()) => {
            cli::emit(&a, &format!("{name}: valid {}\n", lang.name()))?;
            Ok(0)
        }
        Err(e) => {
            cli::emit(&a, &format!("{name}: invalid {}: {e}\n", lang.name()))?;
            Ok(1)
        }
    }
}

// --------------------------------------------------------------------- data

pub const DATA: &str = "\
Usage: data (<file> | --text <s> | -) --to json|csv|tsv [--from csv|tsv|json] [--out <file>]

  Converts between CSV, TSV and JSON (an array of objects; the CSV header row
  gives the keys). --from defaults to the file's extension, or a guess.";

pub fn data(args: &[String]) -> Result<i32, String> {
    use crate::datatools as d;
    let a = Args::parse(
        args,
        &["--text=", "--escapes", "--from=", "--to=", "--out="],
    )?;
    let src = Source::from_args(&a, 0)?;
    let text = src.text()?;
    let kind = |s: &str| match Lang::parse(s) {
        Some(l @ (Lang::Json | Lang::Csv | Lang::Tsv)) => Ok(l),
        _ => Err(format!("expected csv, tsv or json, got {s}")),
    };
    let to = kind(a.get("--to").ok_or("give --to json, csv or tsv")?)?;
    let from = match a.get("--from") {
        Some(f) => kind(f)?,
        None => match Lang::pick(&a, &src, &text) {
            Ok(l @ (Lang::Json | Lang::Csv | Lang::Tsv)) => l,
            _ if text.lines().next().is_some_and(|l| l.contains('\t')) => Lang::Tsv,
            _ => Lang::Csv,
        },
    };
    let out = match (from, to) {
        (x, y) if x == y => return Err(format!("the input is already {}", x.name())),
        (Lang::Csv, Lang::Json) => d::csv_to_json(&text, ','),
        (Lang::Tsv, Lang::Json) => d::tsv_to_csv(&text).and_then(|c| d::csv_to_json(&c, ',')),
        (Lang::Json, Lang::Csv) => d::json_to_csv(&text, ','),
        (Lang::Json, Lang::Tsv) => d::json_to_csv(&text, ',').and_then(|c| d::csv_to_tsv(&c)),
        (Lang::Csv, Lang::Tsv) => d::csv_to_tsv(&text),
        (Lang::Tsv, Lang::Csv) => d::tsv_to_csv(&text),
        _ => unreachable!("only CSV, TSV and JSON get here"),
    }?;
    cli::emit(&a, &cli::line(out))?;
    Ok(0)
}

// ------------------------------------------------------- base64 / url / qp

pub const BASE64: &str = "\
Usage: base64 encode|decode (<file> | --text <s> | -) [--url-safe] [--wrap <n>] [--out <file>]

  encode reads the input's exact bytes (any file, also binary); decode writes
  the decoded bytes (use --out for binary data). --url-safe uses - and _
  without padding; --wrap breaks encoded output into lines of n characters.";

pub const URL: &str = "\
Usage: url encode|decode (--text <s> | <file> | -) [--plus] [--out <file>]

  Percent-encoding (RFC 3986). --plus: when decoding, + means a space.";

pub const QP: &str = "\
Usage: qp encode|decode (--text <s> | <file> | -) [--out <file>]

  Quoted-printable (RFC 2045), as used in email.";

fn direction(a: &Args) -> Result<bool, String> {
    match a.pos.first().map(String::as_str) {
        Some("encode" | "enc" | "e") => Ok(true),
        Some("decode" | "dec" | "d") => Ok(false),
        _ => Err("say encode or decode first".into()),
    }
}

pub fn base64(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(
        args,
        &["--text=", "--escapes", "--url-safe", "--wrap=", "--out="],
    )?;
    let encode = direction(&a)?;
    let src = Source::from_args(&a, 1)?;
    if encode {
        let mut s = fm::base64_encode(&src.bytes()?, a.has("--url-safe"));
        if let Some(w) = a.usize("--wrap")?.filter(|w| *w > 0) {
            s = s
                .as_bytes()
                .chunks(w)
                .map(|c| String::from_utf8_lossy(c).into_owned())
                .collect::<Vec<_>>()
                .join("\n");
        }
        cli::emit(&a, &cli::line(s))?;
    } else {
        let bytes = fm::base64_decode(&src.text()?)?;
        if a.out().is_none() && std::str::from_utf8(&bytes).is_err() {
            return Err("the decoded data isn't text: add --out <file> to save it".into());
        }
        cli::emit_bytes(&a, &bytes)?;
    }
    Ok(0)
}

/// Text input without the line ending a pipe or a file adds at the end.
fn trimmed(src: &Source) -> Result<String, String> {
    let t = src.text()?;
    Ok(t.strip_suffix('\n')
        .map_or(t.as_str(), |s| s.strip_suffix('\r').unwrap_or(s))
        .to_string())
}

pub fn url(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(args, &["--text=", "--escapes", "--plus", "--out="])?;
    let encode = direction(&a)?;
    let text = trimmed(&Source::from_args(&a, 1)?)?;
    let out = if encode {
        fm::url_encode(&text)
    } else {
        fm::url_decode(&text, a.has("--plus"))?
    };
    cli::emit(&a, &cli::line(out))?;
    Ok(0)
}

pub fn qp(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(args, &["--text=", "--escapes", "--out="])?;
    let encode = direction(&a)?;
    let text = Source::from_args(&a, 1)?.text()?;
    let out = if encode {
        fm::qp_encode(&text)
    } else {
        fm::qp_decode(&text)?
    };
    cli::emit(&a, &cli::line(out))?;
    Ok(0)
}

// --------------------------------------------------------------------- hash

pub const HASH: &str = "\
Usage: hash (<file>... | --text <s> | -) [--algo md5|sha1|sha256|sha512|all] [options]

  --algo <a>             default sha256; `all` prints all four
  --upper                upper-case hex
  --check <hex>          compare with this hash: exit 0 if it matches, 1 if not
  --format text|json     output format
  --out <file>           write the result to a file

Files are hashed byte for byte; --text is hashed as UTF-8. Output is
`<hash>  <file>`, like sha256sum.";

pub fn hash(args: &[String]) -> Result<i32, String> {
    use crate::textops::{HashAlgo, hex_digest};
    let a = Args::parse(
        args,
        &[
            "--text=",
            "--escapes",
            "--algo=",
            "--upper",
            "--check=",
            "--format=",
            "--out=",
        ],
    )?;
    let json = a.json()?;
    let algos: Vec<HashAlgo> = match a
        .get("--algo")
        .unwrap_or("sha256")
        .to_ascii_lowercase()
        .replace('-', "")
        .as_str()
    {
        "md5" => vec![HashAlgo::Md5],
        "sha1" => vec![HashAlgo::Sha1],
        "sha256" => vec![HashAlgo::Sha256],
        "sha512" => vec![HashAlgo::Sha512],
        "all" => vec![
            HashAlgo::Md5,
            HashAlgo::Sha1,
            HashAlgo::Sha256,
            HashAlgo::Sha512,
        ],
        other => {
            return Err(format!(
                "unknown --algo {other} (md5, sha1, sha256, sha512, all)"
            ));
        }
    };
    let sources: Vec<Source> = if a.get("--text").is_some() || a.pos.is_empty() {
        vec![Source::from_args(&a, 0)?]
    } else {
        a.pos
            .iter()
            .map(|p| {
                if p == "-" {
                    Source::Stdin
                } else {
                    Source::File(p.into())
                }
            })
            .collect()
    };
    let mut out = String::new();
    let mut docs = Vec::new();
    let mut all_match = true;
    for src in &sources {
        let bytes = src.bytes()?;
        let name = src.name();
        for &algo in &algos {
            let mut h = hex_digest(algo, &bytes);
            if a.has("--upper") {
                h = h.to_uppercase();
            }
            if let Some(c) = a.get("--check") {
                all_match &= h.eq_ignore_ascii_case(c.trim());
            }
            if algos.len() > 1 {
                let _ = writeln!(out, "{:<8} {h}  {name}", algo.name());
            } else {
                let _ = writeln!(out, "{h}  {name}");
            }
            docs.push(serde_json::json!({"input": name, "algorithm": algo.name(), "hash": h}));
        }
    }
    if a.get("--check").is_some() {
        let _ = writeln!(
            out,
            "{}",
            if all_match {
                "matches"
            } else {
                "DOES NOT match"
            }
        );
    }
    if json {
        out = cli::line(serde_json::to_string_pretty(&docs).unwrap_or_default());
    }
    cli::emit(&a, &out)?;
    Ok(i32::from(!all_match))
}

// ----------------------------------------------------------------- password

pub const PASSWORD: &str = "\
Usage: password [options]

  -l, --length <n>       characters (default 16, 4 to 1024)
  -c, --count <n>        how many passwords (default 1)
  --no-upper             leave out A-Z
  --no-lower             leave out a-z
  --no-numbers           leave out 0-9
  --no-symbols           leave out symbols
  --symbols <chars>      the symbols to use (default !@#$%^&*<>); a shell-safe
                         choice: --symbols \"-_.@\"
  --no-ambiguous         leave out look-alikes: I l 1 O 0 o
  --exclude <chars>      leave out these characters too
  --out <file>           write the passwords to a file

From the OS secure random source; every chosen kind of character appears at
least once.";

pub fn password(args: &[String]) -> Result<i32, String> {
    use crate::password as pw;
    let a = Args::parse(
        args,
        &[
            "--length|-l=",
            "--count|-c=",
            "--no-upper",
            "--no-lower",
            "--no-numbers",
            "--no-symbols",
            "--symbols=",
            "--no-ambiguous",
            "--exclude=",
            "--out=",
        ],
    )?;
    let length = a.usize("--length")?.unwrap_or(16);
    if !(4..=1024).contains(&length) {
        return Err("--length must be 4 to 1024".into());
    }
    let count = a.usize("--count")?.unwrap_or(1).clamp(1, 10_000);
    let mut drop: String = a.get("--exclude").unwrap_or_default().to_string();
    if a.has("--no-ambiguous") {
        drop.push_str("Il1O0o");
    }
    let mut sets: Vec<&str> = Vec::new();
    if !a.has("--no-lower") {
        sets.push(pw::LOWERCASE);
    }
    if !a.has("--no-upper") {
        sets.push(pw::UPPERCASE);
    }
    if !a.has("--no-numbers") {
        sets.push(pw::NUMBERS);
    }
    let symbols = a.get("--symbols").unwrap_or(pw::SYMBOLS);
    if !a.has("--no-symbols") {
        sets.push(symbols);
    }
    let sets: Vec<Vec<char>> = sets
        .iter()
        .map(|s| s.chars().filter(|c| !drop.contains(*c)).collect::<Vec<_>>())
        .filter(|s| !s.is_empty())
        .collect();
    if sets.is_empty() {
        return Err("every kind of character is turned off".into());
    }
    let mut out = String::new();
    for _ in 0..count {
        out.push_str(&pw::generate_from(length, &sets)?);
        out.push('\n');
    }
    cli::emit(&a, &out)?;
    Ok(0)
}

// --------------------------------------------------------------------- time

pub const TIME: &str = "\
Usage: time [<value>...] [--utc | --local] [--format text|json] [--out <file>]

  <value>                now (default), Unix seconds or milliseconds
                         (1700000000, 1700000000000), or a date:
                         2023-11-14T22:13:20Z, 2023-11-14T17:13:20-05:00,
                         \"2023-11-14 17:13:20\", 2023-11-14 17:13, 2023-11-14
  --utc                  a date without a time zone is UTC
  --local                ... is this computer's local time (the default)

Prints the moment as Unix seconds and milliseconds, UTC and local time.";

/// A moment from the command line.
fn parse_moment(v: &str, utc: bool) -> Result<DateTime<Utc>, String> {
    let t = v.trim();
    if t.eq_ignore_ascii_case("now") {
        return Ok(Utc::now());
    }
    if let Ok(n) = t.parse::<i64>() {
        let ms = if n.unsigned_abs() >= 100_000_000_000 {
            n
        } else {
            n.saturating_mul(1000)
        };
        return DateTime::<Utc>::from_timestamp_millis(ms)
            .ok_or_else(|| format!("out of range: {t}"));
    }
    let iso = t.replacen(' ', "T", 1);
    if let Ok(d) = DateTime::parse_from_rfc3339(&iso) {
        return Ok(d.with_timezone(&Utc));
    }
    let naive = ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(&iso, f).ok())
        .or_else(|| {
            NaiveDate::parse_from_str(t, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        })
        .ok_or_else(|| {
            format!(
                "not a date or Unix time: {t} (examples: 1700000000, 2023-11-14T22:13:20Z, \"2023-11-14 17:13:20\", 2023-11-14)"
            )
        })?;
    if utc {
        return Ok(naive.and_utc());
    }
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|d| d.with_timezone(&Utc))
        .ok_or_else(|| format!("{t} doesn't exist in local time (a daylight saving gap)"))
}

pub fn time(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(args, &["--utc", "--local", "--format=", "--out="])?;
    let json = a.json()?;
    let values: Vec<String> = if a.pos.is_empty() {
        vec!["now".into()]
    } else {
        a.pos.clone()
    };
    let mut out = String::new();
    let mut docs = Vec::new();
    for v in &values {
        let m = parse_moment(v, a.has("--utc"))?;
        let local = m.with_timezone(&Local);
        let utc_s = m.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
        let local_s = local.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, false);
        let human = local.format("%a %d %b %Y %H:%M:%S (UTC%:z)").to_string();
        if json {
            docs.push(serde_json::json!({
                "input": v, "unix": m.timestamp(), "unix_ms": m.timestamp_millis(),
                "utc": utc_s, "local": local_s, "local_text": human,
            }));
        } else {
            if values.len() > 1 {
                let _ = writeln!(out, "{v}");
            }
            let _ = writeln!(
                out,
                "  Unix seconds:  {}\n  Unix ms:       {}\n  UTC:           {utc_s}\n  Local:         {local_s}\n  Local (text):  {human}",
                m.timestamp(),
                m.timestamp_millis()
            );
        }
    }
    if json {
        let v = if docs.len() == 1 {
            docs.remove(0)
        } else {
            serde_json::Value::Array(docs)
        };
        out = cli::line(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    cli::emit(&a, &out)?;
    Ok(0)
}

// --------------------------------------------------------------------- uuid

pub const UUID: &str = "\
Usage: uuid [-c, --count <n>] [--upper] [--out <file>]

  Random UUIDs (version 4) from the OS secure random source.";

pub fn uuid(args: &[String]) -> Result<i32, String> {
    let a = Args::parse(args, &["--count|-c=", "--upper", "--out="])?;
    let count = a.usize("--count")?.unwrap_or(1).clamp(1, 100_000);
    let mut out = String::new();
    for _ in 0..count {
        let mut b = [0u8; 16];
        getrandom::fill(&mut b).map_err(|e| e.to_string())?;
        b[6] = (b[6] & 0x0f) | 0x40;
        b[8] = (b[8] & 0x3f) | 0x80;
        let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let mut u = format!(
            "{}-{}-{}-{}-{}",
            &h[..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..]
        );
        if a.has("--upper") {
            u = u.to_uppercase();
        }
        out.push_str(&u);
        out.push('\n');
    }
    cli::emit(&a, &out)?;
    Ok(0)
}

/// For tests: whether `p` names a file with an extension `format` knows.
#[cfg(test)]
fn known_extension(p: &Path) -> bool {
    p.extension()
        .and_then(|e| Lang::parse(&e.to_string_lossy()))
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        f: fn(&[String]) -> Result<i32, String>,
        args: &[&str],
    ) -> (Result<i32, String>, String) {
        let out = std::env::temp_dir().join(format!(
            "sfn-cli-tools-{}-{}.txt",
            std::process::id(),
            args.join("_")
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .take(40)
                .collect::<String>()
        ));
        let mut v: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        v.push("--out".into());
        v.push(out.display().to_string());
        let code = f(&v);
        let text = std::fs::read_to_string(&out).unwrap_or_default();
        let _ = std::fs::remove_file(&out);
        (code, text)
    }

    #[test]
    fn formats_minifies_and_validates() {
        let (c, t) = run(format, &["--text", r#"{"a":1,"b":[1,2]}"#]);
        assert_eq!(c, Ok(0));
        // Short objects stay on one line, as in the app.
        assert_eq!(t, "{ \"a\": 1, \"b\": [1, 2] }\n");
        let (_, t) = run(minify, &["--text", "{ \"a\" : 1 }"]);
        assert_eq!(t, "{\"a\":1}\n");
        let (c, t) = run(validate, &["--text", "{\"a\":}"]);
        assert_eq!(c, Ok(1));
        assert!(t.contains("invalid JSON"), "{t}");
        let (c, _) = run(validate, &["--text", "<a><b/></a>"]);
        assert_eq!(c, Ok(0));
        let (c, t) = run(validate, &["--text", "a,b\n1,2,3\n", "--lang", "csv"]);
        assert_eq!(c, Ok(1), "{t}");
        let (_, t) = run(format, &["--text", "a{color:red}", "--lang", "css"]);
        assert!(t.contains("color: red;"), "{t}");
        assert!(run(minify, &["--text", "x", "--lang", "md"]).0.is_err());
        assert!(known_extension(Path::new("web.config")));
    }

    #[test]
    fn converts_data() {
        let (_, t) = run(
            data,
            &[
                "--text",
                "a,b\n1,\"x,y\"\n",
                "--from",
                "csv",
                "--to",
                "json",
            ],
        );
        assert!(t.contains("\"b\": \"x,y\""), "{t}");
        let (_, t) = run(
            data,
            &["--text", "[{\"a\":\"1\",\"b\":\"2\"}]", "--to", "tsv"],
        );
        assert_eq!(t, "a\tb\n1\t2\n");
        let (_, t) = run(data, &["--text", "a\tb\n1\t2\n", "--to", "csv"]);
        assert_eq!(t, "a,b\n1,2\n");
    }

    #[test]
    fn encodes_and_hashes() {
        assert_eq!(
            run(base64, &["encode", "--text", "hi there"]).1,
            "aGkgdGhlcmU=\n"
        );
        assert_eq!(
            run(base64, &["decode", "--text", "aGkgdGhlcmU="]).1,
            "hi there"
        );
        assert_eq!(run(url, &["encode", "--text", "a b&c"]).1, "a%20b%26c\n");
        assert_eq!(run(url, &["decode", "--text", "a+b", "--plus"]).1, "a b\n");
        let (_, t) = run(hash, &["--text", "abc"]);
        assert_eq!(
            t,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  (text)\n"
        );
        let (_, t) = run(hash, &["--text", "abc", "--algo", "md5"]);
        assert!(t.starts_with("900150983cd24fb0d6963f7d28e17f72"));
        let (c, _) = run(
            hash,
            &[
                "--text",
                "abc",
                "--algo",
                "sha1",
                "--check",
                "A9993E364706816ABA3E25717850C26C9CD0D89D",
            ],
        );
        assert_eq!(c, Ok(0));
        let (c, _) = run(hash, &["--text", "abc", "--check", "00"]);
        assert_eq!(c, Ok(1));
    }

    #[test]
    fn passwords_follow_the_options() {
        let (_, t) = run(
            password,
            &[
                "--length",
                "40",
                "--count",
                "5",
                "--no-symbols",
                "--no-ambiguous",
            ],
        );
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 5);
        for p in lines {
            assert_eq!(p.chars().count(), 40);
            assert!(p.chars().all(char::is_alphanumeric), "{p}");
            assert!(!p.contains(['I', 'l', '1', 'O', '0', 'o']), "{p}");
        }
        let (_, t) = run(
            password,
            &[
                "-l",
                "12",
                "--no-upper",
                "--no-lower",
                "--no-numbers",
                "--symbols",
                "-_",
            ],
        );
        assert!(t.trim().chars().all(|c| c == '-' || c == '_'), "{t}");
        assert!(
            run(
                password,
                &["--no-upper", "--no-lower", "--no-numbers", "--no-symbols"]
            )
            .0
            .is_err()
        );
    }

    #[test]
    fn converts_times() {
        let utc = |v: &str| parse_moment(v, true).unwrap().timestamp();
        assert_eq!(utc("1700000000"), 1_700_000_000);
        assert_eq!(utc("1700000000000"), 1_700_000_000);
        assert_eq!(utc("2023-11-14T22:13:20Z"), 1_700_000_000);
        assert_eq!(utc("2023-11-14T17:13:20-05:00"), 1_700_000_000);
        assert_eq!(utc("2023-11-14 22:13:20"), 1_700_000_000);
        assert_eq!(utc("2023-11-14 22:13"), 1_699_999_980);
        assert_eq!(utc("2023-11-14"), 1_699_920_000);
        assert!(parse_moment("yesterday", true).is_err());
        let (_, t) = run(time, &["1700000000", "--format", "json"]);
        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
        assert_eq!(v["utc"], "2023-11-14T22:13:20Z");
        assert_eq!(v["unix_ms"], 1_700_000_000_000_i64);
    }

    #[test]
    fn uuids_are_version_4() {
        let (_, t) = run(uuid, &["-c", "3"]);
        for u in t.lines() {
            assert_eq!(u.len(), 36);
            assert_eq!(&u[14..15], "4");
            assert!("89ab".contains(&u[19..20]));
        }
    }
}
