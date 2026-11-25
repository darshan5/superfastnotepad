//! The one font the editor and the interface use: the platform's built-in
//! monospace face, or a font file picked in Settings > Appearance.

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Built-in monospace faces, first found wins (Menlo on macOS, Consolas on
/// Windows).
fn default_candidates() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/Menlo.ttc",
            "/System/Library/Fonts/Monaco.ttf",
        ]
    } else if cfg!(windows) {
        &[
            "C:\\Windows\\Fonts\\consola.ttf",
            "C:\\Windows\\Fonts\\cour.ttf",
        ]
    } else {
        &["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"]
    }
}

/// Name of the built-in face, for the Settings list.
pub fn default_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(windows) {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}

/// Folders that hold installed fonts.
fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.push("/System/Library/Fonts".into());
        dirs.push("/System/Library/Fonts/Supplemental".into());
        dirs.push("/Library/Fonts".into());
        // SF Mono ships inside Terminal.
        dirs.push("/System/Applications/Utilities/Terminal.app/Contents/Resources/Fonts".into());
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join("Library/Fonts"));
        }
    } else if cfg!(windows) {
        let windir = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
        dirs.push(PathBuf::from(windir).join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft\\Windows\\Fonts"));
        }
    } else {
        dirs.push("/usr/share/fonts".into());
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".local/share/fonts"));
        }
    }
    dirs
}

/// A readable name for a font file, or `None` when it isn't a regular
/// (upright, normal weight) monospace face.
fn monospace_name(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
        return None;
    }
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let lower = stem.to_lowercase();
    // Windows' short file names.
    let known = match lower.as_str() {
        "consola" => Some("Consolas"),
        "cour" => Some("Courier New"),
        "lucon" => Some("Lucida Console"),
        "sfnsmono" => Some("SF Mono (system)"),
        "consolab" | "consolai" | "consolaz" | "courbd" | "courbi" | "couri" => return None,
        _ => None,
    };
    if let Some(name) = known {
        return Some(name.to_string());
    }
    const MONO: &[&str] = &[
        "mono",
        "code",
        "consol",
        "courier",
        "menlo",
        "monaco",
        "hack",
        "inconsolata",
        "iosevka",
    ];
    const VARIANT: &[&str] = &[
        "bold",
        "italic",
        "oblique",
        "light",
        "medium",
        "heavy",
        "thin",
        "black",
        "semibold",
        "extra",
        "nerdfontpropo",
    ];
    // "Arial Unicode" contains "code" but isn't fixed-width.
    if !MONO.iter().any(|m| lower.contains(m))
        || VARIANT.iter().any(|v| lower.contains(v))
        || lower.contains("unicode")
    {
        return None;
    }
    Some(stem.replace(['-', '_'], " ").replace(" Regular", ""))
}

/// Installed monospace fonts as (name, file), sorted by name.
pub fn monospace_fonts() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for dir in font_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if let Some(name) = monospace_name(&path)
                && !out.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name))
            {
                out.push((name, path));
            }
        }
    }
    out.sort_by_key(|(n, _)| n.to_lowercase());
    out
}

/// Makes `font_file` (empty = the built-in face) the font of the whole UI.
/// The built-in face stays behind a chosen font, for characters it lacks.
/// On failure the built-in face is used and the error is returned.
pub fn apply(ctx: &egui::Context, font_file: &str) -> Result<(), String> {
    let mut fonts = egui::FontDefinitions::empty();
    let mut family = Vec::new();
    let mut err = None;
    if !font_file.is_empty() {
        match std::fs::read(font_file) {
            Ok(b) => {
                fonts
                    .font_data
                    .insert("chosen".into(), Arc::new(egui::FontData::from_owned(b)));
                family.push("chosen".to_string());
            }
            Err(e) => err = Some(format!("{font_file}: {e}")),
        }
    }
    fonts.font_data.insert(
        "system".into(),
        Arc::new(egui::FontData::from_owned(load_default()?)),
    );
    family.push("system".into());
    for fam in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
        fonts.families.insert(fam, family.clone());
    }
    ctx.set_fonts(fonts);
    err.map_or(Ok(()), Err)
}

fn load_default() -> Result<Vec<u8>, String> {
    let candidates = default_candidates();
    candidates
        .iter()
        .find_map(|p| std::fs::read(p).ok())
        .ok_or_else(|| format!("no system font found (tried {})", candidates.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_regular_monospace_files() {
        let n = |s: &str| monospace_name(Path::new(s));
        assert_eq!(n("/w/consola.ttf").as_deref(), Some("Consolas"));
        assert_eq!(n("/w/consolab.ttf"), None);
        assert_eq!(n("/x/SF-Mono-Regular.otf").as_deref(), Some("SF Mono"));
        assert_eq!(n("/x/SF-Mono-Bold.otf"), None);
        assert_eq!(
            n("/x/JetBrainsMono-Regular.ttf").as_deref(),
            Some("JetBrainsMono")
        );
        assert_eq!(n("/x/Andale Mono.ttf").as_deref(), Some("Andale Mono"));
        assert_eq!(n("/x/Helvetica.ttc"), None);
        assert_eq!(n("/x/Arial Unicode.ttf"), None);
        assert_eq!(n("/x/Mono notes.txt"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finds_installed_monospace_fonts() {
        let names: Vec<String> = monospace_fonts().into_iter().map(|(n, _)| n).collect();
        assert!(names.iter().any(|n| n == "Menlo"), "{names:?}");
    }
}
