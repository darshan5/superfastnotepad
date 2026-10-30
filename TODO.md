# TODO

## File compare: after the first version

The first version covers aligned side-by-side panes, added/removed/changed highlighting,
word-level highlighting inside changed lines, and comparing the two views or a tab with the next
tab. It also covers "Compare with this" on a tab, comparing with the clipboard or the last saved
version, ignoring whitespace, case and empty lines, automatic re-compare, next/previous
difference, copying a difference to the left or right, and difference markers on the scrollbar.

Since then (2026-10-30): inline single-pane view (Compare > Show Differences Inline), moved
lines in blue, comparing selected lines, a blank "Original / Changed" compare, an ignore pattern
(taken from the Find box), Swap Sides, hiding the side panels while comparing, compare options
saved with the session (per `--session-dir`), difference markers in the Document Map, and Copy
All to Left/Right (one undo step). Word wrap is never changed by a compare, and each window can
hold its own compare.

Still to do:

- [ ] **Custom compare colours** (today they follow the light or dark theme)

## Performance

See section 1 of [docs/ROADMAP.md](docs/ROADMAP.md) for the plan to open very large files
instantly (memory-mapped piece table, background line index, byte-offset viewport, and so on).
