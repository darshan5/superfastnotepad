//! Language definitions, loaded from `assets/languages.json`.
//!
//! Each language carries its file extensions, comment delimiters and keyword
//! lists. A small per-language profile (`LexProfile`) adds the lexical details
//! the generic lexer needs (string delimiters, case sensitivity, folding).

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

const LANGUAGES_JSON: &str = include_str!("../assets/languages.json");

/// Keyword classes in the order their styles appear in a theme.
pub const KEYWORD_CLASSES: [&str; 4] = ["keyword", "type", "builtin", "constant"];

#[derive(Deserialize)]
struct LanguageSource {
    name: String,
    display: String,
    extensions: Vec<String>,
    #[serde(default)]
    line_comment: Option<String>,
    #[serde(default)]
    block_comment: Option<(String, String)>,
    #[serde(default)]
    keywords: HashMap<String, Vec<String>>,
}

/// How the generic lexer should treat a language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// No highlighting at all.
    Plain,
    /// Brace/keyword based programming languages.
    Code,
    /// XML / HTML style tag markup.
    Markup,
    /// `[section]` + `key=value` files.
    Ini,
    /// Unified/context diff output.
    Diff,
    /// Markdown-lite.
    Markdown,
    /// Log files: levels, timestamps, numbers and quoted strings.
    Log,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldMode {
    None,
    Braces,
    Indent,
    Markup,
    Keywords,
}

#[derive(Clone, Debug)]
pub struct LexProfile {
    pub family: Family,
    pub case_insensitive: bool,
    /// Characters that open a string literal.
    pub string_delims: &'static str,
    /// Python-style `'''` / `"""`.
    pub triple_quotes: bool,
    /// Lines starting with this character are preprocessor directives.
    pub preprocessor: Option<char>,
    /// Backslash escapes inside strings.
    pub escapes: bool,
    pub fold: FoldMode,
    pub fold_open: &'static [&'static str],
    pub fold_close: &'static [&'static str],
    /// Characters (besides alphanumerics and `_`) allowed in identifiers.
    pub ident_extra: &'static str,
    /// `$var` style variables (shell, perl, php, powershell).
    pub dollar_vars: bool,
}

impl Default for LexProfile {
    fn default() -> Self {
        Self {
            family: Family::Code,
            case_insensitive: false,
            string_delims: "\"'",
            triple_quotes: false,
            preprocessor: None,
            escapes: true,
            fold: FoldMode::Braces,
            fold_open: &[],
            fold_close: &[],
            ident_extra: "",
            dollar_vars: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Language {
    /// Internal id (`cpp`, `python`, ...).
    pub name: String,
    /// Human readable name for menus.
    pub display: String,
    pub exts: Vec<String>,
    pub comment_line: Option<String>,
    pub comment_start: Option<String>,
    pub comment_end: Option<String>,
    /// Keyword class (one of `KEYWORD_CLASSES`) -> words.
    pub keywords: Vec<(String, HashSet<String>)>,
    pub profile: LexProfile,
}

impl Language {
    pub fn is_plain(&self) -> bool {
        self.profile.family == Family::Plain
    }

    /// All keywords from all classes (used for auto-completion).
    pub fn all_keywords(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .keywords
            .iter()
            .flat_map(|(_, set)| set.iter().cloned())
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

const BASH_OPEN: &[&str] = &["if", "do", "case", "{"];
const BASH_CLOSE: &[&str] = &["fi", "done", "esac", "}"];

fn profile_for(name: &str) -> LexProfile {
    let mut p = LexProfile::default();
    match name {
        "normal" => {
            p.family = Family::Plain;
            p.fold = FoldMode::None;
        }
        "xml" | "html" | "php" => {
            p.family = Family::Markup;
            p.fold = FoldMode::Markup;
        }
        "ini" => {
            p.family = Family::Ini;
            p.fold = FoldMode::None;
        }
        "diff" => {
            p.family = Family::Diff;
            p.fold = FoldMode::None;
        }
        "markdown" => {
            p.family = Family::Markdown;
            p.fold = FoldMode::None;
        }
        "log" => {
            p.family = Family::Log;
            p.fold = FoldMode::None;
        }
        "c" | "cpp" | "cs" => p.preprocessor = Some('#'),
        "python" => {
            p.triple_quotes = true;
            p.fold = FoldMode::Indent;
        }
        "vb" | "batch" | "sql" | "dockerfile" => p.fold = FoldMode::Indent,
        "bash" => {
            p.fold = FoldMode::Keywords;
            p.fold_open = BASH_OPEN;
            p.fold_close = BASH_CLOSE;
            p.dollar_vars = true;
            p.string_delims = "\"'`";
        }
        _ => {}
    }
    match name {
        "perl" | "php" | "powershell" | "dockerfile" => p.dollar_vars = true,
        "javascript" | "typescript" | "go" | "kotlin" | "sql" => {
            p.string_delims = "\"'`";
        }
        "vb" | "batch" => {
            p.escapes = false;
            p.string_delims = "\"";
        }
        "css" => p.ident_extra = "-",
        _ => {}
    }
    if name == "powershell" {
        p.ident_extra = "-";
    }
    if matches!(name, "sql" | "vb" | "batch" | "powershell" | "html" | "ini") {
        p.case_insensitive = true;
    }
    p
}

pub struct LangRegistry {
    pub langs: Vec<Language>,
    by_name: HashMap<String, usize>,
    by_ext: HashMap<String, usize>,
}

impl LangRegistry {
    pub fn load() -> Self {
        let sources: Vec<LanguageSource> =
            serde_json::from_str(LANGUAGES_JSON).expect("assets/languages.json is valid");
        let mut langs: Vec<Language> = sources
            .into_iter()
            .map(|src| {
                let profile = profile_for(&src.name);
                let keywords = KEYWORD_CLASSES
                    .iter()
                    .filter_map(|class| {
                        let words: HashSet<String> = src
                            .keywords
                            .get(*class)?
                            .iter()
                            .map(|w| {
                                if profile.case_insensitive {
                                    w.to_lowercase()
                                } else {
                                    w.clone()
                                }
                            })
                            .collect();
                        (!words.is_empty()).then(|| ((*class).to_string(), words))
                    })
                    .collect();
                let (comment_start, comment_end) = src.block_comment.unzip();
                Language {
                    display: src.display,
                    exts: src.extensions.iter().map(|e| e.to_lowercase()).collect(),
                    comment_line: src.line_comment.filter(|c| !c.is_empty()),
                    comment_start,
                    comment_end,
                    keywords,
                    profile,
                    name: src.name,
                }
            })
            .collect();
        if !langs.iter().any(|l| l.name == "normal") {
            langs.insert(
                0,
                Language {
                    name: "normal".into(),
                    display: "Normal text".into(),
                    exts: vec!["txt".into()],
                    comment_line: None,
                    comment_start: None,
                    comment_end: None,
                    keywords: Vec::new(),
                    profile: profile_for("normal"),
                },
            );
        }
        let mut by_name = HashMap::new();
        let mut by_ext = HashMap::new();
        for (i, l) in langs.iter().enumerate() {
            by_name.insert(l.name.clone(), i);
            for e in &l.exts {
                by_ext.entry(e.clone()).or_insert(i);
            }
        }
        Self {
            langs,
            by_name,
            by_ext,
        }
    }

    pub fn normal(&self) -> usize {
        self.by_name.get("normal").copied().unwrap_or(0)
    }

    pub fn by_name(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    /// Detects the language from a file name (extension, then well-known names).
    pub fn detect(&self, file_name: &str, first_line: &str) -> usize {
        let lower = file_name.to_lowercase();
        let special = match lower.as_str() {
            ".bashrc" | ".bash_profile" | ".profile" | ".zshrc" => Some("bash"),
            "dockerfile" | "containerfile" => Some("dockerfile"),
            "cargo.lock" => Some("ini"),
            _ => None,
        };
        if let Some(idx) = special.and_then(|n| self.by_name(n)) {
            return idx;
        }
        if let Some(ext) = lower.rsplit_once('.').map(|(_, e)| e)
            && let Some(&i) = self.by_ext.get(ext)
        {
            return i;
        }
        // Shebang / XML prolog detection for extension-less files.
        let fl = first_line.trim_start();
        let by_content = if fl.starts_with("<?xml") {
            Some("xml")
        } else if fl.to_lowercase().starts_with("<!doctype html") || fl.starts_with("<html") {
            Some("html")
        } else if fl.starts_with("#!") {
            if fl.contains("python") {
                Some("python")
            } else if fl.contains("perl") {
                Some("perl")
            } else if fl.contains("node") {
                Some("javascript")
            } else if fl.contains("php") {
                Some("php")
            } else {
                Some("bash")
            }
        } else {
            None
        };
        by_content
            .and_then(|n| self.by_name(n))
            .unwrap_or_else(|| self.normal())
    }

    /// Languages sorted for display in the Language menu.
    pub fn sorted_for_menu(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.langs.len())
            .filter(|&i| self.langs[i].name != "normal")
            .collect();
        idx.sort_by_key(|&i| self.langs[i].display.to_lowercase());
        idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_core_languages() {
        let reg = LangRegistry::load();
        assert_eq!(reg.langs.len(), 28);
        let cpp = &reg.langs[reg.by_name("cpp").unwrap()];
        assert_eq!(cpp.comment_line.as_deref(), Some("//"));
        assert!(
            cpp.keywords
                .iter()
                .any(|(c, s)| c == "keyword" && s.contains("while"))
        );
    }

    #[test]
    fn detects_by_extension_and_shebang() {
        let reg = LangRegistry::load();
        assert_eq!(reg.langs[reg.detect("main.rs", "")].name, "rust");
        assert_eq!(reg.langs[reg.detect("x.PY", "")].name, "python");
        assert_eq!(
            reg.langs[reg.detect("run", "#!/usr/bin/env python3")].name,
            "python"
        );
        assert_eq!(reg.langs[reg.detect("Dockerfile", "")].name, "dockerfile");
        assert_eq!(reg.langs[reg.detect("Cargo.toml", "")].name, "ini");
        assert_eq!(reg.langs[reg.detect("notes.unknown", "")].name, "normal");
    }
}
