# Lane B — WP4 terminal actor, serialization, queries, snapshot, embedding handle (crate pty-terminal)

Read lane-common.md first. Worktree name: `laneB`. Branch: `laneB`.

You own: `crates/pty-terminal/**` entirely, plus `crates/pty-testkit/src/session.rs` ONLY where it must switch
from the old `screenshot::capture` to the new actor API (keep its public API unchanged). Read the libghostty-vt
0.2.1 source in `~/.cargo/registry/src/*/libghostty-vt-0.2.1/src/` (terminal.rs, fmt.rs, screen.rs, osc.rs,
style.rs, key.rs) before designing; the crate has `grid_ref`, `Row::is_wrapped`, `Cell::{codepoint, wide,
graphemes}`, `GridRef::style`, `on_device_attributes`, `on_xtversion`, `on_enquiry`, `on_bell`, `on_title_changed`,
`on_color_scheme`, `set_default_fg/bg_color`, `set_default_color_palette`, `kitty_keyboard_flags`,
`is_mouse_tracking`, `active_screen`, `scrollback_rows`, `total_rows`, `FormatterOptions::{with_trim, with_unwrap,
with_modes, with_cursor, with_kitty_keyboard}`.

Deliverables (plan-core.md "WP4"; node-daemon-protocol-disk.md 1.6, 1.7, 3.8, 3.9, 6; node-testing-tui.md 2.4):
1. `actor.rs`: `TerminalActor::new(rows, cols, scrollback=10000)`; synchronous methods `write(&[u8])`
   (runs strip + OSC tap + vt_write + mode diff), `resize(cols, rows)`, `reset()`, `plain(Range::Viewport|Full)`,
   `serialize(SerializeOpts)`, `snapshot(scroll_offset) -> CellGrid`, `modes() -> Modes`, `take_pty_replies() -> Vec<u8>`,
   `take_events() -> Vec<TerminalEvent>`, `cursor() -> (x, y, visible)`, `title()`, `scrollback_used()`.
   `TerminalEvent::{Bell, TitleChange(String) (deduplicated), Notification{title, body, source}, FocusRequest,
   CursorVisible}`. `Modes { sgr_mouse, mouse_tracking (1000/1002/1003 individually), alt_screen, cursor_hidden,
   bracketed_paste, focus_events, kitty_stack: Vec<u8> }`. Track the kitty STACK by scanning `CSI > n u` (push)
   and `CSI < [n] u` (pop) in the input stream, and alt screen via `?1049/?1047/?47` — like Node server.ts:343-391.
2. `queries.rs`: install callbacks so a child that asks gets Node's exact answers: DA1 `ESC[?62;22c`,
   DA2 `ESC[>0;382;0c`, DSR `ESC[<y+1>;<x+1>R`, XTVERSION `DCS >|pty(0.8) ST`, OSC 10 `ESC]10;rgb:c0c0/c0c0/c0c0 ESC\`,
   OSC 11 `ESC]11;rgb:0000/0000/0000 ESC\`, OSC 4;i `ESC]4;<i>;rgb:0000/0000/0000 ESC\`. First hour: verify with a
   test which of these libghostty answers via callbacks/default colors; anything it cannot answer, answer
   yourself in `strip.rs` where the query bytes are already intercepted (queue the reply into pty_replies).
   The query bytes must never reach the DATA broadcast (that is `strip.rs`: OSC 10/11/4 `?` BEL or ST, `ESC[c`,
   `ESC[0c`, `ESC[>c`, `ESC[6n`, `ESC[>0q` — tests/terminal-queries.test.ts:20-89 pin the exact set).
3. `serialize.rs`: `serialize_for_replay(&actor, SerializeOpts{include_alt_screen_prefix: bool, scrollback: bool})`
   = Node mode prefix (server.ts:1065-1082: `?1049h` only for attach and only if alt screen active; `?1000h`,
   `?1002h`, `?1003h`, `?1006h` when set; `?25l` when hidden; one `ESC[>flags u` per kitty stack entry) followed by
   libghostty `Format::Vt` with cursor + modes + kitty; `plain_viewport()` = rows `base_y..len`, `plain_full()` =
   all rows, both right-trim each row like xterm `translateToString(true)` and drop trailing empty rows, joined
   by `\n` (server.ts:1269-1293). Decide trailing-written-space handling by the shared fixture
   `tests/fixtures/parity/screens.json` `idle-prompt-plain` (expects `READY> ` length 7) — run the Node pty to
   confirm what xterm does with a written trailing space vs. never-written cells, and match it.
4. `snapshot.rs`: `CellGrid { rows: Vec<Vec<CellSnap>>, wrapped: Vec<bool>, cursor: (row, col, visible), base_y,
   len, cols, rows_n }`; `CellSnap { text: String (grapheme cluster, "" for a wide spacer), fg/bg: ColorSnap::
   {Default, Indexed(u8), Rgb(u8,u8,u8)}, bold, dim, italic, underline, inverse, strikethrough, wide: Narrow|Wide|
   Spacer }` from `grid_ref`/`Row::is_wrapped`/style (palette index preserved when the source was indexed — the
   `fgIndex` contract in node-testing-tui.md 2.4).
5. `handle.rs`: `TerminalHandle` (issues #1 and #3 on compoundingtech/pty-rust; node-testing-tui.md 2.4 PtyHandle):
   `TerminalHandle::spawn(cmd, args, SpawnOptions{rows, cols, cwd, env, scrollback})` owns a portable-pty child;
   `TerminalHandle::attach(SessionRef{root, id}, AttachOptions{rows, cols, readonly})` connects to
   `<root>/<id>.sock` using `pty_core::protocol` (ATTACH or PEEK, DATA, RESIZE, DETACH; GEOMETRY(10) resizes
   the actor; SCREEN = reset + write; EXIT → exited). One actor thread per handle (the `!Send` terminal lives
   there; the public handle is `Send + Sync`); `AttemptId` per attach/reconnect so frames from an older attempt
   are dropped before they reach the actor; explicit readiness = first SCREEN parsed (a `ready()` future/blocking
   wait, no fixed 100 ms delay); API: `write(&[u8])`, `resize(cols, rows)`, `snapshot(scroll_offset) -> CellGrid`
   (cached per revision), `rev()`, `subscribe() -> Receiver<HandleEvent::{Dirty(rev), Title, Bell, Geometry(rows,
   cols), Exited(code)}>`, `modes()`, `cursor()`, `cols()/rows()`, `exited()`, `kill()/close()` (spawn: kill +
   reap; attach: DETACH + drop), `reconnect()`, `set_palette(theme colors)`.
6. Update `pty-testkit::Session` to use `TerminalActor` (same screenshot semantics) so there is one serializer.

Tests (crates/pty-terminal/tests/ and existing pty-testkit tests must stay green): terminal-queries responses
byte-equal (tests/terminal-queries.test.ts:93-149 via a real child that echoes); the three `screens.json`
fixtures pass with viewport semantics; mode prefix cases from tests/screen-replay-altscreen.test.ts:62-143
and tests/integration.test.ts:1617-1671 (kitty `>1u`, `?1006h`, `?25l` reach a late attacher; `?1047h`
normalized to `?1049h`); a ratatui-compat style replay (tests/ratatui-compat.test.ts sections 1-3: ECH/CUF with
backgrounds, full-screen EL redraw, kitty stack push/pop) proven by re-parsing the replay in a second actor and
comparing `plain_full()` + styles with the original; snapshot tests ported from tests/pty-handle.test.ts (cursor,
mouseMode 1000/1002/1003, alternateScreen, kitty stack copy, wrapped flags, scrollback/bufferLength/baseY,
readCells offsets and clamping, palette index preservation for SGR 34/94/38;5;N/48;5;N, truecolor → no index);
handle tests: spawn `cat`, write, snapshot; attach identity (`--id a`, exit, `--id a` again → new attach reaches
the replacement) and late-event rejection against the Rust daemon built from this worktree (`cargo build -p pty`,
the daemon in this worktree still lacks GEOMETRY — handle must tolerate a daemon that sends SCREEN first).
Write decision records under docs/decisions/ for any rendering difference you cannot hide (expected: 0002 ANSI
serialization byte-differs-but-equivalent; maybe emoji width, reflow); name the fixture that proves each.
