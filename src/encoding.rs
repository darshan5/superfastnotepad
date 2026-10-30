//! File encodings: detection on load, conversion on save.
//!
//! Backs the Encoding menu: ANSI (Windows-1252), UTF-8, UTF-8-BOM and
//! UTF-16 BE/LE BOM.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Eol {
    Crlf,
    Lf,
    Cr,
}

impl Eol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Crlf => "\r\n",
            Self::Lf => "\n",
            Self::Cr => "\r",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Crlf => "Windows (CR LF)",
            Self::Lf => "Unix (LF)",
            Self::Cr => "Macintosh (CR)",
        }
    }

    pub fn platform_default() -> Self {
        if cfg!(windows) { Self::Crlf } else { Self::Lf }
    }

    /// Picks the dominant line ending in `text`.
    pub fn detect(text: &str) -> Option<Self> {
        let [crlf, lf, cr] = Self::count(text);
        if crlf + lf + cr == 0 {
            return None;
        }
        Some(if crlf >= lf && crlf >= cr {
            Self::Crlf
        } else if lf >= cr {
            Self::Lf
        } else {
            Self::Cr
        })
    }

    /// Counts line endings in `text`: [CR LF, LF, CR].
    pub fn count(text: &str) -> [usize; 3] {
        let b = text.as_bytes();
        let mut n = [0usize; 3];
        let mut i = 0;
        while let Some(off) = memchr2(b'\r', b'\n', &b[i..]) {
            let j = i + off;
            if b[j] == b'\n' {
                n[1] += 1;
                i = j + 1;
            } else if b.get(j + 1) == Some(&b'\n') {
                n[0] += 1;
                i = j + 2;
            } else {
                n[2] += 1;
                i = j + 1;
            }
        }
        n
    }
}

/// Position of the first `a` or `b` in `hay`.
fn memchr2(a: u8, b: u8, hay: &[u8]) -> Option<usize> {
    hay.iter().position(|&c| c == a || c == b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEncoding {
    /// Windows-1252 (Western European), the most common legacy code page.
    Ansi,
    Utf8,
    Utf8Bom,
    Utf16BeBom,
    Utf16LeBom,
}

impl TextEncoding {
    /// Stable identifier used in session files.
    pub fn key(self) -> String {
        match self {
            Self::Ansi => "ansi".into(),
            Self::Utf8 => "utf8".into(),
            Self::Utf8Bom => "utf8bom".into(),
            Self::Utf16BeBom => "utf16be".into(),
            Self::Utf16LeBom => "utf16le".into(),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "ansi" => Self::Ansi,
            "utf8" => Self::Utf8,
            "utf8bom" => Self::Utf8Bom,
            "utf16be" => Self::Utf16BeBom,
            "utf16le" => Self::Utf16LeBom,
            _ => return None,
        })
    }

    pub fn label(self) -> String {
        match self {
            Self::Ansi => "ANSI".into(),
            Self::Utf8 => "UTF-8".into(),
            Self::Utf8Bom => "UTF-8-BOM".into(),
            Self::Utf16BeBom => "UTF-16 BE BOM".into(),
            Self::Utf16LeBom => "UTF-16 LE BOM".into(),
        }
    }

    /// Decodes raw bytes with this encoding (BOM is stripped if present).
    pub fn decode(self, bytes: &[u8]) -> String {
        match self {
            Self::Utf8 | Self::Utf8Bom => {
                let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
                String::from_utf8_lossy(b).into_owned()
            }
            Self::Utf16LeBom => {
                let b = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes);
                encoding_rs::UTF_16LE
                    .decode_without_bom_handling(b)
                    .0
                    .into_owned()
            }
            Self::Utf16BeBom => {
                let b = bytes.strip_prefix(&[0xFE, 0xFF]).unwrap_or(bytes);
                encoding_rs::UTF_16BE
                    .decode_without_bom_handling(b)
                    .0
                    .into_owned()
            }
            Self::Ansi => encoding_rs::WINDOWS_1252
                .decode_without_bom_handling(bytes)
                .0
                .into_owned(),
        }
    }

    /// Encodes text into bytes, writing a BOM where the encoding requires it.
    pub fn encode(self, text: &str) -> Vec<u8> {
        match self {
            Self::Utf8 => text.as_bytes().to_vec(),
            Self::Utf8Bom => {
                let mut v = vec![0xEF, 0xBB, 0xBF];
                v.extend_from_slice(text.as_bytes());
                v
            }
            Self::Utf16LeBom => {
                let mut v = vec![0xFF, 0xFE];
                for u in text.encode_utf16() {
                    v.extend_from_slice(&u.to_le_bytes());
                }
                v
            }
            Self::Utf16BeBom => {
                let mut v = vec![0xFE, 0xFF];
                for u in text.encode_utf16() {
                    v.extend_from_slice(&u.to_be_bytes());
                }
                v
            }
            Self::Ansi => encoding_rs::WINDOWS_1252.encode(text).0.into_owned(),
        }
    }

    /// Detects the encoding and decodes in one step, taking ownership so valid
    /// UTF-8 needs no copy. Detection only samples the start of the file; if a
    /// file that looked like UTF-8 has invalid bytes further on, it is detected
    /// again from all of its bytes.
    pub fn detect_and_decode(bytes: Vec<u8>) -> (Self, String) {
        let sampled = bytes.len() > DETECT_SAMPLE;
        let enc = Self::detect_in(&bytes[..bytes.len().min(DETECT_SAMPLE)], sampled);
        if enc != Self::Utf8 {
            let text = enc.decode(&bytes);
            return (enc, text);
        }
        match String::from_utf8(bytes) {
            Ok(text) => (enc, text),
            Err(e) => {
                let bytes = e.into_bytes();
                let enc = Self::detect(&bytes);
                let text = enc.decode(&bytes);
                (enc, text)
            }
        }
    }

    /// Detects the encoding of file contents: BOM first, then UTF-8
    /// validity, then UTF-16 heuristics; anything else is ANSI.
    pub fn detect(bytes: &[u8]) -> Self {
        Self::detect_in(bytes, false)
    }

    /// `sampled` means `bytes` is only the start of the file, so a multi-byte
    /// character cut off at the end doesn't count as invalid UTF-8.
    fn detect_in(bytes: &[u8], sampled: bool) -> Self {
        if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            return Self::Utf8Bom;
        }
        if bytes.starts_with(&[0xFF, 0xFE]) {
            return Self::Utf16LeBom;
        }
        if bytes.starts_with(&[0xFE, 0xFF]) {
            return Self::Utf16BeBom;
        }
        match std::str::from_utf8(bytes) {
            Ok(_) => return Self::Utf8,
            Err(e) if sampled && e.error_len().is_none() => return Self::Utf8,
            Err(_) => {}
        }
        // BOM-less UTF-16: lots of NULs on one side.
        if bytes.len() >= 4 {
            let sample = &bytes[..bytes.len().min(4096)];
            let even_nul = sample.iter().step_by(2).filter(|&&b| b == 0).count();
            let odd_nul = sample
                .iter()
                .skip(1)
                .step_by(2)
                .filter(|&&b| b == 0)
                .count();
            let half = sample.len() / 2;
            if odd_nul > half * 6 / 10 && even_nul < half / 10 {
                return Self::Utf16LeBom;
            }
            if even_nul > half * 6 / 10 && odd_nul < half / 10 {
                return Self::Utf16BeBom;
            }
        }
        Self::Ansi
    }
}

/// How much of a file encoding detection looks at.
const DETECT_SAMPLE: usize = 64 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_unicode_forms() {
        let s = "héllo – 世界\r\n";
        for enc in [
            TextEncoding::Utf8,
            TextEncoding::Utf8Bom,
            TextEncoding::Utf16LeBom,
            TextEncoding::Utf16BeBom,
        ] {
            let bytes = enc.encode(s);
            assert_eq!(TextEncoding::detect(&bytes), enc);
            assert_eq!(enc.decode(&bytes), s);
        }
    }

    #[test]
    fn sampled_detection() {
        // Large ASCII file with a UTF-8 character cut by the sample boundary.
        let mut s = "a".repeat(DETECT_SAMPLE - 1);
        s.push_str("é tail");
        let (enc, text) = TextEncoding::detect_and_decode(s.clone().into_bytes());
        assert_eq!(enc, TextEncoding::Utf8);
        assert_eq!(text, s);
        // Looks like UTF-8 in the sample, but Latin-1 bytes come later.
        let mut b = vec![b'a'; DETECT_SAMPLE * 2];
        b.extend_from_slice(b"caf\xe9 au lait");
        let (enc, text) = TextEncoding::detect_and_decode(b);
        assert_eq!(enc, TextEncoding::Ansi);
        assert!(text.ends_with("café au lait"));
        // BOM files still win over sampling.
        let mut b = vec![0xEF, 0xBB, 0xBF];
        b.extend_from_slice("x".repeat(DETECT_SAMPLE * 2).as_bytes());
        assert_eq!(TextEncoding::detect_and_decode(b).0, TextEncoding::Utf8Bom);
    }

    #[test]
    fn detects_ansi() {
        let bytes = b"caf\xe9 au lait";
        assert_eq!(TextEncoding::detect(bytes), TextEncoding::Ansi);
        assert_eq!(TextEncoding::Ansi.decode(bytes), "café au lait");
    }

    #[test]
    fn eol_detection() {
        assert_eq!(Eol::detect("a\r\nb\r\nc\n"), Some(Eol::Crlf));
        assert_eq!(Eol::detect("a\nb\n"), Some(Eol::Lf));
        assert_eq!(Eol::detect("a\rb\r"), Some(Eol::Cr));
        assert_eq!(Eol::detect("abc"), None);
    }
}
