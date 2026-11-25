//! Colour themes, loaded from `assets/themes/*.json`.
//!
//! A theme has UI colours (`ui`) and one style per token kind (`tokens`).
//! The same token styles apply to every language.

use std::collections::HashMap;

use egui::Color32;
use serde::Deserialize;

use crate::langs::{KEYWORD_CLASSES, Language};
use crate::lexer::TokenKind;

include!(concat!(env!("OUT_DIR"), "/themes.rs"));

/// The theme used when the configured one doesn't exist.
pub const DEFAULT_THEME: &str = "Light";
/// The dark theme used when following the system's dark mode.
pub const DEFAULT_DARK_THEME: &str = "Dark";

#[derive(Clone, Copy, Debug, Default, Deserialize)]
struct TokenSource {
    #[serde(default, deserialize_with = "hex_opt")]
    fg: Option<Color32>,
    #[serde(default, deserialize_with = "hex_opt")]
    bg: Option<Color32>,
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    italic: bool,
    #[serde(default)]
    underline: bool,
}

#[derive(Deserialize)]
struct ThemeSource {
    name: String,
    /// Menus, dialogs and search results also use the full text colour.
    #[serde(default)]
    high_contrast: bool,
    ui: HashMap<String, String>,
    tokens: HashMap<String, TokenSource>,
}

fn parse_hex(s: &str) -> Option<Color32> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    let [_, r, g, b] = v.to_be_bytes();
    Some(Color32::from_rgb(r, g, b))
}

fn hex_opt<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Color32>, D::Error> {
    let s: Option<String> = Option::deserialize(d)?;
    Ok(s.as_deref().and_then(parse_hex))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub fg: Color32,
    pub bg: Option<Color32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// Styles of one language under one theme, indexed by token kind.
#[derive(Clone, Debug)]
pub struct LangStyles {
    pub kinds: Vec<Style>,
    pub keywords: Vec<Style>,
}

impl LangStyles {
    pub fn get(&self, kind: TokenKind) -> Style {
        match kind {
            TokenKind::Keyword(i) => self
                .keywords
                .get(i as usize)
                .copied()
                .unwrap_or(self.kinds[TokenKind::Default.index()]),
            k => self.kinds[k.index()],
        }
    }
}

/// UI colours from the theme's GlobalStyles section.
#[derive(Clone, Debug)]
pub struct Globals {
    pub fg: Color32,
    pub bg: Color32,
    pub current_line_bg: Color32,
    pub selection_bg: Color32,
    pub caret: Color32,
    pub line_number_fg: Color32,
    pub line_number_bg: Color32,
    pub bookmark_margin_bg: Color32,
    pub fold_margin_bg: Color32,
    pub fold_fg: Color32,
    pub fold_bg: Color32,
    pub indent_guide: Color32,
    pub brace_fg: Color32,
    pub bad_brace_fg: Color32,
    pub whitespace: Color32,
    pub smart_highlight: Color32,
    pub find_mark: Color32,
    pub mark_styles: [Color32; 5],
    pub incremental: Color32,
    pub edge: Color32,
    pub change_modified: Color32,
    pub change_saved: Color32,
    pub active_tab_indicator: Color32,
    pub is_dark: bool,
    /// Full-strength text everywhere (for screenshots and OCR).
    pub high_contrast: bool,
}

pub struct Theme {
    pub name: String,
    high_contrast: bool,
    ui: HashMap<String, Color32>,
    tokens: HashMap<String, TokenSource>,
}

fn luminance(c: Color32) -> f32 {
    0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b())
}

impl Theme {
    pub fn parse(json: &str) -> Option<Self> {
        let src: ThemeSource = serde_json::from_str(json).ok()?;
        Some(Self {
            name: src.name,
            high_contrast: src.high_contrast,
            ui: src
                .ui
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), parse_hex(v)?)))
                .collect(),
            tokens: src.tokens,
        })
    }

    fn ui(&self, key: &str, fallback: Color32) -> Color32 {
        self.ui.get(key).copied().unwrap_or(fallback)
    }

    pub fn globals(&self) -> Globals {
        let fg = self.ui("fg", Color32::BLACK);
        let bg = self.ui("bg", Color32::WHITE);
        let u = |k: &str, f: Color32| self.ui(k, f);
        let line_number_bg = u("line_number_bg", Color32::from_gray(0xE4));
        Globals {
            fg,
            bg,
            current_line_bg: u("current_line_bg", Color32::from_rgb(0xE8, 0xE8, 0xFF)),
            selection_bg: u("selection_bg", Color32::from_gray(0xC0)),
            caret: u("caret", fg),
            line_number_fg: u("line_number_fg", Color32::GRAY),
            line_number_bg,
            bookmark_margin_bg: u("bookmark_margin_bg", line_number_bg),
            fold_margin_bg: u("fold_margin_bg", Color32::from_gray(0xE9)),
            fold_fg: u("fold_fg", Color32::GRAY),
            fold_bg: u("fold_bg", bg),
            indent_guide: u("indent_guide", Color32::from_gray(0xC0)),
            brace_fg: u("brace_fg", Color32::RED),
            bad_brace_fg: u("bad_brace_fg", Color32::from_rgb(0x80, 0, 0)),
            whitespace: u("whitespace", Color32::from_rgb(0xFF, 0xB5, 0x6A)),
            smart_highlight: u("smart_highlight", Color32::GREEN),
            find_mark: u("find_mark", Color32::RED),
            mark_styles: [
                u("mark_1", Color32::from_rgb(0, 0xFF, 0xFF)),
                u("mark_2", Color32::from_rgb(0xFF, 0x80, 0)),
                u("mark_3", Color32::from_rgb(0xFF, 0xFF, 0)),
                u("mark_4", Color32::from_rgb(0x80, 0, 0xFF)),
                u("mark_5", Color32::from_rgb(0, 0x80, 0)),
            ],
            incremental: u("incremental", Color32::from_rgb(0, 0x80, 0xFF)),
            edge: u("edge", Color32::from_rgb(0x80, 0xFF, 0xFF)),
            change_modified: u("change_modified", Color32::from_rgb(0xFF, 0x80, 0)),
            change_saved: u("change_saved", Color32::from_rgb(0, 0xA0, 0)),
            active_tab_indicator: u("active_tab_indicator", Color32::from_rgb(0xFA, 0xAA, 0x3C)),
            is_dark: luminance(bg) < 110.0,
            high_contrast: self.high_contrast,
        }
    }

    /// Resolves the colours for every token kind of `lang`.
    pub fn resolve(&self, lang: &Language) -> LangStyles {
        let fg = self.ui("fg", Color32::BLACK);
        let style = |key: &str, fallback: Style| -> Style {
            self.tokens.get(key).map_or(fallback, |t| Style {
                fg: t.fg.unwrap_or(fg),
                // Plain text never gets token backgrounds.
                bg: t.bg.filter(|_| !lang.is_plain()),
                bold: t.bold,
                italic: t.italic,
                underline: t.underline,
            })
        };
        let base = Style {
            fg,
            bg: None,
            bold: false,
            italic: false,
            underline: false,
        };
        let default = style("default", base);
        let comment = style("comment", default);
        let string = style("string", default);
        let mut kinds = vec![default; TokenKind::COUNT];
        let table = [
            (TokenKind::Comment, "comment", comment),
            (TokenKind::CommentLine, "comment_line", comment),
            (TokenKind::CommentDoc, "comment_doc", comment),
            (TokenKind::Number, "number", default),
            (TokenKind::String, "string", string),
            (TokenKind::Char, "char", string),
            (TokenKind::Operator, "operator", default),
            (TokenKind::Preprocessor, "preprocessor", default),
            (TokenKind::Variable, "variable", default),
            (TokenKind::Tag, "tag", default),
            (TokenKind::Attribute, "attribute", default),
            (TokenKind::AttrValue, "attr_value", string),
            (TokenKind::Entity, "entity", default),
            (
                TokenKind::Section,
                "section",
                Style {
                    bold: true,
                    ..default
                },
            ),
            (TokenKind::Key, "key", default),
            (TokenKind::Assign, "assign", default),
            (TokenKind::DiffAdd, "diff_add", default),
            (TokenKind::DiffDel, "diff_del", default),
            (TokenKind::DiffHeader, "diff_header", default),
            (TokenKind::DiffPosition, "diff_position", default),
            (
                TokenKind::Heading,
                "heading",
                Style {
                    bold: true,
                    ..default
                },
            ),
            (
                TokenKind::Emphasis,
                "emphasis",
                Style {
                    italic: true,
                    ..default
                },
            ),
            (TokenKind::Code, "code", string),
            (
                TokenKind::Link,
                "link",
                Style {
                    underline: true,
                    ..default
                },
            ),
        ];
        for (kind, key, fallback) in table {
            kinds[kind.index()] = style(key, fallback);
        }
        let keyword = style(
            "keyword",
            Style {
                bold: true,
                ..default
            },
        );
        let keywords = lang
            .keywords
            .iter()
            .map(|(class, _)| {
                if KEYWORD_CLASSES.contains(&class.as_str()) {
                    style(class, keyword)
                } else {
                    keyword
                }
            })
            .collect();
        LangStyles { kinds, keywords }
    }
}

pub struct ThemeRegistry {
    pub themes: Vec<Theme>,
}

impl ThemeRegistry {
    pub fn load() -> Self {
        let mut themes: Vec<Theme> = BUILTIN_THEMES
            .iter()
            .filter_map(|(_, json)| Theme::parse(json))
            .collect();
        // The default theme first, then the rest by name.
        themes.sort_by_key(|t| (t.name != DEFAULT_THEME, t.name.to_lowercase()));
        Self { themes }
    }

    /// The theme called `name`, ignoring case, `-` and `_`
    /// (`high-contrast-light` finds "High Contrast Light").
    pub fn find(&self, name: &str) -> Option<usize> {
        let norm = |s: &str| s.to_lowercase().replace(['-', '_'], " ");
        let want = norm(name.trim());
        self.themes.iter().position(|t| norm(&t.name) == want)
    }

    /// Saved theme names that no longer exist (the dark "High Contrast" was
    /// removed) go back to the default light and dark themes.
    pub fn replace_missing(&self, light: &mut String, dark: &mut String) {
        if self.find(light).is_none() {
            *light = DEFAULT_THEME.into();
        }
        if self.find(dark).is_none() {
            *dark = DEFAULT_DARK_THEME.into();
        }
    }

    pub fn index_of(&self, name: &str) -> usize {
        self.themes.iter().position(|t| t.name == name).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langs::LangRegistry;

    #[test]
    fn all_themes_load_and_default_is_first() {
        let reg = ThemeRegistry::load();
        assert_eq!(reg.themes.len(), BUILTIN_THEMES.len());
        assert_eq!(reg.themes[0].name, DEFAULT_THEME);
        for t in &reg.themes {
            assert_eq!(t.ui.len(), 27, "{}: missing ui colours", t.name);
        }
    }

    #[test]
    fn light_theme_resolves_cpp_keywords() {
        let reg = ThemeRegistry::load();
        let langs = LangRegistry::load();
        let cpp = &langs.langs[langs.by_name("cpp").unwrap()];
        let styles = reg.themes[0].resolve(cpp);
        let kw = styles.get(TokenKind::Keyword(0));
        assert!(kw.bold);
        assert_ne!(kw.fg, styles.get(TokenKind::Default).fg);
        assert_ne!(
            styles.get(TokenKind::Comment).fg,
            styles.get(TokenKind::Default).fg
        );
    }

    #[test]
    fn dark_theme_detected() {
        let reg = ThemeRegistry::load();
        assert!(reg.themes[reg.index_of("Dark")].globals().is_dark);
        assert!(!reg.themes[reg.index_of("Light")].globals().is_dark);
        let hc = reg
            .find("high-contrast-light")
            .expect("High Contrast Light");
        let g = reg.themes[hc].globals();
        assert!(g.high_contrast && !g.is_dark);
        assert_eq!((g.fg, g.bg), (Color32::BLACK, Color32::WHITE));
        assert!(!reg.themes[reg.index_of("Light")].globals().high_contrast);
        assert_eq!(reg.find("HIGH contrast LIGHT"), Some(hc));
        assert_eq!(reg.find("nope"), None);
        // The removed dark "High Contrast" falls back to the defaults.
        let (mut light, mut dark) = ("High Contrast".to_string(), "High Contrast".to_string());
        reg.replace_missing(&mut light, &mut dark);
        assert_eq!(
            (light.as_str(), dark.as_str()),
            (DEFAULT_THEME, DEFAULT_DARK_THEME)
        );
        let (mut light, mut dark) = ("Paper".to_string(), "Midnight".to_string());
        reg.replace_missing(&mut light, &mut dark);
        assert_eq!((light.as_str(), dark.as_str()), ("Paper", "Midnight"));
    }

    /// The OCR theme: no italics, and every text colour at least 7:1
    /// against its white background (WCAG AAA).
    #[test]
    fn high_contrast_light_is_readable() {
        fn lum(c: Color32) -> f64 {
            let ch = |v: u8| {
                let v = f64::from(v) / 255.0;
                if v <= 0.040_45 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        }
        let reg = ThemeRegistry::load();
        let t = &reg.themes[reg.find("High Contrast Light").unwrap()];
        for (name, tok) in &t.tokens {
            assert!(!tok.italic, "{name} is italic");
            let fg = tok.fg.unwrap();
            let ratio = 1.05 / (lum(fg) + 0.05);
            assert!(ratio >= 7.0, "{name}: contrast {ratio:.1}");
        }
        assert_eq!(t.ui("line_number_fg", Color32::GRAY), Color32::BLACK);
    }
}
