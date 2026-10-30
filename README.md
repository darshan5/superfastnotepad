# superfastnotepad

A fast text and source-code editor written in **pure Rust**. Runs natively on **Windows**
(x86_64) and **macOS** (Apple Silicon and Intel), and is drawn on the GPU for smooth scrolling.

Language definitions, colour themes and Function List rules are plain JSON files in `assets/`,
embedded in the binary at build time.

## Build

```sh
cargo build --release            # binary: target/release/superfastnotepad(.exe)
cargo test                       # unit tests (lexer, search, ops, encodings, ...)
```

- **macOS (Apple Silicon):** `cargo build --release --target aarch64-apple-darwin`, or
  `scripts/bundle-macos.sh` to build `target/aarch64-apple-darwin/release/superfastnotepad.app`
  (icon from `scripts/make-icon.py`, ad-hoc signed; pass `x86_64-apple-darwin` for Intel)
- **Windows:** `cargo build --release --target x86_64-pc-windows-msvc` (build it on Windows; on a
  Mac you can type-check it with `cargo check --target x86_64-pc-windows-msvc`)

The release build uses the GUI subsystem on Windows, so no console window opens.

## Features

| Area | What's implemented |
|---|---|
| **Editing** | Rope buffer, undo/redo with typing merge, multi-caret (Ctrl/⌘+click), column selection (Alt+drag, Alt+Shift+arrows), column paste, overtype, auto-indent, smart Home, auto-close brackets/quotes, word and keyword completion (names the file declares first; members after `.`), call tips with the parameter under the caret underlined |
| **View** | Syntax highlighting for 26 languages plus log files (off by default; toolbar button or View > Syntax Highlighting; colours the rest of the file in the background), font size (⌘/Ctrl + and −, View > Font Size), line numbers, bookmarks, change history, folding, word wrap, whitespace and line-ending symbols, indent guides, brace matching, split view, full screen, post-it, always on top |
| **File** | Open/save/save as/save a copy, reload, rename, recent files, sessions (with `--session-dir` for separate workspaces), unsaved-content backups every 3 s when something changed, window position restored, change detection on disk, drag and drop, single instance, File > New Window |
| **Encoding** | Language, ANSI, UTF-8, UTF-8-BOM, UTF-16 LE/BE BOM, 30+ legacy character sets (reinterpret or convert), automatic detection from a sample, Windows/Unix/Mac line endings with a "Mixed" indicator in the status bar |
| **Edit** | Cut/copy/paste, selection tools, line operations, 16 sort modes, case conversions, comments, whitespace tools, Column Editor, insert date/time or a UUID, redact |
| **Search** | Find/Replace/Find in Files (background), Mark, Normal/Extended/Regex modes, Find All results panel, incremental search, Filter Lines (show only matching lines, or hide them; export to a new tab), Go to Line (also a box on the toolbar), highlight tokens in 5 styles, bookmarks |
| **Panels** | Function List (20 languages, background scan), JSON Tree (jump to source, copy JSON pointer/key/value), Document Map (with compare markers), Document List, Folder as Workspace, Character Panel, Clipboard History |
| **Compare** | Side by side with aligned spacer rows, or inline in one pane. Added/removed/changed/moved lines and changed words, scrollbar and Document Map markers. Compare two views, a tab with the next, selected lines, the clipboard, the last save, or a blank Original/Changed pair. Ignore whitespace, case, empty lines or a regex; swap sides; copy one or all differences left/right; side panels hidden while comparing |
| **Tools** | Format: JSON, XML, HTML, CSS, JavaScript/TypeScript, Markdown (pretty print; minify for JSON/XML/CSS). Encode/Decode: Base64, URL, Quoted-printable. Data: CSV↔JSON, CSV↔TSV. Hash: MD5, SHA-1, SHA-256, SHA-512. Date and Unix Time: live clock, date picker to Unix time and back |
| **Other** | Six built-in themes (Light, Dark, Midnight, Paper, Forest, High Contrast) that can follow the OS light/dark setting, Preferences, keyboard shortcut list, tab colours and pinning |

Command line: `superfastnotepad [-multiInst] [-nosession] [--session-dir <folder>] [-ro] [-alwaysOnTop] [-l<lang>] [-n<line>] [-c<col>] [files or folders...]`

Settings, session and backups are stored in the OS config dir (for example
`~/Library/Application Support/org.superfastnotepad.superfastnotepad` on macOS,
`%APPDATA%\superfastnotepad` on Windows). Set `SUPERFASTNOTEPAD_CONFIG_DIR` to override it, and
`SUPERFASTNOTEPAD_PROFILE=1` to print frame timings to stderr.

See [docs/ROADMAP.md](docs/ROADMAP.md) for performance measurements, the performance plan and
features that aren't built yet, and [TODO.md](TODO.md) for planned compare features.

## Limitations

- **Fonts:** the app uses only the built-in system monospace font (Menlo on macOS, Consolas on
  Windows) for the editor and the UI. There's no font chooser or fallback font, so characters
  that font lacks (for example Chinese, Japanese, Korean) show as empty boxes.
- **Highlighting** uses one generic, data-driven lexer for all languages. Embedded JS/CSS in HTML
  and PHP blocks are handled; very language-specific constructs (for example Perl heredocs) are
  highlighted more simply.
- No macros, plugins, printing, user-defined languages, translations, auto-updater, shortcut
  remapping, spell check, or right-to-left text.

## Source layout

```
src/
  main.rs           startup, CLI args, font (one system monospace face), single-instance
  app.rs            state + command dispatch
  ui.rs             menus, toolbar, tab bars, split views, status bar
  editor.rs         the editor widget
  ops.rs            editing operations on document + selections
  document.rs       rope buffer, undo, line metadata, folds, lex cache
  lexer.rs          generic incremental lexer
  langs.rs/theme.rs language and theme loading (assets/*.json)
  search.rs         find/replace/find-in-files engine
  function_list.rs  Function List engine (assets/function_lists.json)
  textops.rs        pure text transforms (sort, case, trim, hashes, ...)
  formatters.rs     MIME tools + JSON/XML formatters
  formatters_web.rs HTML/CSS/JS/TS/Markdown formatters
  datatools.rs      CSV/TSV/JSON conversion, UUID, timestamps
  json_tree.rs      JSON tree parser (offsets, JSON pointers)
  filter.rs         Filter Lines
  calltip.rs        call tips from the file's own definitions
  compare.rs        diff engine (alignment, word-level changes, moved lines)
  compare_ui.rs     compare session: panes, navigation, copy left/right
  encoding.rs       encoding detection and conversion
  dialogs.rs panels.rs icons.rs config.rs platform.rs commands.rs
```

License: [MIT](LICENSE).
