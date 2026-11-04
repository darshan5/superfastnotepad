//! Embeds the built-in theme files from `assets/themes`, and on Windows the
//! app icon and version details shown in Explorer.

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

/// The `.exe` icon (Explorer, the taskbar, shortcuts) and its Properties >
/// Details, from `assets/windows/app.ico` (made by `scripts/make-icon.py`).
fn windows_resources() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=assets/windows/app.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/windows/app.ico")
        .set("ProductName", "superfastnotepad")
        .set("FileDescription", "superfastnotepad")
        .set("LegalCopyright", "MIT License");
    if let Err(e) = res.compile() {
        // Release builds run on Windows, which has the resource compiler.
        // Checking the Windows build from another OS without `llvm-rc` or
        // MinGW `windres` just leaves the icon out.
        if cfg!(windows) {
            panic!("couldn't embed the Windows icon: {e}");
        }
        println!("cargo:warning=building without the Windows icon: {e}");
    }
}

fn main() {
    windows_resources();
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let mut themes = String::new();
    embed_dir("assets/themes", "BUILTIN_THEMES", &mut themes);
    std::fs::write(Path::new(&out_dir).join("themes.rs"), themes).unwrap();
}
