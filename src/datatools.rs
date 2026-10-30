//! Data tools: CSV / JSON / TSV conversion, UUID generation and Unix
//! timestamp <-> date conversion.
//!
//! Every function is pure (text in, text out) apart from `uuid_v4`, which
//! needs a source of randomness.

use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

// ---------------------------------------------------------------- CSV parse

/// One parsed CSV record and the (1-based) line it starts on.
struct Record {
    line: usize,
    fields: Vec<String>,
}

/// RFC 4180 parser. Quoted fields may contain the delimiter, doubled quotes
/// and line breaks (LF, CRLF or CR). A quote inside an unquoted field is
/// kept literally. Completely empty lines are skipped, so a trailing newline
/// or blank last line does not produce an extra record.
fn parse_csv(text: &str, delimiter: char) -> Result<Vec<Record>, String> {
    let mut records = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    // True once the current field started with a quote (even if closed).
    let mut quoted = false;
    let mut line = 1usize;
    let mut record_line = 1usize;
    let mut quote_line = 1usize;
    let mut chars = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .chars()
        .peekable();

    let end_record = |fields: &mut Vec<String>,
                      field: &mut String,
                      quoted: bool,
                      records: &mut Vec<Record>,
                      record_line: usize| {
        let blank = fields.is_empty() && field.is_empty() && !quoted;
        fields.push(std::mem::take(field));
        let fields = std::mem::take(fields);
        if !blank {
            records.push(Record {
                line: record_line,
                fields,
            });
        }
    };

    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                '\n' => {
                    line += 1;
                    field.push(c);
                }
                '\r' => {
                    if chars.peek() != Some(&'\n') {
                        line += 1;
                    }
                    field.push(c);
                }
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() && !quoted => {
                in_quotes = true;
                quoted = true;
                quote_line = line;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                end_record(&mut fields, &mut field, quoted, &mut records, record_line);
                quoted = false;
                line += 1;
                record_line = line;
            }
            c if c == delimiter => {
                fields.push(std::mem::take(&mut field));
                quoted = false;
            }
            _ => field.push(c),
        }
    }
    if in_quotes {
        return Err(format!(
            "Unterminated quoted field starting on line {quote_line}"
        ));
    }
    end_record(&mut fields, &mut field, quoted, &mut records, record_line);
    Ok(records)
}

/// Quote a CSV field if it contains the delimiter, a quote, CR or LF.
fn csv_field(s: &str, delimiter: char) -> String {
    if s.contains([delimiter, '"', '\r', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Write rows as CSV, each line ending in "\n".
fn write_csv<S: AsRef<str>>(rows: &[Vec<S>], delimiter: char) -> String {
    let mut out = String::new();
    let mut sep = [0u8; 4];
    let sep = delimiter.encode_utf8(&mut sep);
    for row in rows {
        let line: Vec<String> = row
            .iter()
            .map(|f| csv_field(f.as_ref(), delimiter))
            .collect();
        out.push_str(&line.join(sep));
        out.push('\n');
    }
    out
}

/// Make header names unique and non-empty: empty → `column_N` (1-based
/// position), repeats → `name_2`, `name_3`, ...
fn dedup_headers(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    for (i, name) in raw.iter().enumerate() {
        let base = if name.is_empty() {
            format!("column_{}", i + 1)
        } else {
            name.clone()
        };
        let mut candidate = base.clone();
        let mut n = 2;
        while out.contains(&candidate) {
            candidate = format!("{base}_{n}");
            n += 1;
        }
        out.push(candidate);
    }
    out
}

/// JSON string literal for `s` (with escaping).
fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

// ------------------------------------------------------------- CSV <-> JSON

/// Convert CSV (RFC 4180) to a pretty-printed JSON array of objects.
///
/// The first row is the header. Keys keep header order; every value is a
/// JSON string (no number guessing, so the conversion is lossless). Empty
/// header names become `column_N` and duplicates get `_2`, `_3` suffixes.
/// Short rows are padded with empty strings; long rows are an error.
pub fn csv_to_json(text: &str, delimiter: char) -> Result<String, String> {
    let records = parse_csv(text, delimiter)?;
    let mut iter = records.into_iter();
    let header = iter.next().ok_or("No header row: the input is empty")?;
    let keys = dedup_headers(&header.fields);
    let rows: Vec<Record> = iter.collect();
    if rows.is_empty() {
        return Ok("[]".to_string());
    }
    let mut out = String::from("[\n");
    for (r, row) in rows.iter().enumerate() {
        if row.fields.len() > keys.len() {
            return Err(format!(
                "Line {}: {} fields, but the header has {}",
                row.line,
                row.fields.len(),
                keys.len()
            ));
        }
        out.push_str("  {\n");
        for (i, key) in keys.iter().enumerate() {
            let value = row.fields.get(i).map(String::as_str).unwrap_or("");
            let comma = if i + 1 < keys.len() { "," } else { "" };
            let _ = writeln!(out, "    {}: {}{comma}", json_str(key), json_str(value));
        }
        out.push_str(if r + 1 < rows.len() {
            "  },\n"
        } else {
            "  }\n"
        });
    }
    out.push(']');
    Ok(out)
}

/// JSON value that keeps object keys in document order (serde_json's
/// `preserve_order` feature is not enabled in this build).
enum Json {
    Null,
    Bool(bool),
    Num(serde_json::Number),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Json;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_unit<E>(self) -> Result<Json, E> {
                Ok(Json::Null)
            }
            fn visit_none<E>(self) -> Result<Json, E> {
                Ok(Json::Null)
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Json, D::Error> {
                Json::deserialize(d)
            }
            fn visit_bool<E>(self, b: bool) -> Result<Json, E> {
                Ok(Json::Bool(b))
            }
            fn visit_i64<E>(self, n: i64) -> Result<Json, E> {
                Ok(Json::Num(n.into()))
            }
            fn visit_u64<E>(self, n: u64) -> Result<Json, E> {
                Ok(Json::Num(n.into()))
            }
            fn visit_f64<E>(self, n: f64) -> Result<Json, E> {
                Ok(serde_json::Number::from_f64(n).map_or(Json::Null, Json::Num))
            }
            fn visit_str<E>(self, s: &str) -> Result<Json, E> {
                Ok(Json::Str(s.to_string()))
            }
            fn visit_string<E>(self, s: String) -> Result<Json, E> {
                Ok(Json::Str(s))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Json::Arr(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
                let mut entries: Vec<(String, Json)> = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, Json>()? {
                    if let Some(slot) = entries.iter_mut().find(|(ek, _)| *ek == k) {
                        slot.1 = v; // Last duplicate wins, first position kept.
                    } else {
                        entries.push((k, v));
                    }
                }
                Ok(Json::Obj(entries))
            }
        }
        d.deserialize_any(V)
    }
}

impl Json {
    /// Compact JSON text, keys in original order.
    fn compact(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(&n.to_string()),
            Json::Str(s) => out.push_str(&json_str(s)),
            Json::Arr(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.compact(out);
                }
                out.push(']');
            }
            Json::Obj(entries) => {
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&json_str(k));
                    out.push(':');
                    v.compact(out);
                }
                out.push('}');
            }
        }
    }

    /// Text of this value as a CSV cell.
    fn cell(&self) -> String {
        match self {
            Json::Null => String::new(),
            Json::Str(s) => s.clone(),
            other => {
                let mut s = String::new();
                other.compact(&mut s);
                s
            }
        }
    }
}

/// Convert a JSON array to CSV.
///
/// An array of objects gets a header row: the union of all keys in
/// first-seen order. An array of arrays is written without a header.
/// Strings are written as-is, numbers and booleans as their JSON text, null
/// as an empty field, and nested objects/arrays as compact JSON. Lines end
/// with "\n".
pub fn json_to_csv(text: &str, delimiter: char) -> Result<String, String> {
    let value: Json = serde_json::from_str(text).map_err(|e| format!("Invalid JSON: {e}"))?;
    let Json::Arr(items) = value else {
        return Err("Expected a JSON array of objects or arrays".to_string());
    };
    if items.is_empty() {
        return Ok(String::new());
    }
    if items.iter().all(|v| matches!(v, Json::Obj(_))) {
        let mut header: Vec<&str> = Vec::new();
        for item in &items {
            if let Json::Obj(entries) = item {
                for (k, _) in entries {
                    if !header.contains(&k.as_str()) {
                        header.push(k);
                    }
                }
            }
        }
        let mut rows: Vec<Vec<String>> = vec![header.iter().map(|s| s.to_string()).collect()];
        for item in &items {
            if let Json::Obj(entries) = item {
                rows.push(
                    header
                        .iter()
                        .map(|h| {
                            entries
                                .iter()
                                .find(|(k, _)| k == h)
                                .map(|(_, v)| v.cell())
                                .unwrap_or_default()
                        })
                        .collect(),
                );
            }
        }
        Ok(write_csv(&rows, delimiter))
    } else if items.iter().all(|v| matches!(v, Json::Arr(_))) {
        let rows: Vec<Vec<String>> = items
            .iter()
            .map(|item| match item {
                Json::Arr(cells) => cells.iter().map(Json::cell).collect(),
                _ => Vec::new(),
            })
            .collect();
        Ok(write_csv(&rows, delimiter))
    } else {
        Err("Expected every array element to be an object, or every one to be an array".to_string())
    }
}

// -------------------------------------------------------------- CSV <-> TSV

/// Convert CSV (comma-separated) to TSV.
///
/// TSV has no quoting, so a field cannot hold a tab or line break. Such
/// characters are escaped: tab → `\t`, LF → `\n`, CR → `\r`, and a literal
/// backslash → `\\`. `tsv_to_csv` reverses this exactly.
pub fn csv_to_tsv(text: &str) -> Result<String, String> {
    let records = parse_csv(text, ',')?;
    let mut out = String::new();
    for record in records {
        let fields: Vec<String> = record.fields.iter().map(|f| tsv_escape(f)).collect();
        out.push_str(&fields.join("\t"));
        out.push('\n');
    }
    Ok(out)
}

/// Convert TSV to CSV (comma-separated), undoing the `\t`, `\n`, `\r`, `\\`
/// escapes written by `csv_to_tsv`. Other backslash sequences are kept
/// as-is. Fields are quoted when needed; lines end with "\n".
pub fn tsv_to_csv(text: &str) -> Result<String, String> {
    let rows: Vec<Vec<String>> = text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.split('\t').map(tsv_unescape).collect())
        .collect();
    Ok(write_csv(&rows, ','))
}

fn tsv_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn tsv_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

// --------------------------------------------------------------------- UUID

/// A random version-4 UUID, lowercase and hyphenated
/// (`xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`, `y` in `89ab`).
///
/// Randomness comes from std's randomly keyed `RandomState` hashing a
/// process-wide counter and the current time; good enough for identifiers,
/// not for cryptographic use.
pub fn uuid_v4() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let state = RandomState::new();
    let hi = state.hash_one((count, nanos, 0u8));
    let lo = state.hash_one((count, nanos, 1u8));
    let mut bits = (u128::from(hi) << 64) | u128::from(lo);
    bits = (bits & !(0xf << 76)) | (0x4 << 76); // version 4
    bits = (bits & !(0x3 << 62)) | (0x2 << 62); // variant 10xx
    let hex = format!("{bits:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

// --------------------------------------------------------------- timestamps

/// Convert a Unix timestamp to an RFC 3339 UTC date.
///
/// Values with magnitude >= 1e11 are taken as milliseconds, smaller ones as
/// seconds. A `.mmm` part is added only when the milliseconds are non-zero.
/// Example: `1700000000` → `2023-11-14T22:13:20Z`.
pub fn timestamp_to_date(text: &str) -> Result<String, String> {
    let t = text.trim();
    let n: i64 = t
        .parse()
        .map_err(|_| format!("Not a Unix timestamp: \"{t}\""))?;
    let (secs, millis) = if n.unsigned_abs() >= 100_000_000_000 {
        (n.div_euclid(1000), n.rem_euclid(1000))
    } else {
        (n, 0)
    };
    let dt = DateTime::<Utc>::from_timestamp(secs, (millis * 1_000_000) as u32)
        .ok_or_else(|| format!("Timestamp out of range: {t}"))?;
    let base = dt.format("%Y-%m-%dT%H:%M:%S").to_string();
    Ok(if millis == 0 {
        format!("{base}Z")
    } else {
        format!("{base}.{millis:03}Z")
    })
}

/// Convert a date to a Unix timestamp in seconds (decimal string).
///
/// Accepts RFC 3339 (`Z` or `+02:00` offsets, optional fractional seconds,
/// which are truncated), `YYYY-MM-DD HH:MM:SS` (UTC) and `YYYY-MM-DD`
/// (midnight UTC).
pub fn date_to_timestamp(text: &str) -> Result<String, String> {
    let t = text.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(t) {
        return Ok(dt.timestamp().to_string());
    }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(t, fmt) {
            return Ok(dt.and_utc().timestamp().to_string());
        }
    }
    if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d")
        && let Some(dt) = d.and_hms_opt(0, 0, 0)
    {
        return Ok(dt.and_utc().timestamp().to_string());
    }
    Err(format!(
        "Not a recognised date: \"{t}\" (use RFC 3339, YYYY-MM-DD HH:MM:SS or YYYY-MM-DD)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn csv_to_json_basic_and_key_order() {
        let out = csv_to_json("zeta,alpha\n1,2\n3,4\n", ',').unwrap();
        assert_eq!(
            out,
            "[\n  {\n    \"zeta\": \"1\",\n    \"alpha\": \"2\"\n  },\n  {\n    \"zeta\": \"3\",\n    \"alpha\": \"4\"\n  }\n]"
        );
        // No trailing newline gives the same result.
        assert_eq!(csv_to_json("zeta,alpha\n1,2\n3,4", ',').unwrap(), out);
        assert_eq!(csv_to_json("a,b\n", ',').unwrap(), "[]");
        assert!(csv_to_json("", ',').is_err());
    }

    #[test]
    fn csv_quoting_edge_cases() {
        let csv = "a,b,c\r\n\"say \"\"hi\"\"\",\"x,y\",\"line1\r\nline2\"\r\n\r\n";
        let v = parsed(&csv_to_json(csv, ',').unwrap());
        assert_eq!(v[0]["a"], "say \"hi\"");
        assert_eq!(v[0]["b"], "x,y");
        assert_eq!(v[0]["c"], "line1\r\nline2");
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert!(csv_to_json("a\n\"open", ',').is_err());
    }

    #[test]
    fn csv_semicolon_delimiter_and_numbers_stay_strings() {
        let v = parsed(&csv_to_json("n;s\n42;a,b\n", ';').unwrap());
        assert_eq!(v[0]["n"], "42");
        assert_eq!(v[0]["s"], "a,b");
    }

    #[test]
    fn header_dedup() {
        let out = csv_to_json("name,name,,name\n1,2,3,4\n", ',').unwrap();
        let keys: Vec<&str> = out
            .lines()
            .filter_map(|l| l.trim().strip_prefix('"'))
            .map(|l| l.split('"').next().unwrap())
            .collect();
        assert_eq!(keys, ["name", "name_2", "column_3", "name_3"]);
    }

    #[test]
    fn ragged_rows() {
        let v = parsed(&csv_to_json("a,b,c\n1\n", ',').unwrap());
        assert_eq!(v[0]["a"], "1");
        assert_eq!(v[0]["b"], "");
        assert_eq!(v[0]["c"], "");
        let err = csv_to_json("a,b\n1,2\n\"x\ny\",2\n1,2,3\n", ',').unwrap_err();
        assert!(err.contains("Line 5"), "{err}");
    }

    #[test]
    fn json_to_csv_objects_nested_and_null() {
        let json = r#"[
            {"id": 1, "name": "Ann, B", "tags": ["x", "y"], "meta": {"z": 1, "a": null}},
            {"id": 2.5, "ok": true, "name": "say \"hi\"", "meta": null}
        ]"#;
        let out = json_to_csv(json, ',').unwrap();
        assert_eq!(
            out,
            "id,name,tags,meta,ok\n\
             1,\"Ann, B\",\"[\"\"x\"\",\"\"y\"\"]\",\"{\"\"z\"\":1,\"\"a\"\":null}\",\n\
             2.5,\"say \"\"hi\"\"\",,,true\n"
        );
    }

    #[test]
    fn json_to_csv_arrays_and_errors() {
        assert_eq!(
            json_to_csv(r#"[[1,"a\nb"],[null,false]]"#, '\t').unwrap(),
            "1\t\"a\nb\"\n\tfalse\n"
        );
        assert_eq!(json_to_csv("[]", ',').unwrap(), "");
        assert!(json_to_csv("{}", ',').is_err());
        assert!(json_to_csv("[1,2]", ',').is_err());
        assert!(json_to_csv("[{}, []]", ',').is_err());
        assert!(json_to_csv("[", ',').is_err());
    }

    #[test]
    fn csv_json_csv_round_trip() {
        let csv = "b,a,c\n\"q\"\"uote\",\"x,y\",\"multi\nline\"\n,2,\n";
        let json = csv_to_json(csv, ',').unwrap();
        assert_eq!(json_to_csv(&json, ',').unwrap(), csv);
    }

    #[test]
    fn csv_tsv_escaping_round_trip() {
        let csv = "a,b\ntab\there,back\\slash\n\"two\nlines\",\"cr\rx\"\n";
        let tsv = csv_to_tsv(csv).unwrap();
        assert_eq!(
            tsv,
            "a\tb\ntab\\there\tback\\\\slash\ntwo\\nlines\tcr\\rx\n"
        );
        assert_eq!(tsv_to_csv(&tsv).unwrap(), csv);
        assert_eq!(tsv_to_csv("x\t\\q\r\n").unwrap(), "x,\\q\n");
    }

    #[test]
    fn uuid_format() {
        let a = uuid_v4();
        let b = uuid_v4();
        assert_ne!(a, b);
        for u in [&a, &b] {
            assert_eq!(u.len(), 36);
            let parts: Vec<&str> = u.split('-').collect();
            assert_eq!(
                parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
                [8, 4, 4, 4, 12]
            );
            assert!(
                u.chars()
                    .all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c))
            );
            assert_eq!(&u[14..15], "4");
            assert!("89ab".contains(&u[19..20]), "{u}");
        }
    }

    #[test]
    fn timestamp_to_date_seconds_and_millis() {
        assert_eq!(
            timestamp_to_date("1700000000").unwrap(),
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(
            timestamp_to_date("  1700000000123\n").unwrap(),
            "2023-11-14T22:13:20.123Z"
        );
        assert_eq!(
            timestamp_to_date("1700000000000").unwrap(),
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(timestamp_to_date("0").unwrap(), "1970-01-01T00:00:00Z");
        assert_eq!(timestamp_to_date("-1").unwrap(), "1969-12-31T23:59:59Z");
        assert!(timestamp_to_date("abc").is_err());
        assert!(timestamp_to_date("").is_err());
        assert!(timestamp_to_date("1.5").is_err());
    }

    #[test]
    fn date_to_timestamp_formats_and_offsets() {
        assert_eq!(
            date_to_timestamp("2023-11-14T22:13:20Z").unwrap(),
            "1700000000"
        );
        assert_eq!(
            date_to_timestamp("2023-11-14T22:13:20.123Z").unwrap(),
            "1700000000"
        );
        assert_eq!(
            date_to_timestamp("2023-11-15T00:13:20+02:00").unwrap(),
            "1700000000"
        );
        assert_eq!(
            date_to_timestamp("2023-11-14T17:13:20-05:00").unwrap(),
            "1700000000"
        );
        assert_eq!(
            date_to_timestamp(" 2023-11-14 22:13:20 ").unwrap(),
            "1700000000"
        );
        assert_eq!(date_to_timestamp("1970-01-02").unwrap(), "86400");
        assert!(date_to_timestamp("yesterday").is_err());
        assert!(date_to_timestamp("2023-13-01").is_err());
    }

    #[test]
    fn timestamp_round_trip() {
        let date = timestamp_to_date("1234567890").unwrap();
        assert_eq!(date_to_timestamp(&date).unwrap(), "1234567890");
    }
}
