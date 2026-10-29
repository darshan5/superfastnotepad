# TODO

## File compare: after the first version

The first version covers aligned side-by-side panes, added/removed/changed highlighting,
word-level highlighting inside changed lines, and comparing the two views or a tab with the next
tab. It also covers "Compare with this" on a tab, comparing with the clipboard or the last saved
version, ignoring whitespace, case and empty lines, automatic re-compare, next/previous
difference, copying a difference to the left or right, and difference markers on the scrollbar.

Still to do:

- [ ] **Inline (single-pane) diff view** as an alternative to side by side
- [ ] **Moved-line detection:** show lines that only moved in a separate colour (blue)
- [ ] **Compare selected lines** of one view against selected lines of the other, with selections expanded to full lines
- [ ] **Compare against Git:** the file's last committed version (HEAD) or the index
- [ ] **Blank compare window:** paste two texts into "Original" and "Changed" panes without creating files
- [ ] **Ignore a regex:** use the current Find expression as an ignore pattern, plus "Clear ignore regex"
- [ ] **Remember wrap state:** turn word wrap off while comparing and restore the user's setting afterwards (today wrap is simply ignored during a compare)
- [ ] **Colour settings:** choose which side shows additions; custom added/removed/changed colours
- [ ] **Hide side panels while comparing** (Function List, Folder as Workspace…) and restore them afterwards
- [ ] **Per-workspace compare options** that persist with the session (today they're global settings)
- [ ] **Clear size limits and messages** for very large inputs, for example a combined size limit and a per-line limit for word highlighting (today: a fixed 64 MB cap plus a message)
- [ ] **Several compare pairs at once**
- [ ] **Difference markers in the Document Map** panel, not only the scrollbar
- [ ] **Merge helpers:** "copy all differences left/right" and undo of a copy as one step across both documents

## Performance

See section 1 of [docs/ROADMAP.md](docs/ROADMAP.md) for the plan to open very large files
instantly (memory-mapped piece table, background line index, byte-offset viewport, and so on).
