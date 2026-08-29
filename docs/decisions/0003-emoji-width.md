# 0003 — Emoji are two cells wide under libghostty; xterm-headless makes them one

**Status:** accepted

**Node behavior.** `@xterm/headless` 6.0.0 without the `Unicode11Addon`
(the daemon and the testing library register none; `src/server.ts:333-338`,
`src/testing/session.ts:108-115`) uses its Unicode 6 width table. Emoji added
after Unicode 6.0 — `😀` U+1F600 among them — are width 1. Oracle run against
`pty 0.12.0+500eab2`: `printf '😀X中Y'` then `pty stats --json` reports
`cursorX: 5` (1 + 1 + 2 + 1). Only the TUI's `tests/buffer-wide-char-diff.test.ts`
opts into Unicode 11 widths.

**Rust behavior.** libghostty uses current Unicode width data: `😀` occupies
two cells (a `Wide` cell plus a `Spacer`), so the same output leaves the cursor
at column 6 and the cell after the emoji is at column 2, not column 1. Text
that wraps because of the extra column wraps one cell earlier.

**Why.** libghostty's width tables are not configurable, and width 2 is what
every real terminal the payload is replayed into uses (kitty, ghostty,
wezterm, iTerm2, xterm with modern tables). Matching xterm-headless's width 1
would make the daemon's picture disagree with the attached terminal's.

**Client effect.** `pty stats` cursor columns, DSR answers, `readCells`
column positions and wrap points differ from Node after an emoji on the same
row. Plain text is unaffected (the emoji is one grapheme in both).

**Test.** `crates/pty-terminal/tests/decisions.rs::emoji_is_two_cells_wide`
pins the Rust behavior and cites this record; the Node number above is the
oracle it was measured against.

**Migration.** None. A consumer that needs xterm's answer for a mixed-width
row must ask the terminal it renders into, not the daemon.
