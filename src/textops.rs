//! Pure text transformations behind the Edit menu
//! (Line Operations, Convert Case, Blank Operations, Column Editor...).

use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SortMode {
    LexAsc,
    LexDesc,
    LexCiAsc,
    LexCiDesc,
    LocaleAsc,
    LocaleDesc,
    IntAsc,
    IntDesc,
    DecCommaAsc,
    DecCommaDesc,
    DecDotAsc,
    DecDotDesc,
    LengthAsc,
    LengthDesc,
    Reverse,
    Random,
    /// Largest first, by the first number on each line (`1,234.5`,
    /// `$80`, `-3`); lines without a number go last.
    NumberDesc,
}

fn leading_number(s: &str, decimal: Option<char>) -> Option<f64> {
    let t = s.trim_start();
    let mut out = String::new();
    let mut seen_digit = false;
    let mut seen_dec = false;
    for (i, c) in t.char_indices() {
        if c.is_ascii_digit() {
            out.push(c);
            seen_digit = true;
        } else if (c == '-' || c == '+') && i == 0 {
            out.push(c);
        } else if Some(c) == decimal && !seen_dec {
            out.push('.');
            seen_dec = true;
        } else if decimal == Some(',') && c == '.' && seen_digit {
            // thousands separator in decimal-comma mode
        } else if decimal == Some('.') && c == ',' && seen_digit {
            // thousands separator in decimal-dot mode
        } else {
            break;
        }
    }
    if seen_digit { out.parse().ok() } else { None }
}

/// The first number anywhere in `s`, ignoring thousands commas.
fn first_number(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let start = (0..b.len()).find(|&i| {
        b[i].is_ascii_digit()
            || (matches!(b[i], b'-' | b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit))
    })?;
    let rest = &s[start..];
    // `.5` reads as 0.5.
    if rest.starts_with('.') {
        leading_number(&format!("0{rest}"), Some('.'))
    } else {
        leading_number(rest, Some('.'))
    }
}

fn cmp_num(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Tiny xorshift PRNG so we don't need the `rand` crate.
pub struct Rng(u64);

impl Rng {
    pub fn seeded() -> Self {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64);
        Self(t | 1)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

pub fn sort_lines(lines: &mut [String], mode: SortMode) {
    match mode {
        SortMode::LexAsc => lines.sort(),
        SortMode::LexDesc => lines.sort_by(|a, b| b.cmp(a)),
        SortMode::LexCiAsc | SortMode::LocaleAsc => {
            lines.sort_by_cached_key(|s| s.to_lowercase());
        }
        SortMode::LexCiDesc | SortMode::LocaleDesc => {
            lines.sort_by_cached_key(|s| std::cmp::Reverse(s.to_lowercase()));
        }
        SortMode::IntAsc => {
            lines.sort_by(|a, b| cmp_num(leading_number(a, None), leading_number(b, None)))
        }
        SortMode::IntDesc => {
            lines.sort_by(|a, b| cmp_num(leading_number(b, None), leading_number(a, None)))
        }
        SortMode::DecCommaAsc => lines
            .sort_by(|a, b| cmp_num(leading_number(a, Some(',')), leading_number(b, Some(',')))),
        SortMode::DecCommaDesc => lines
            .sort_by(|a, b| cmp_num(leading_number(b, Some(',')), leading_number(a, Some(',')))),
        SortMode::DecDotAsc => lines
            .sort_by(|a, b| cmp_num(leading_number(a, Some('.')), leading_number(b, Some('.')))),
        SortMode::DecDotDesc => lines
            .sort_by(|a, b| cmp_num(leading_number(b, Some('.')), leading_number(a, Some('.')))),
        SortMode::LengthAsc => lines.sort_by_key(|s| s.chars().count()),
        SortMode::LengthDesc => lines.sort_by_key(|s| std::cmp::Reverse(s.chars().count())),
        SortMode::NumberDesc => {
            lines.sort_by_cached_key(|s| std::cmp::Reverse(NumKey(first_number(s))));
        }
        SortMode::Reverse => lines.reverse(),
        SortMode::Random => {
            let mut rng = Rng::seeded();
            for i in (1..lines.len()).rev() {
                let j = rng.below(i + 1);
                lines.swap(i, j);
            }
        }
    }
}

/// Orders numbers with no number below every number, for sorting keys.
struct NumKey(Option<f64>);

impl PartialEq for NumKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for NumKey {}

impl PartialOrd for NumKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NumKey {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_num(self.0, other.0)
    }
}

pub fn remove_duplicate_lines(lines: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    lines
        .into_iter()
        .filter(|l| seen.insert(l.clone()))
        .collect()
}

pub fn remove_consecutive_duplicates(mut lines: Vec<String>) -> Vec<String> {
    lines.dedup();
    lines
}

pub fn remove_empty_lines(lines: Vec<String>, include_blank: bool) -> Vec<String> {
    lines
        .into_iter()
        .filter(|l| {
            if include_blank {
                !l.trim().is_empty()
            } else {
                !l.is_empty()
            }
        })
        .collect()
}

pub fn trim_trailing(s: &str) -> String {
    s.trim_end_matches([' ', '\t']).to_string()
}

pub fn trim_leading(s: &str) -> String {
    s.trim_start_matches([' ', '\t']).to_string()
}

pub fn trim_both(s: &str) -> String {
    s.trim_matches([' ', '\t']).to_string()
}

pub fn tabs_to_spaces(line: &str, tab: usize) -> String {
    let tab = tab.max(1);
    let mut out = String::with_capacity(line.len());
    let mut col = 0;
    for c in line.chars() {
        if c == '\t' {
            let n = tab - col % tab;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}

/// Converts runs of spaces that end on tab stops to tabs.
pub fn spaces_to_tabs(line: &str, tab: usize, leading_only: bool) -> String {
    let tab = tab.max(1);
    let expanded = tabs_to_spaces(line, tab);
    let chars: Vec<char> = expanded.chars().collect();
    let lead_end = chars.iter().position(|c| *c != ' ').unwrap_or(chars.len());
    let mut out = String::with_capacity(line.len());
    let mut col = 0;
    let mut pending = 0usize;
    for (i, &c) in chars.iter().enumerate() {
        let convertible = !leading_only || i < lead_end;
        if c == ' ' && convertible {
            pending += 1;
            col += 1;
            if col % tab == 0 {
                if pending > 1 {
                    out.push('\t');
                } else {
                    out.push(' ');
                }
                pending = 0;
            }
        } else {
            out.extend(std::iter::repeat_n(' ', pending));
            pending = 0;
            out.push(c);
            col += 1;
        }
    }
    out.extend(std::iter::repeat_n(' ', pending));
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CaseMode {
    Upper,
    Lower,
    ProperForce,
    ProperBlend,
    SentenceForce,
    SentenceBlend,
    Invert,
    Random,
}

pub fn convert_case(s: &str, mode: CaseMode) -> String {
    match mode {
        CaseMode::Upper => s.to_uppercase(),
        CaseMode::Lower => s.to_lowercase(),
        CaseMode::Invert => s
            .chars()
            .flat_map(|c| {
                if c.is_uppercase() {
                    c.to_lowercase().collect::<Vec<_>>()
                } else {
                    c.to_uppercase().collect::<Vec<_>>()
                }
            })
            .collect(),
        CaseMode::Random => {
            let mut rng = Rng::seeded();
            s.chars()
                .flat_map(|c| {
                    if rng.next() & 1 == 0 {
                        c.to_uppercase().collect::<Vec<_>>()
                    } else {
                        c.to_lowercase().collect::<Vec<_>>()
                    }
                })
                .collect()
        }
        CaseMode::ProperForce | CaseMode::ProperBlend => {
            let force = mode == CaseMode::ProperForce;
            let mut out = String::with_capacity(s.len());
            let mut at_word_start = true;
            let mut prev: Option<char> = None;
            for c in s.chars() {
                if c.is_alphanumeric() || (c == '\'' && prev.is_some_and(char::is_alphanumeric)) {
                    if at_word_start {
                        out.extend(c.to_uppercase());
                    } else if force {
                        out.extend(c.to_lowercase());
                    } else {
                        out.push(c);
                    }
                    at_word_start = false;
                } else {
                    out.push(c);
                    at_word_start = true;
                }
                prev = Some(c);
            }
            out
        }
        CaseMode::SentenceForce | CaseMode::SentenceBlend => {
            let force = mode == CaseMode::SentenceForce;
            let mut out = String::with_capacity(s.len());
            let mut new_sentence = true;
            for c in s.chars() {
                if c.is_alphabetic() {
                    if new_sentence {
                        out.extend(c.to_uppercase());
                        new_sentence = false;
                    } else if force {
                        out.extend(c.to_lowercase());
                    } else {
                        out.push(c);
                    }
                } else {
                    if matches!(c, '.' | '!' | '?' | '\n' | '\r') {
                        new_sentence = true;
                    }
                    out.push(c);
                }
            }
            out
        }
    }
}

/// Splits a line into pieces no longer than `width` columns, at spaces when possible.
pub fn split_line(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in line.split(' ') {
        let wl = word.chars().count();
        let cl = cur.chars().count();
        if cl > 0 && cl + 1 + wl > width {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    out.push(cur);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumFormat {
    Dec,
    Hex,
    Oct,
    Bin,
}

/// Generates the Column Editor number sequence.
pub fn number_sequence(
    count: usize,
    initial: i64,
    increment: i64,
    repeat: usize,
    format: NumFormat,
    leading: LeadingMode,
    uppercase_hex: bool,
) -> Vec<String> {
    let repeat = repeat.max(1);
    let raw: Vec<i64> = (0..count)
        .map(|i| initial + increment * i64::try_from(i / repeat).unwrap_or(0))
        .collect();
    let fmt = |n: i64| -> String {
        let neg = n < 0;
        let a = n.unsigned_abs();
        let body = match format {
            NumFormat::Dec => a.to_string(),
            NumFormat::Hex => {
                if uppercase_hex {
                    format!("{a:X}")
                } else {
                    format!("{a:x}")
                }
            }
            NumFormat::Oct => format!("{a:o}"),
            NumFormat::Bin => format!("{a:b}"),
        };
        if neg { format!("-{body}") } else { body }
    };
    let strs: Vec<String> = raw.iter().map(|&n| fmt(n)).collect();
    let width = strs.iter().map(String::len).max().unwrap_or(0);
    strs.into_iter()
        .map(|s| match leading {
            LeadingMode::None => s,
            LeadingMode::Zeros => {
                if let Some(rest) = s.strip_prefix('-') {
                    format!("-{}{rest}", "0".repeat(width.saturating_sub(s.len())))
                } else {
                    format!("{}{s}", "0".repeat(width - s.len()))
                }
            }
            LeadingMode::Spaces => format!("{s:>width$}"),
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeadingMode {
    None,
    Zeros,
    Spaces,
}

/// Toggles a line comment on every line. If all non-blank lines are
/// commented, uncomments; otherwise comments (at the minimal indentation,
/// like "Toggle Single Line Comment").
pub fn toggle_line_comment(lines: &[String], prefix: &str) -> Vec<String> {
    let non_blank: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    let all_commented =
        !non_blank.is_empty() && non_blank.iter().all(|l| l.trim_start().starts_with(prefix));
    if all_commented {
        lines.iter().map(|l| uncomment_line(l, prefix)).collect()
    } else {
        comment_lines(lines, prefix)
    }
}

pub fn comment_lines(lines: &[String], prefix: &str) -> Vec<String> {
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                l.clone()
            } else {
                let (a, b) = l.split_at(indent.min(l.len()));
                format!("{a}{prefix} {b}")
            }
        })
        .collect()
}

pub fn uncomment_line(l: &str, prefix: &str) -> String {
    let ws = l.len() - l.trim_start().len();
    let rest = &l[ws..];
    rest.strip_prefix(prefix).map_or_else(
        || l.to_string(),
        |r| {
            let r = r.strip_prefix(' ').unwrap_or(r);
            format!("{}{r}", &l[..ws])
        },
    )
}

/// Characters in a "Find characters in range" category.
pub fn char_in_range(c: char, range: CharRange) -> bool {
    let v = c as u32;
    match range {
        CharRange::NonAscii => v > 127,
        CharRange::Ascii => v <= 127,
        CharRange::Custom(a, b) => (a..=b).contains(&v),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharRange {
    NonAscii,
    Ascii,
    Custom(u32, u32),
}

pub fn hex_digest(algo: HashAlgo, data: &[u8]) -> String {
    use md5::Digest as _;
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
    match algo {
        HashAlgo::Md5 => hex(&md5::Md5::digest(data)),
        HashAlgo::Sha1 => hex(&sha1::Sha1::digest(data)),
        HashAlgo::Sha256 => hex(&sha2::Sha256::digest(data)),
        HashAlgo::Sha512 => hex(&sha2::Sha512::digest(data)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HashAlgo {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl HashAlgo {
    pub fn name(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha1 => "SHA-1",
            Self::Sha256 => "SHA-256",
            Self::Sha512 => "SHA-512",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn sorts() {
        let mut l = v(&["b", "B", "a", "c"]);
        sort_lines(&mut l, SortMode::LexAsc);
        assert_eq!(l, v(&["B", "a", "b", "c"]));
        sort_lines(&mut l, SortMode::LexCiDesc);
        assert_eq!(l[0], "c");
        let mut n = v(&["10", "9", "-1", "100"]);
        sort_lines(&mut n, SortMode::IntAsc);
        assert_eq!(n, v(&["-1", "9", "10", "100"]));
        let mut d = v(&["1,5", "1,25", "0,9"]);
        sort_lines(&mut d, SortMode::DecCommaAsc);
        assert_eq!(d, v(&["0,9", "1,25", "1,5"]));
        let mut len = v(&["ccc", "a", "bb"]);
        sort_lines(&mut len, SortMode::LengthDesc);
        assert_eq!(len, v(&["ccc", "bb", "a"]));
    }

    #[test]
    fn sorts_by_number_largest_first() {
        let mut l = v(&[
            "no number here",
            "9 apples",
            "total: 1,234.50",
            "$80",
            "-3 degrees",
            "item 12,000",
            ".5 cup",
        ]);
        sort_lines(&mut l, SortMode::NumberDesc);
        assert_eq!(
            l,
            v(&[
                "item 12,000",
                "total: 1,234.50",
                "$80",
                "9 apples",
                ".5 cup",
                "-3 degrees",
                "no number here",
            ])
        );
        let mut a = v(&["banana", "Apple", "cherry"]);
        sort_lines(&mut a, SortMode::LexCiAsc);
        assert_eq!(a, v(&["Apple", "banana", "cherry"]));
    }

    #[test]
    fn dedup_and_empty() {
        assert_eq!(
            remove_duplicate_lines(v(&["a", "b", "a", "c", "b"])),
            v(&["a", "b", "c"])
        );
        assert_eq!(
            remove_consecutive_duplicates(v(&["a", "a", "b", "a"])),
            v(&["a", "b", "a"])
        );
        assert_eq!(
            remove_empty_lines(v(&["a", "", "  ", "b"]), false),
            v(&["a", "  ", "b"])
        );
        assert_eq!(
            remove_empty_lines(v(&["a", "", "  ", "b"]), true),
            v(&["a", "b"])
        );
    }

    #[test]
    fn tabs_spaces() {
        assert_eq!(tabs_to_spaces("\ta\tb", 4), "    a   b");
        assert_eq!(spaces_to_tabs("        x", 4, true), "\t\tx");
        assert_eq!(spaces_to_tabs("a       b", 4, false), "a\t\tb");
        assert_eq!(spaces_to_tabs("a       b", 4, true), "a       b");
    }

    #[test]
    fn cases() {
        assert_eq!(
            convert_case("hello wORLD", CaseMode::ProperForce),
            "Hello World"
        );
        assert_eq!(
            convert_case("hello wORLD", CaseMode::ProperBlend),
            "Hello WORLD"
        );
        assert_eq!(
            convert_case("abc. def? ghi", CaseMode::SentenceForce),
            "Abc. Def? Ghi"
        );
        assert_eq!(convert_case("AbC", CaseMode::Invert), "aBc");
    }

    #[test]
    fn comments() {
        let l = v(&["  a", "  b", ""]);
        let c = toggle_line_comment(&l, "//");
        assert_eq!(c, v(&["  // a", "  // b", ""]));
        assert_eq!(toggle_line_comment(&c, "//"), l);
    }

    #[test]
    fn number_sequences() {
        assert_eq!(
            number_sequence(4, 8, 1, 1, NumFormat::Dec, LeadingMode::Zeros, false),
            v(&["08", "09", "10", "11"])
        );
        assert_eq!(
            number_sequence(4, 1, 2, 2, NumFormat::Hex, LeadingMode::None, true),
            v(&["1", "1", "3", "3"])
        );
    }

    #[test]
    fn hashes() {
        assert_eq!(
            hex_digest(HashAlgo::Md5, b"abc"),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            hex_digest(HashAlgo::Sha256, b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn split() {
        assert_eq!(split_line("aa bb cc dd", 5), v(&["aa bb", "cc dd"]));
    }
}
