//! Text tools: MIME (Base64, URL, Quoted-printable) and
//! JSON / XML formatting.
//!
//! Both printers are configured by `FormatOptions` (indent width, tabs,
//! print width, object wrapping, XML whitespace handling, self-closing tag
//! spacing, attribute sorting and quoting, one attribute per line, and where
//! the closing bracket goes).

use serde::{Deserialize, Serialize};

// ------------------------------------------------------------------ options

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum XmlWhitespace {
    /// Keep all whitespace inside elements that contain text (the default).
    #[default]
    Strict,
    /// Re-indent everything; collapse blank lines to at most one.
    Ignore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum XmlQuotes {
    #[default]
    Preserve,
    Double,
    Single,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FormatOptions {
    pub tab_width: usize,
    pub use_tabs: bool,
    pub print_width: usize,
    /// `object_wrap_preserve`: keep an object on one line if it
    /// was written that way and fits.
    pub json_object_wrap_preserve: bool,
    pub xml_whitespace: XmlWhitespace,
    pub xml_self_closing_space: bool,
    pub xml_sort_attributes: bool,
    pub xml_quotes: XmlQuotes,
    pub single_attribute_per_line: bool,
    pub bracket_same_line: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            tab_width: 2,
            use_tabs: false,
            print_width: 80,
            json_object_wrap_preserve: true,
            xml_whitespace: XmlWhitespace::Strict,
            xml_self_closing_space: true,
            xml_sort_attributes: false,
            xml_quotes: XmlQuotes::Preserve,
            single_attribute_per_line: false,
            bracket_same_line: true,
        }
    }
}

impl FormatOptions {
    fn indent(&self, level: usize) -> String {
        if self.use_tabs {
            "\t".repeat(level)
        } else {
            " ".repeat(self.tab_width * level)
        }
    }

    fn indent_width(&self, level: usize) -> usize {
        level * if self.use_tabs { 4 } else { self.tab_width }
    }
}

/// Byte offset → "line L, column C" (1-based).
fn location(text: &str, pos: usize) -> String {
    let pos = pos.min(text.len());
    let before = &text[..pos];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    format!("line {line}, column {col}")
}

// ------------------------------------------------------------------ MIME

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8], url_safe: bool) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let idx = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (i, &x) in idx.iter().enumerate() {
            if i <= chunk.len() {
                let mut c = B64[x as usize] as char;
                if url_safe {
                    c = match c {
                        '+' => '-',
                        '/' => '_',
                        c => c,
                    };
                }
                out.push(c);
            } else if !url_safe {
                out.push('=');
            }
        }
    }
    out
}

/// Decodes standard or URL-safe Base64; whitespace is ignored, padding optional.
pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    let mut padding = false;
    for (i, c) in text.char_indices() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' | '-' => 62,
            '/' | '_' => 63,
            '=' => {
                padding = true;
                continue;
            }
            c if c.is_whitespace() => continue,
            _ => {
                return Err(format!(
                    "Invalid Base64 character '{c}' at {}",
                    location(text, i)
                ));
            }
        };
        if padding {
            return Err(format!(
                "Unexpected data after padding at {}",
                location(text, i)
            ));
        }
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    if bits >= 6 {
        return Err("Truncated Base64 input".into());
    }
    Ok(out)
}

/// RFC 3986 percent-encoding (unreserved characters are kept).
pub fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn url_decode(text: &str, plus_as_space: bool) -> Result<String, String> {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = text
                    .get(i + 1..i + 3)
                    .ok_or_else(|| format!("Incomplete escape at {}", location(text, i)))?;
                let v = u8::from_str_radix(hex, 16)
                    .map_err(|_| format!("Invalid escape '%{hex}' at {}", location(text, i)))?;
                out.push(v);
                i += 3;
            }
            b'+' if plus_as_space => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| "Decoded bytes are not valid UTF-8".into())
}

/// RFC 2045 quoted-printable encoding (76-column soft line breaks).
pub fn qp_encode(text: &str) -> String {
    let mut out = String::new();
    for (li, line) in text.split('\n').enumerate() {
        if li > 0 {
            out.push('\n');
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        let bytes = line.as_bytes();
        let mut col = 0;
        for (i, &b) in bytes.iter().enumerate() {
            let last = i + 1 == bytes.len();
            let piece =
                if (b == b' ' || b == b'\t') && !last || (33..=126).contains(&b) && b != b'=' {
                    (b as char).to_string()
                } else {
                    format!("={b:02X}")
                };
            if col + piece.len() > 75 {
                out.push_str("=\n");
                col = 0;
            }
            col += piece.len();
            out.push_str(&piece);
        }
    }
    out
}

pub fn qp_decode(text: &str) -> Result<String, String> {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' {
            // Soft line break.
            if b.get(i + 1) == Some(&b'\n') {
                i += 2;
                continue;
            }
            if b.get(i + 1) == Some(&b'\r') && b.get(i + 2) == Some(&b'\n') {
                i += 3;
                continue;
            }
            let hex = text
                .get(i + 1..i + 3)
                .ok_or_else(|| format!("Incomplete escape at {}", location(text, i)))?;
            out.push(
                u8::from_str_radix(hex, 16)
                    .map_err(|_| format!("Invalid escape '={hex}' at {}", location(text, i)))?,
            );
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "Decoded bytes are not valid UTF-8".into())
}

// ------------------------------------------------------------------ JSON

#[derive(Debug)]
enum J {
    Obj {
        members: Vec<(Vec<String>, String, J)>,
        trailing: Vec<String>,
        multiline: bool,
    },
    Arr {
        items: Vec<(Vec<String>, J)>,
        trailing: Vec<String>,
    },
    Scalar(String),
}

struct JParser<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

impl JParser<'_> {
    fn err<T>(&self, msg: &str) -> Result<T, String> {
        Err(format!("JSON error: {msg} at {}", location(self.s, self.i)))
    }

    /// Skips whitespace, collecting comments.
    fn ws(&mut self, comments: &mut Vec<String>) -> Result<bool, String> {
        let mut newline = false;
        loop {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                newline |= self.b[self.i] == b'\n';
                self.i += 1;
            }
            if self.s[self.i..].starts_with("//") {
                let e = self.s[self.i..]
                    .find('\n')
                    .map_or(self.s.len(), |p| self.i + p);
                comments.push(self.s[self.i..e].trim_end().to_string());
                self.i = e;
            } else if self.s[self.i..].starts_with("/*") {
                let Some(e) = self.s[self.i + 2..].find("*/") else {
                    return self.err("unterminated comment");
                };
                let end = self.i + 2 + e + 2;
                comments.push(self.s[self.i..end].to_string());
                self.i = end;
            } else {
                return Ok(newline);
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        let start = self.i;
        let q = self.b[self.i];
        self.i += 1;
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'\\' => self.i += 2,
                c if c == q => {
                    self.i += 1;
                    return Ok(self.s[start..self.i].to_string());
                }
                b'\n' => return self.err("unterminated string"),
                _ => self.i += 1,
            }
        }
        self.err("unterminated string")
    }

    fn value(&mut self) -> Result<J, String> {
        let Some(&c) = self.b.get(self.i) else {
            return self.err("unexpected end of input");
        };
        match c {
            b'{' => {
                self.i += 1;
                let mut members = Vec::new();
                let mut comments = Vec::new();
                let multiline = self.ws(&mut comments)?;
                loop {
                    if self.b.get(self.i) == Some(&b'}') {
                        self.i += 1;
                        return Ok(J::Obj {
                            members,
                            trailing: comments,
                            multiline,
                        });
                    }
                    let key = match self.b.get(self.i) {
                        Some(b'"' | b'\'') => self.string()?,
                        Some(c) if c.is_ascii_alphabetic() || *c == b'_' || *c == b'$' => {
                            // JSON5 unquoted key.
                            let s = self.i;
                            while self.i < self.b.len()
                                && (self.b[self.i].is_ascii_alphanumeric()
                                    || matches!(self.b[self.i], b'_' | b'$'))
                            {
                                self.i += 1;
                            }
                            self.s[s..self.i].to_string()
                        }
                        _ => return self.err("expected a property name or '}'"),
                    };
                    self.ws(&mut comments)?;
                    if self.b.get(self.i) != Some(&b':') {
                        return self.err("expected ':'");
                    }
                    self.i += 1;
                    self.ws(&mut comments)?;
                    let v = self.value()?;
                    members.push((std::mem::take(&mut comments), key, v));
                    self.ws(&mut comments)?;
                    match self.b.get(self.i) {
                        Some(b',') => {
                            self.i += 1;
                            self.ws(&mut comments)?;
                        }
                        Some(b'}') => {}
                        _ => return self.err("expected ',' or '}'"),
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                let mut comments = Vec::new();
                self.ws(&mut comments)?;
                loop {
                    if self.b.get(self.i) == Some(&b']') {
                        self.i += 1;
                        return Ok(J::Arr {
                            items,
                            trailing: comments,
                        });
                    }
                    let v = self.value()?;
                    items.push((std::mem::take(&mut comments), v));
                    self.ws(&mut comments)?;
                    match self.b.get(self.i) {
                        Some(b',') => {
                            self.i += 1;
                            self.ws(&mut comments)?;
                        }
                        Some(b']') => {}
                        _ => return self.err("expected ',' or ']'"),
                    }
                }
            }
            b'"' | b'\'' => Ok(J::Scalar(self.string()?)),
            _ => {
                let s = self.i;
                while self.i < self.b.len()
                    && (self.b[self.i].is_ascii_alphanumeric()
                        || matches!(self.b[self.i], b'-' | b'+' | b'.' | b'_'))
                {
                    self.i += 1;
                }
                let tok = &self.s[s..self.i];
                let valid = matches!(
                    tok,
                    "true" | "false" | "null" | "Infinity" | "-Infinity" | "NaN"
                ) || (!tok.is_empty() && tok.parse::<f64>().is_ok())
                    || tok.starts_with("0x")
                    || tok.starts_with("-0x");
                if valid {
                    Ok(J::Scalar(tok.to_string()))
                } else {
                    self.i = s;
                    self.err("unexpected token")
                }
            }
        }
    }
}

fn parse_json(text: &str) -> Result<(Vec<String>, J, Vec<String>), String> {
    let text_trim = text.trim_start_matches('\u{feff}');
    let mut p = JParser {
        s: text_trim,
        b: text_trim.as_bytes(),
        i: 0,
    };
    let mut lead = Vec::new();
    p.ws(&mut lead)?;
    let v = p.value()?;
    let mut tail = Vec::new();
    p.ws(&mut tail)?;
    if p.i < p.b.len() {
        return p.err("unexpected data after the JSON value");
    }
    Ok((lead, v, tail))
}

/// Single-line rendering, or None if it contains comments.
fn json_flat(v: &J) -> Option<String> {
    match v {
        J::Scalar(s) => Some(s.clone()),
        J::Arr { items, trailing } => {
            if !trailing.is_empty() || items.iter().any(|(c, _)| !c.is_empty()) {
                return None;
            }
            let parts: Option<Vec<String>> = items.iter().map(|(_, v)| json_flat(v)).collect();
            Some(format!("[{}]", parts?.join(", ")))
        }
        J::Obj {
            members, trailing, ..
        } => {
            if members.is_empty() && trailing.is_empty() {
                return Some("{}".into());
            }
            if !trailing.is_empty() || members.iter().any(|(c, _, _)| !c.is_empty()) {
                return None;
            }
            let parts: Option<Vec<String>> = members
                .iter()
                .map(|(_, k, v)| json_flat(v).map(|f| format!("{k}: {f}")))
                .collect();
            Some(format!("{{ {} }}", parts?.join(", ")))
        }
    }
}

fn json_print(v: &J, level: usize, col: usize, o: &FormatOptions, out: &mut String) {
    let width_left = o.print_width.saturating_sub(col);
    match v {
        J::Scalar(s) => out.push_str(s),
        J::Arr { items, trailing } => {
            if items.is_empty() && trailing.is_empty() {
                out.push_str("[]");
                return;
            }
            // Arrays of scalars stay on one line when they fit .
            let all_scalar = items.iter().all(|(_, v)| matches!(v, J::Scalar(_)));
            if all_scalar
                && let Some(flat) = json_flat(v)
                && flat.len() <= width_left
            {
                out.push_str(&flat);
                return;
            }
            out.push_str("[\n");
            for (i, (comments, item)) in items.iter().enumerate() {
                for c in comments {
                    out.push_str(&o.indent(level + 1));
                    out.push_str(c);
                    out.push('\n');
                }
                out.push_str(&o.indent(level + 1));
                json_print(item, level + 1, o.indent_width(level + 1), o, out);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            for c in trailing {
                out.push_str(&o.indent(level + 1));
                out.push_str(c);
                out.push('\n');
            }
            out.push_str(&o.indent(level));
            out.push(']');
        }
        J::Obj {
            members,
            trailing,
            multiline,
        } => {
            if members.is_empty() && trailing.is_empty() {
                out.push_str("{}");
                return;
            }
            if o.json_object_wrap_preserve
                && !multiline
                && let Some(flat) = json_flat(v)
                && flat.len() <= width_left
            {
                out.push_str(&flat);
                return;
            }
            out.push_str("{\n");
            for (i, (comments, key, val)) in members.iter().enumerate() {
                for c in comments {
                    out.push_str(&o.indent(level + 1));
                    out.push_str(c);
                    out.push('\n');
                }
                out.push_str(&o.indent(level + 1));
                out.push_str(key);
                out.push_str(": ");
                let col = o.indent_width(level + 1) + key.chars().count() + 2;
                json_print(val, level + 1, col, o, out);
                if i + 1 < members.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            for c in trailing {
                out.push_str(&o.indent(level + 1));
                out.push_str(c);
                out.push('\n');
            }
            out.push_str(&o.indent(level));
            out.push('}');
        }
    }
}

/// Pretty-prints JSON / JSONC / JSON5 (comments kept, key order and number
/// literals preserved exactly, trailing commas removed).
pub fn json_format(text: &str, o: &FormatOptions) -> Result<String, String> {
    let (lead, v, tail) = parse_json(text)?;
    let mut out = String::new();
    for c in &lead {
        out.push_str(c);
        out.push('\n');
    }
    json_print(&v, 0, 0, o, &mut out);
    for c in &tail {
        out.push('\n');
        out.push_str(c);
    }
    out.push('\n');
    Ok(out)
}

fn json_min(v: &J, out: &mut String) {
    match v {
        J::Scalar(s) => out.push_str(s),
        J::Arr { items, .. } => {
            out.push('[');
            for (i, (_, it)) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json_min(it, out);
            }
            out.push(']');
        }
        J::Obj { members, .. } => {
            out.push('{');
            for (i, (_, k, val)) in members.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(k);
                out.push(':');
                json_min(val, out);
            }
            out.push('}');
        }
    }
}

/// Minifies JSON (comments are dropped).
pub fn json_minify(text: &str) -> Result<String, String> {
    let (_, v, _) = parse_json(text)?;
    let mut out = String::new();
    json_min(&v, &mut out);
    Ok(out)
}

pub fn json_validate(text: &str) -> Result<(), String> {
    parse_json(text).map(|_| ())
}

// ------------------------------------------------------------------ XML

#[derive(Debug, Clone)]
enum X {
    Elem {
        name: String,
        attrs: Vec<(String, String, char)>,
        children: Vec<X>,
        self_closing: bool,
    },
    Text(String),
    Comment(String),
    CData(String),
    /// `<?...?>` and `<!DOCTYPE ...>` kept verbatim.
    Raw(String),
}

struct XParser<'a> {
    s: &'a str,
    i: usize,
}

impl XParser<'_> {
    fn err<T>(&self, msg: &str) -> Result<T, String> {
        Err(format!("XML error: {msg} at {}", location(self.s, self.i)))
    }

    fn rest(&self) -> &str {
        &self.s[self.i..]
    }

    fn take_until(&mut self, end: &str, what: &str) -> Result<String, String> {
        match self.rest().find(end) {
            Some(p) => {
                let s = self.s[self.i..self.i + p + end.len()].to_string();
                self.i += p + end.len();
                Ok(s)
            }
            None => self.err(&format!("unterminated {what}")),
        }
    }

    fn name(&mut self) -> String {
        let s = self.i;
        while let Some(c) = self.rest().chars().next() {
            if c.is_whitespace() || matches!(c, '>' | '/' | '=' | '<') {
                break;
            }
            self.i += c.len_utf8();
        }
        self.s[s..self.i].to_string()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.rest().chars().next() {
            if !c.is_whitespace() {
                break;
            }
            self.i += c.len_utf8();
        }
    }

    fn nodes(&mut self, parent: Option<&str>) -> Result<Vec<X>, String> {
        let mut out = Vec::new();
        while self.i < self.s.len() {
            let r = self.rest();
            if r.starts_with("</") {
                let Some(p) = parent else {
                    return self.err("unexpected closing tag");
                };
                let at = self.i;
                self.i += 2;
                let n = self.name();
                self.skip_ws();
                if !self.rest().starts_with('>') {
                    return self.err("expected '>'");
                }
                self.i += 1;
                if n != p {
                    self.i = at;
                    return self.err(&format!("closing tag </{n}> does not match <{p}>"));
                }
                return Ok(out);
            } else if r.starts_with("<!--") {
                out.push(X::Comment(self.take_until("-->", "comment")?));
            } else if r.starts_with("<![CDATA[") {
                out.push(X::CData(self.take_until("]]>", "CDATA section")?));
            } else if r.starts_with("<?") {
                out.push(X::Raw(self.take_until("?>", "processing instruction")?));
            } else if r.starts_with("<!") {
                // DOCTYPE (may contain an internal subset in brackets).
                let s = self.i;
                let mut depth = 0;
                for (off, c) in r.char_indices() {
                    match c {
                        '[' => depth += 1,
                        ']' => depth -= 1,
                        '>' if depth == 0 => {
                            self.i = s + off + 1;
                            break;
                        }
                        _ => {}
                    }
                }
                if self.i == s {
                    return self.err("unterminated declaration");
                }
                out.push(X::Raw(self.s[s..self.i].to_string()));
            } else if r.starts_with('<') {
                self.i += 1;
                let name = self.name();
                if name.is_empty() {
                    return self.err("expected element name");
                }
                let mut attrs = Vec::new();
                loop {
                    self.skip_ws();
                    let r = self.rest();
                    if r.starts_with("/>") {
                        self.i += 2;
                        out.push(X::Elem {
                            name,
                            attrs,
                            children: Vec::new(),
                            self_closing: true,
                        });
                        break;
                    }
                    if r.starts_with('>') {
                        self.i += 1;
                        let children = self.nodes(Some(&name))?;
                        out.push(X::Elem {
                            name,
                            attrs,
                            children,
                            self_closing: false,
                        });
                        break;
                    }
                    if r.is_empty() {
                        return self.err(&format!("unterminated tag <{name}>"));
                    }
                    let an = self.name();
                    if an.is_empty() {
                        return self.err("invalid attribute");
                    }
                    self.skip_ws();
                    if !self.rest().starts_with('=') {
                        return self.err(&format!("expected '=' after attribute {an}"));
                    }
                    self.i += 1;
                    self.skip_ws();
                    let q = self.rest().chars().next().unwrap_or(' ');
                    if q != '"' && q != '\'' {
                        return self.err("expected quoted attribute value");
                    }
                    self.i += 1;
                    let Some(e) = self.rest().find(q) else {
                        return self.err("unterminated attribute value");
                    };
                    let v = self.s[self.i..self.i + e].to_string();
                    self.i += e + 1;
                    attrs.push((an, v, q));
                }
            } else {
                let e = r.find('<').unwrap_or(r.len());
                out.push(X::Text(r[..e].to_string()));
                self.i += e;
            }
        }
        if let Some(p) = parent {
            return self.err(&format!("missing closing tag </{p}>"));
        }
        Ok(out)
    }
}

fn parse_xml(text: &str) -> Result<Vec<X>, String> {
    let t = text.trim_start_matches('\u{feff}');
    let mut p = XParser { s: t, i: 0 };
    let nodes = p.nodes(None)?;
    if !nodes.iter().any(|n| matches!(n, X::Elem { .. })) {
        return Err("XML error: no root element".into());
    }
    Ok(nodes)
}

/// Verbatim serialisation (used for whitespace-sensitive content).
fn xml_raw(n: &X, o: &FormatOptions, out: &mut String) {
    match n {
        X::Text(t) => out.push_str(t),
        X::Comment(t) | X::CData(t) | X::Raw(t) => out.push_str(t),
        X::Elem {
            name,
            attrs,
            children,
            self_closing,
        } => {
            out.push('<');
            out.push_str(name);
            for a in xml_attrs(attrs, o) {
                out.push(' ');
                out.push_str(&a);
            }
            if *self_closing {
                out.push_str(if o.xml_self_closing_space {
                    " />"
                } else {
                    "/>"
                });
            } else {
                out.push('>');
                for c in children {
                    xml_raw(c, o, out);
                }
                out.push_str(&format!("</{name}>"));
            }
        }
    }
}

fn xml_attrs(attrs: &[(String, String, char)], o: &FormatOptions) -> Vec<String> {
    let mut v: Vec<&(String, String, char)> = attrs.iter().collect();
    if o.xml_sort_attributes {
        v.sort_by(|a, b| {
            let ax = a.0 == "xmlns" || a.0.starts_with("xmlns:");
            let bx = b.0 == "xmlns" || b.0.starts_with("xmlns:");
            bx.cmp(&ax).then_with(|| a.0.cmp(&b.0))
        });
    }
    v.iter()
        .map(|(n, val, q)| {
            let want = match o.xml_quotes {
                XmlQuotes::Preserve => *q,
                XmlQuotes::Double => '"',
                XmlQuotes::Single => '\'',
            };
            // Never switch to a quote that appears inside the value.
            let quote = if val.contains(want) { *q } else { want };
            format!("{n}={quote}{val}{quote}")
        })
        .collect()
}

fn has_text(children: &[X]) -> bool {
    children
        .iter()
        .any(|c| matches!(c, X::Text(t) if !t.trim().is_empty()) || matches!(c, X::CData(_)))
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn xml_open_tag(
    name: &str,
    attrs: &[(String, String, char)],
    self_closing: bool,
    level: usize,
    o: &FormatOptions,
) -> String {
    let a = xml_attrs(attrs, o);
    let close = if self_closing {
        if o.xml_self_closing_space {
            " />"
        } else {
            "/>"
        }
    } else {
        ">"
    };
    let flat = if a.is_empty() {
        format!(
            "<{name}{}",
            if self_closing {
                close.trim_start().to_string()
            } else {
                close.to_string()
            }
        )
    } else {
        format!("<{name} {}{close}", a.join(" "))
    };
    let fits = o.indent_width(level) + flat.chars().count() <= o.print_width;
    if a.len() <= 1 || (fits && !o.single_attribute_per_line) {
        if a.is_empty() && self_closing {
            return format!("<{name}{close}");
        }
        return flat;
    }
    let mut s = format!("<{name}");
    for attr in &a {
        s.push('\n');
        s.push_str(&o.indent(level + 1));
        s.push_str(attr);
    }
    if o.bracket_same_line {
        s.push_str(close);
    } else {
        s.push('\n');
        s.push_str(&o.indent(level));
        s.push_str(close.trim_start());
    }
    s
}

fn xml_print(nodes: &[X], level: usize, o: &FormatOptions, out: &mut Vec<String>) {
    let mut prev_was_node = false;
    for n in nodes {
        let ind = o.indent(level);
        match n {
            X::Text(t) => {
                // Keep at most one blank line between nodes.
                if t.trim().is_empty() {
                    if prev_was_node && t.matches('\n').count() >= 2 {
                        out.push(String::new());
                    }
                    continue;
                }
                out.push(format!("{ind}{}", collapse_ws(t)));
            }
            X::Comment(t) | X::Raw(t) => {
                let mut lines = t.lines();
                if let Some(first) = lines.next() {
                    out.push(format!("{ind}{first}"));
                }
                for l in lines {
                    out.push(l.to_string());
                }
            }
            X::CData(t) => out.push(format!("{ind}{t}")),
            X::Elem {
                name,
                attrs,
                children,
                self_closing,
            } => {
                let open = xml_open_tag(name, attrs, *self_closing, level, o);
                if *self_closing {
                    out.push(format!("{ind}{open}"));
                } else if children
                    .iter()
                    .all(|c| matches!(c, X::Text(t) if t.trim().is_empty()))
                {
                    out.push(format!("{ind}{open}</{name}>"));
                } else if has_text(children) && o.xml_whitespace == XmlWhitespace::Strict {
                    // Whitespace is significant: print content verbatim.
                    let mut inner = String::new();
                    for c in children {
                        xml_raw(c, o, &mut inner);
                    }
                    out.push(format!("{ind}{open}{inner}</{name}>"));
                } else if children.len() == 1
                    && let X::Text(t) = &children[0]
                    && !t.contains('\n')
                    && o.indent_width(level) + open.len() + collapse_ws(t).len() + name.len() + 3
                        <= o.print_width
                    && !open.contains('\n')
                {
                    out.push(format!("{ind}{open}{}</{name}>", collapse_ws(t)));
                } else {
                    out.push(format!("{ind}{open}"));
                    xml_print(children, level + 1, o, out);
                    out.push(format!("{ind}</{name}>"));
                }
            }
        }
        prev_was_node = !matches!(n, X::Text(t) if t.trim().is_empty());
    }
    // Trailing blank lines inside an element aren't kept.
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
}

/// Pretty-prints XML.
pub fn xml_format(text: &str, o: &FormatOptions) -> Result<String, String> {
    let nodes = parse_xml(text)?;
    let mut lines = Vec::new();
    xml_print(&nodes, 0, o, &mut lines);
    let mut s = lines.join("\n");
    s.push('\n');
    Ok(s)
}

fn xml_lin(nodes: &[X], o: &FormatOptions, out: &mut String) {
    for n in nodes {
        match n {
            X::Text(t) if t.trim().is_empty() => {}
            X::Elem {
                name,
                attrs,
                children,
                self_closing: false,
            } if !has_text(children) => {
                out.push_str(&xml_open_tag(
                    name,
                    attrs,
                    false,
                    0,
                    &FormatOptions {
                        print_width: usize::MAX,
                        ..o.clone()
                    },
                ));
                xml_lin(children, o, out);
                out.push_str(&format!("</{name}>"));
            }
            other => xml_raw(other, o, out),
        }
    }
}

/// Removes insignificant whitespace between tags (XML Tools "Linearize").
pub fn xml_minify(text: &str, o: &FormatOptions) -> Result<String, String> {
    let nodes = parse_xml(text)?;
    let mut out = String::new();
    xml_lin(&nodes, o, &mut out);
    Ok(out)
}

pub fn xml_validate(text: &str) -> Result<(), String> {
    parse_xml(text)?;
    // roxmltree also checks entities, namespaces and duplicate attributes.
    roxmltree::Document::parse_with_options(
        text.trim_start_matches('\u{feff}'),
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map(|_| ())
    .map_err(|e| format!("XML error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip_and_known_values() {
        assert_eq!(base64_encode(b"Man", false), "TWFu");
        assert_eq!(base64_encode(b"Ma", false), "TWE=");
        assert_eq!(base64_encode(b"M", false), "TQ==");
        assert_eq!(base64_encode(&[0xfb, 0xff], true), "-_8");
        assert_eq!(base64_decode("TWE=").unwrap(), b"Ma");
        assert_eq!(base64_decode("TW\nE").unwrap(), b"Ma");
        assert_eq!(base64_decode("-_8").unwrap(), vec![0xfb, 0xff]);
        assert!(base64_decode("T!").is_err());
        let s = "héllo 世界 ✓";
        assert_eq!(
            base64_decode(&base64_encode(s.as_bytes(), false)).unwrap(),
            s.as_bytes()
        );
    }

    #[test]
    fn url_and_qp() {
        assert_eq!(url_encode("a b&c/é"), "a%20b%26c%2F%C3%A9");
        assert_eq!(url_decode("a%20b%26c%2F%C3%A9", false).unwrap(), "a b&c/é");
        assert_eq!(url_decode("a+b", true).unwrap(), "a b");
        assert!(url_decode("%zz", false).is_err());
        let s = "café = ok ";
        let e = qp_encode(s);
        assert_eq!(e, "caf=C3=A9 =3D ok=20");
        assert_eq!(qp_decode(&e).unwrap(), s);
        let long = "x".repeat(200);
        assert!(qp_encode(&long).lines().all(|l| l.len() <= 76));
        assert_eq!(qp_decode(&qp_encode(&long)).unwrap(), long);
    }

    #[test]
    fn json_pretty_print_layout() {
        let o = FormatOptions::default();
        let src =
            r#"{"name":"x","n":1.50,"tags":["a","b"],"nested":{"k":null},"empty":{},"list":[]}"#;
        // Too wide for one line (printWidth 80), so the object breaks; short children stay flat.
        assert_eq!(
            json_format(src, &o).unwrap(),
            "{
  \"name\": \"x\",
  \"n\": 1.50,
  \"tags\": [\"a\", \"b\"],
  \"nested\": { \"k\": null },
  \"empty\": {},
  \"list\": []
}
"
        );
        // Single-line objects that are too wide break; multi-line ones always break.
        let src2 = "{\n\"a\": 1, \"b\": [1,2,3], \"c\": {\"d\": true}}";
        assert_eq!(
            json_format(src2, &o).unwrap(),
            "{\n  \"a\": 1,\n  \"b\": [1, 2, 3],\n  \"c\": { \"d\": true }\n}\n"
        );
    }

    #[test]
    fn json_keeps_comments_and_drops_trailing_commas() {
        let o = FormatOptions {
            tab_width: 4,
            ..Default::default()
        };
        let src = "// head\n{\n  // the answer\n  \"a\": 42,\n  \"b\": [1,],\n}";
        assert_eq!(
            json_format(src, &o).unwrap(),
            "// head\n{\n    // the answer\n    \"a\": 42,\n    \"b\": [1]\n}\n"
        );
        assert_eq!(json_minify(src).unwrap(), "{\"a\":42,\"b\":[1]}");
    }

    #[test]
    fn json_errors_report_location() {
        let e = json_validate("{\n  \"a\": 1\n  \"b\": 2\n}").unwrap_err();
        assert!(e.contains("line 3"), "{e}");
        assert!(json_validate("[1, 2").is_err());
    }

    #[test]
    fn xml_pretty_print() {
        let o = FormatOptions::default();
        let src = "<?xml version=\"1.0\"?><root a=\"1\"><item>text</item><empty/><list><x/><y/></list></root>";
        assert_eq!(
            xml_format(src, &o).unwrap(),
            "<?xml version=\"1.0\"?>\n<root a=\"1\">\n  <item>text</item>\n  <empty />\n  <list>\n    <x />\n    <y />\n  </list>\n</root>\n"
        );
    }

    #[test]
    fn xml_options() {
        let src = "<r><e zeta='1' alpha=\"2\" xmlns=\"u\"/></r>";
        let o = FormatOptions {
            xml_sort_attributes: true,
            xml_quotes: XmlQuotes::Double,
            xml_self_closing_space: false,
            ..Default::default()
        };
        assert_eq!(
            xml_format(src, &o).unwrap(),
            "<r>\n  <e xmlns=\"u\" alpha=\"2\" zeta=\"1\"/>\n</r>\n"
        );
        let o2 = FormatOptions {
            single_attribute_per_line: true,
            bracket_same_line: false,
            ..Default::default()
        };
        assert_eq!(
            xml_format("<a x=\"1\" y=\"2\"/>", &o2).unwrap(),
            "<a\n  x=\"1\"\n  y=\"2\"\n/>\n"
        );
    }

    #[test]
    fn xml_whitespace_modes() {
        let src = "<p>Hello   <b>world</b> !</p>";
        let strict = FormatOptions::default();
        assert_eq!(
            xml_format(src, &strict).unwrap(),
            "<p>Hello   <b>world</b> !</p>\n"
        );
        let ignore = FormatOptions {
            xml_whitespace: XmlWhitespace::Ignore,
            ..Default::default()
        };
        assert_eq!(
            xml_format(src, &ignore).unwrap(),
            "<p>\n  Hello\n  <b>world</b>\n  !\n</p>\n"
        );
        // Blank lines between elements collapse to one.
        assert_eq!(
            xml_format("<r>\n<a/>\n\n\n<b/></r>", &strict).unwrap(),
            "<r>\n  <a />\n\n  <b />\n</r>\n"
        );
    }

    #[test]
    fn xml_errors_and_minify() {
        let e = xml_validate("<a>\n<b></a>").unwrap_err();
        assert!(e.contains("does not match") && e.contains("line 2"), "{e}");
        assert!(xml_validate("<a><!-- c --><b x='1'/></a>").is_ok());
        assert_eq!(
            xml_minify(
                "<a>\n  <b x=\"1\" />\n  <c>t  x</c>\n</a>",
                &FormatOptions::default()
            )
            .unwrap(),
            "<a><b x=\"1\" /><c>t  x</c></a>"
        );
    }
}
