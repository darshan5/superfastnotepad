//! Embeds the built-in theme files from `assets/themes`.

use std::fmt::Write as _;
use std::path::Path;

fn embed_dir(dir: &str, const_name: &str, out: &mut String) {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = Path::new(&manifest).join(dir);
    println!("cargo:rerun-if-changed={}", path.display());
    let mut entries: Vec<_> = std::fs::read_dir(&path)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    entries.retain(|p| p.extension().is_some_and(|e| e == "json"));
    entries.sort();
    writeln!(out, "pub static {const_name}: &[(&str, &str)] = &[").unwrap();
    for p in entries {
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        writeln!(
            out,
            "    ({stem:?}, include_str!({:?})),",
            p.display().to_string()
        )
        .unwrap();
    }
    writeln!(out, "];").unwrap();
}

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let mut themes = String::new();
    embed_dir("assets/themes", "BUILTIN_THEMES", &mut themes);
    std::fs::write(Path::new(&out_dir).join("themes.rs"), themes).unwrap();
}
