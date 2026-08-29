# Lane WP-TUI — pty-tui on ratatui + crossterm, all 28 widgets, and the session manager

Read lane-common.md, plan-verify-libs.md "B4", node-testing-tui.md sections 2, 3, 4, 5 (the whole feature list),
docs/parity.md section 9. Worktree: `wptui`. Branch: `wptui` (off `parity` after lanes B and C are merged; the
manager's attach needs WP7b — build the library and widgets first, the manager last).

You own: `crates/pty-tui/**`, `crates/pty/src/interactive/**`, the `interactive` hook in `crates/pty/src/cli/mod.rs`
(one call), `crates/pty/tests/interactive.rs`. Deps: `ratatui` (latest 0.29.x), `crossterm` 0.29 (features for
kitty keyboard `PushKeyboardEnhancementFlags` and SGR mouse), `pty-terminal` (TerminalHandle, CellGrid), `pty-core`.
Node's TUI source is <node-pty-checkout>/src/tui/** — read each widget before porting.

Part 1 — the library (`pty-tui`):
- `theme.rs`: `Theme` (13 slots bg1 bg2 bgHi bgAc fg1 fg2 fgAc fgMu ok warn err info border, each Option<Rgb>),
  the 11 built-in themes from colors.ts:289-357 incl. `terminal` (all None → ratatui `Color::Reset`), 9 semantic
  tokens → slot map (tokens.ts:20-30), `resolve(Color) -> ratatui::style::Color`, `theme_tokens()` serializer,
  `theme_to_palette()` → 16 colors for libghostty `set_default_color_palette` (builders.ts:35-61).
- `focus.rs`: `FocusStack` with scopes `{id, active: Fn -> bool, on_key, on_mouse}`, push → guard, innermost-first
  dispatch over a snapshot (focus.ts:53-134).
- `fuzzy.rs`: `fuzzy_match(query, target) -> Option<score>` (fuzzy.ts:19-67, same scoring).
- `input.rs`: map crossterm `KeyEvent`/`MouseEvent` to a `KeyEvent{name, char, ctrl, alt, shift}` with Node's names
  (`up down left right home end pageup pagedown delete tab backtab return escape backspace`, ctrl+letter, alt+char,
  kitty CSI-u decoded by crossterm), `MouseEvent{action press|release|drag|move|scrollUp|scrollDown, button, x, y,
  mods}`; enable kitty flags + SGR mouse + bracketed paste on start.
- `line_edit.rs`: `TextFieldState{text, cursor}` + `apply_text_key` (form.ts:50-118: backspace, delete, arrows,
  alt-word motion, alt+b/f, home/end, ctrl+a/e/u/w/k, printable) + a render fn producing spans with an inverse cursor.
- `scroll.rs`: `ScrollRegion{offset, selected, total, viewport}` and the pure ops (scrollable.ts); grouped selectable
  list rendering with section headers (selection counts items only).
- `app.rs`: `App::run(config)` — enter (alt screen, raw, hide cursor, mouse/paste/kitty), render loop on a channel of
  events (input, tick 1 s, resize, `Dirty` from handles), `pause()/resume()` (leave the terminal fully, hand it to an
  in-process `pty_core::client::attach`, re-enter with a full redraw), global key hook, default ctrl+c → exit 130,
  overlay rendering via `Clear`, `quit()`; synchronized output via crossterm `BeginSynchronizedUpdate`.
- `pane.rs`: `PtyPane` widget: takes a `&CellGrid` (from `TerminalHandle::snapshot(scroll_offset)`) and renders into
  the ratatui `Buffer` with `Color::Indexed` preserved, border (4 styles) + title, focus vs muted border color,
  content-anchored selection `{start_row, start_col, end_row, end_col, scroll_offset}` inverted, cursor reported only
  when focused and on screen; a `PtyView` flex widget without chrome; a per-handle cell cache keyed by `rev`.
- `widgets/`: ALL 28 from node-testing-tui.md 2.3, each with the same state-first shape (you own the state, widgets are
  pure render + pure key dispatch) and Node's key maps and outputs: tree, date-picker, form, markdown, text-area,
  virtual-list, stream-view, tabs, confirm, toast, command-palette, command-registry, table, help-overlay,
  prompt-bar, toolbar, sparkline, bar-chart, badge, breadcrumbs, progress-bars, accordion, action-list-item,
  code-block, message, select, pty-pane (above), plus `canvas` (free-form draw context). Where ratatui has a built-in
  (Table, Tabs, Sparkline, BarChart, Gauge, Paragraph, List, Scrollbar) wrap it so the widget keeps Node's state and
  keys; do not re-implement rendering that ratatui already does. Each widget gets a test ported from its Node
  `tests/widgets-*.test.ts` / `tests/<widget>.test.ts` (render to a `Buffer` and assert cell text; key dispatch
  transitions). Widget docs: one doc comment per widget with the key map.

Part 2 — the session manager (`crates/pty/src/interactive/`), everything in node-testing-tui.md section 3 and
docs/parity.md §9: nesting guard (three-line text, `--force`), list panel titled `pty`, filter line `  Filter: ` with
inverse cursor and `(type to filter)` dim placeholder and `#k=v` tag filter suffix, item rows (`▸ ` marker, `●/○`,
`displayName (id)` or `id`, ` [permanent]`, inline non-reserved `#k=v`, `  <~cwd>  <displayCommand>`,
`(exited Xs/m/h/d ago)`), `+ Create new session...` per group, section headers only when relay hosts exist
(`Local`, `<host> (n)`), footer `↑↓ select  ⏎ attach  ctrl+g theme (<name>)  q quit`, fuzzy filter over
name/displayName/cwd/displayCommand with running +100000 and name +10000 bonuses and `host/session` syntax, keys
(up/down clamped; return → attach running | restart exited/vanished then attach | create | remote create/attach via
`pty-relay connect ...` with the app paused; escape clears filter else quits; `q` quits when filter empty; ctrl+c;
ctrl+g cycles theme persisted to `<root>/theme`), attach-and-return (pause app, `client::attach` with on_detach →
re-list, clamp, resume; on_exit → wait 200 ms, re-list, resume; filter and selection preserved), one-key create
(random id, `$SHELL` else bash, `$HOME`, `--filter-tag` tags, creation lock error to stderr), restart of
exited/vanished reusing stored fields, `--preselect-new`, `--filter-tag` inheritance, `pty-relay ls --json` refresh
(10 s, async, at start and after remote actions), 1 s `list_sessions` refresh paused during attach, status semantics
running/exited/vanished.

Tests: `crates/pty/tests/interactive.rs` porting tests/tui.test.ts through `pty_testkit::Session::spawn` of the
built binary (box fills width at 80/120/200 cols, path not truncated at 120, works at 60; filter; kitty CSI-u
escape clears then quits; Ctrl+\ detaches and returns to the list; multiple attach/detach cycles; keystrokes not
doubled; external create/exit/tag refresh; `--preselect-new`; `--filter-tag`; empty state shows only the create row).
