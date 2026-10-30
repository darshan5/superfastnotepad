# Roadmap

Planned performance work and features that aren't built yet. Effort is a rough guess:
**S** = hours, **M** = a day or two, **L** = a week or more.

---

## 1. Performance

### Measured today

macOS, Apple Silicon, release build, file already in the OS page cache.

**Typical files (5 MB):**

| | C++ source | JSON | Log (plain text) |
|---|---|---|---|
| `Document::open` | 5.2 ms | 4.0 ms | 3.7 ms |
| First screen of highlighted lines | 0.12 ms | 0.06 ms | 0.06 ms |
| Colouring the whole file in the background (highlighting on, about 2 ms per frame) | under 0.2 s | about 0.2 s | none needed |
| Jump to the last line afterwards | 0.15 ms | 0.3 ms | 0.09 ms |
| Function List for the whole file (background thread) | 165 ms | n/a | n/a |

CPU time to build and tessellate one frame of the whole window, measured off-screen with the
`ui_bench` test (highlighting on; off is the same or faster):

| | C++ | JSON | Log |
|---|---|---|---|
| Idle | 0.24 ms | 0.41 ms | 0.22 ms |
| Scrolling (new rows every frame) | 0.58 ms | 0.49 ms | 0.64 ms |
| Typing | 1.0 ms | 1.75 ms | 0.93 ms |

A 120 Hz display allows 8.3 ms per frame. When idle, the app only wakes for the caret blink
(about 2 frames a second) and uses under 1 % CPU. Set `SUPERFASTNOTEPAD_PROFILE=1` to print frame
timings of the running app to stderr every two seconds.

Already fixed: the app redrew 120 times a second while idle (the window title was re-sent every
frame); word completion scanned the whole file on every keystroke (now 256 KiB around the
caret: 11–20 ms down to 1.5 ms); brace highlighting scanned to the matching bracket on every
frame, which for a JSON file starting with `[` meant the whole file (87 ms per frame; now cached
per caret position, iterator-based, and limited to 100,000 characters); the Function List re-parsed on the UI thread after every
keystroke and laid out every entry each frame (now a background thread once typing pauses for
0.3 s, and only visible rows are drawn).

**Very large files (1 GiB log, 11 M lines):** opening takes 1.25 s (read 0.42 s, detect and
decode 0.16 s, rope 0.31 s, per-line state the rest), the first jump to the end takes 5 s, and the
app uses about 700 MB.

**Startup:** about 185 ms from launch to the window on screen, most of it the OS creating the
window and the graphics context. Loading languages takes 5 ms and themes under 1 ms.

### Plan (in order of payoff)

| # | Change | Effect | Effort |
|---|---|---|---|
| 1 | **(Done for files up to 2 M lines)** Colour in the background a little each frame; after an edit, re-colour only until the lexer state matches the old one again. For huge files, keep one saved state every N lines instead of one per line | No hitch when jumping around; for 1 GiB, needs the checkpoint version | S |
| 2 | **Sparse per-line state:** keep bookmarks, folds and change history only for the lines that have them, instead of one record per line | Less memory and no O(lines) work at open or on edits that add or remove lines | S |
| 3 | **Memory-map files** instead of reading them | Removes the read time and one full copy | S |
| 4 | **Piece table over the mapped file** (original and added pieces, byte offsets), replacing the rope for large files | Open becomes O(1) in time and memory | L |
| 5 | **Background line index:** count newlines in 512 KiB blocks on another thread; the status bar shows progress | Line count, go-to-line and the scrollbar work while counting | M |
| 6 | **Byte-offset viewport:** scroll position is the byte offset of the top row; wrap and fold maps only cover the visible window | Word wrap stays fast on huge files | M |
| 7 | **Split very long lines** into rows of at most 2 KiB for layout and hit-testing | A 100 MB single-line file no longer freezes rendering | S |
| 8 | **Chunked search** over the buffer instead of copying the whole document into one string; smart highlighting and Find All run off the UI thread | No full-text copy per search | M |
| 9 | **Streaming, atomic save:** write to a temporary file, flush, then rename over the original | Saving never leaves a half-written file | M |
| 10 | **Benchmarks in CI:** `open_bench`, `typing_bench`, `fl_bench` and `ui_bench` (ignored tests) on 100 KB, 5 MB and 1 GiB files | Keeps the gains | S |

**Risks of memory mapping:** a mapped file truncated by another process crashes the reader on
macOS (and raises an access violation on Windows), so the app must watch size and modification
time and fall back to a copy; files on removable or network drives should be copied, not mapped.
Windows locks mapped files, so the map must be released before saving over the same path.

---

## 2. Features not built yet

### Views

| Feature | Effort |
|---|---|
| Hex view (read and edit bytes, offsets column) | M |
| JSON tree view with JSON-pointer copy and jump from a node to its source | M |
| Rendered Markdown preview | M |
| CSV/TSV table view with sortable, resizable columns | M |
| Large-file log viewer: filter lines by text or regex, highlight log levels, export the filtered lines | M |
| Command palette (fuzzy search over all commands) | S |

### Editing and completion

| Feature | Effort |
|---|---|
| Editable keyboard shortcuts | S |
| Function call tips and parameter hints | M |
| Completion of local declarations and members | M |
| Spell check for plain text and Markdown | M–L |
| Natural sort and exact decimal sort | S |
| User-defined languages | M |
| Plugins with a versioned C ABI | L |

### Data tools

| Feature | Effort |
|---|---|
| JSON: sort keys | S |
| YAML formatting | M |
| HTML entity and JSON string escape/unescape | S |
| JWT decode (header and payload, local only) | S |
| Insert a UUID; Unix timestamp ↔ date conversion | S |
| CSV ↔ JSON and CSV ↔ TSV conversion | S |
| Formatters for HTML, CSS, JavaScript/TypeScript and Markdown | L |

### Files and sessions

| Feature | Effort |
|---|---|
| Backups every 1–3 s (today every 7 s); store only edits for big files | S / M |
| Scratchpad: a persistent note with no Save dialog | S |
| Optional autosave after a delay or on focus loss | S |
| Crash recovery prompt and panic log | S |
| Separate session folders (`--session-dir`) | S |
| `--wait` on the command line (block until the file is closed, for `git commit`) | S |
| Offer to detach when a file is truncated | S |
| Go to a percentage of the file | S |

### UI and platform

| Feature | Effort |
|---|---|
| Native macOS menu bar and dock-icon reopen | M |
| Follow the OS light/dark setting | S |
| Translations | M |
| "Mixed" line-ending indicator in the status bar | S |
| Optional, signed update check | M |
| Font fallback for scripts the system monospace font lacks (CJK) | S |
