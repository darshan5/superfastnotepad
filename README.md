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

- **macOS (Apple Silicon):** `cargo build --release --target aarch64-apple-darwin`
- **Windows:** `cargo build --release --target x86_64-pc-windows-msvc` (build it on Windows; on a
  Mac you can type-check it with `cargo check --target x86_64-pc-windows-msvc`)

The release build uses the GUI subsystem on Windows, so no console window opens.

## Features

| Area | What's implemented |
|---|---|
| **Editing** | Rope buffer, undo/redo with typing merge, multi-caret (Ctrl/⌘+click), column selection (Alt+drag, Alt+Shift+arrows), column paste, overtype, auto-indent, smart Home, word/line navigation, Copy/Cut line without selection, auto-close brackets/quotes, word and keyword auto-completion |
| **View** | Syntax highlighting for 50 languages, line numbers, bookmark margin, change-history margin, folding (braces / tags / indentation / keywords), word wrap, whitespace and EOL symbols, indent guides, current-line highlight, brace matching, smart highlighting, edge column, zoom, split view with move/clone to other view, synchronized scrolling, full screen, post-it, always on top |
| **File** | Open/save/save as/save a copy, reload, rename, recent files, restore last closed, sessions, session snapshot with unsaved-content backup, change detection, `tail -f` monitoring, drag & drop, single instance, read-only |
| **Encoding** | ANSI, UTF-8, UTF-8-BOM, UTF-16 LE/BE BOM, 30+ legacy charsets (reinterpret or convert), automatic detection; Windows/Unix/Mac EOL conversion |
| **Edit menu** | Line operations (duplicate, delete, move, split, join, transpose, blank lines), 16 sort modes including column-selection sort, dedupe, case conversions, trims, tab/space conversion, line/block comment toggling, Column Editor (text or number sequences), date/time insert, redact, multi-select all/next, character panel, clipboard history |
| **Search** | Find/Replace/Find in Files (background thread)/Mark, in Normal, Extended (`\n \t \xNN`) and Regex modes (back-references, look-around, `\1 $1 ${name} \U \L \E` in replacements), Find All to a results panel (F4 / Shift+F4), incremental search, volatile/select-and-find, Go To line/offset, 5 token styles, bookmarks (cut/copy/paste/remove/inverse bookmarked lines), find characters in range |
| **Panels** | Function List (functions, methods and classes for 39 languages), Document Map, Document List, Folder as Workspace |
| **Compare** | Side-by-side file compare in the two views: spacer rows keep matching lines aligned; added/removed/changed lines and changed words are highlighted, with marks on the scrollbar. Compare the two views, a tab with the next tab, "Compare with this tab" (tab right-click), the clipboard, or the last saved version. Options to ignore whitespace, case or empty lines; automatic re-compare while editing. Next/previous difference (⌘/Ctrl+Alt+N / P), copy a difference to the right or left (⌘/Ctrl+Alt+. / ,). Compare is ⌘/Ctrl+Alt+C and Clear is ⌘/Ctrl+Alt+X. Planned extras are in [TODO.md](TODO.md) |
| **Tools > MIME Tools** | Base64 encode/decode (standard, URL-safe, per line), URL encode/decode, Quoted-printable encode/decode. Applies to the selection, or to the whole document when nothing is selected |
| **Tools > Format** | JSON/JSONC/JSON5 pretty print, minify and validate (keeps comments, key order and number literals), and XML pretty print, minify and syntax check. Options live under Preferences > Formatter. Shortcuts: ⌘/Ctrl+Alt+M (JSON), ⌘/Ctrl+Alt+Shift+B (XML), Alt+Shift+F (auto-detect) |
| **Other** | Six built-in themes (Light, Dark, Midnight, Paper, Forest, High Contrast), Preferences, Shortcut Mapper (read-only), MD5/SHA-1/SHA-256/SHA-512 tools, Run menu with `$(FULL_CURRENT_PATH)` variables, Windows dialog, tab colours/pinning |

Command line: `superfastnotepad [-multiInst] [-nosession] [-ro] [-alwaysOnTop] [-l<lang>] [-n<line>] [-c<col>] [files or folders...]`

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
  remapping, or right-to-left text.

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
  compare.rs        diff engine (alignment, word-level changes)
  compare_ui.rs     compare session: panes, navigation, copy left/right
  encoding.rs       encoding detection and conversion
  dialogs.rs panels.rs icons.rs config.rs platform.rs commands.rs
```

License: [MIT](LICENSE).
