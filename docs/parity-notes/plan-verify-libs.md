# Plan half B: verification, libraries, cutover

Facts checked on this host (2026-08-29): `which pty` → `~/.local/bin/pty` → the Node checkout's
`bin/pty`; `pty --version` → `0.12.0+500eab2`. crossterm 0.29 has `PushKeyboardEnhancementFlags`
(kitty CSI-u: `DISAMBIGUATE_ESCAPE_CODES`, `REPORT_ALL_KEYS_AS_ESCAPE_CODES`) and SGR mouse
(`EnableMouseCapture`, `MouseEventKind::{Down,Up,Drag,Moved,ScrollUp,ScrollDown}`). st2's
`flake.nix` pins the Node pty at line 9 and consumes `pty.packages.${system}.default` at 193, 307,
323–326, 443 (checks: parked-recovery, pty-fleet-contract, hooks-replacement).

---

## B1. Conformance suite (`conformance/`) — the parity oracle

**Goal.** One suite that runs against ANY `pty` binary. Green against the Node pty proves the
suite is faithful. Green against the Rust pty proves parity. Same run, two binaries.

**Where.** `pty-rust/crates/pty-conformance/` — a Rust crate with `tests/*.rs`, no library code
except `src/harness.rs`. It is a workspace member; `cargo test -p pty-conformance` runs it.

**Binary selection.** `PTY_TEST_BIN` (absolute path). Unset → the workspace's own
`target/{profile}/pty` (via `CARGO_BIN_EXE_pty` is not available across crates, so the harness
resolves `env!("CARGO_MANIFEST_DIR")/../../target/<profile>/pty`). `PTY_TEST_BIN=$(which pty)`
selects the Node pty. A `justfile`/`Makefile` target `conformance-both` runs both and diffs the
two JUnit reports.

**Harness (`src/harness.rs`), one struct `Rig`:**
- `Rig::new()` → `tempdir` under `/tmp/pc-XXXX` (short: socket paths ≤ 104 bytes), `PTY_ROOT` =
  that dir, `PTY_ROOT_LEGACY_SILENT=1`, scrubs `PTY_SESSION`, `PTY_SESSION_GENERATION`,
  `PTY_SESSION_DIR`, `PTY_REAP_ON_EXIT`, `PTY_SERVER_CONFIG`, `ST_AGENT`, `ST_ROOT`, `NO_COLOR`;
  sets `TERM=xterm-256color`, `HOME`=tempdir/home (so `~` paths and the default root are
  deterministic), `PATH` preserved.
- `rig.pty(&[args]) -> Out { status: i32, stdout: String, stderr: String }` — `std::process::Command`
  with a 30 s timeout (kill on expiry, mark failed). `rig.pty_env(extra_env, args)`,
  `rig.pty_stdin(bytes, args)` (for `metadata patch`), `rig.pty_tty(args, rows, cols)` — runs the
  CLI inside a real PTY via `pty-testkit::Session::spawn` (needed for `attach`, `peek -f`, the
  dead-session prompt, `restart` prompt, the machine-stream tests).
- `rig.daemon(id, cmd, opts) -> Daemon` — `pty run -d --id <id> [--no-display-name] [--tag k=v]
  [-e] [--env] [--unset-env] [--isolate-env] [--cwd] -- cmd...`; waits for `<id>.sock` and
  `<id>.json` (≤ 30 s); `Daemon::socket()` opens a `UnixStream`; `Daemon::meta()` parses
  `<id>.json` as `serde_json::Value` (not a typed struct — unknown fields must be visible).
- `rig.connect(id) -> Conn` — protocol client from `pty-protocol` (framing) with helpers
  `attach(rows, cols)`, `peek(plain, full)`, `resize`, `data`, `detach`, `status`,
  `next_packet(timeout)`, `collect_until(EXIT|timeout)`, `sequence() -> Vec<MessageType>`.
- Timing: every wait is a poll with a deadline (default 10 s; `PC_SLOW=1` doubles). Node's
  constants are asserted with tolerance: "within 150 ms" → poll at 10 ms, assert `< 250 ms`;
  "no SIGWINCH within 150 ms" → sleep 150 ms then assert. 80 ms settle is asserted by ordering,
  never by wall clock.
- Teardown (`Drop`): SIGTERM every pid in `<root>/*.pid`, wait 2 s, SIGKILL survivors; `lsof
  +D root` not required — pids are enough because every daemon writes its pid. Sessions are
  serialized with a process-wide mutex only for tests that read `ps` (`process-title`,
  `process-tree`); everything else runs in parallel (`--test-threads` default).
- Assertions use `assert_eq!` on exact strings when Node pins an exact string, `contains` when
  Node's test uses `toContain`, regex when Node uses `toMatch`. The harness has `expect_json(&out)
  -> Value` and `expect_lines(&out)`.

**Organization.** One Rust test file per Node test file, same base name, in the same order as
`node-cli-surface.md` §6 and `node-daemon-protocol-disk.md`. Each `#[test]` carries a doc
comment `/// node: tests/<file>.test.ts:<line>` so the mapping is greppable. A generated
`docs/conformance.md` table (file → tests → kind → Node lines) is produced by
`scripts/conformance-map.rs` from those comments and committed.

**Kind per Node suite** (120 files):

| Kind | Node suites | How |
|---|---|---|
| **cli** (black-box, 48) | help, completions, version, pty-root, display-name, spawn-options, nesting, nesting-prevention, restart-*, restart-guardrail, kill-wait, rm-*, exit-reap, exit-signal, gc, gc-parent-child, gc-flap-clear-badge-root-len (root-length and badge halves only), list-filters, list-purity, list-liveness-budget, list-live-session-race, tags, tags-helpers (CLI half), tag-mutate, tag-bulk, tag-multi, metadata-events (CLI half), events-emit, events (CLI half), peek-wait, send-paste, seq-delay, stats-cli, up-down, up-name-decouple, ptyfile (CLI half), env-isolation (CLI half), exec, attach-no-restart, attach-stream, wrapper-signal-forwarding, process-title, process-tree (via `kill` on a 3-deep tree; the port of `pty-kill-releases-socket-test`), spawn-bundle-fallback (only the `pty run -d --id --no-display-name ... --unset-env` argv shape), shutdown-backstop (`PTY_SHUTDOWN_DEADLINE_MS`), spawner-pid-watchdog (`PTY_SPAWNER_PID` set on `pty run`), parity-fixtures, parity-shapes, parity-node-reference | `rig.pty`, `rig.pty_tty`, `rig.daemon` |
| **protocol** (socket, 12) | integration (sync, roles, geometry, malformed packets, stats), effective-geometry, connection (the attach/screen/exit/geometry parts), attach-stream (frame checks), screen-replay-altscreen, scrollback-fidelity, terminal-queries (responses), sanitize (bytes emitted by `attach`), exit-event-race, atomic-writes (concurrent CLI writers), security-fixes (lock steal via `<id>.lock` files), recovery → **skipped: deferred** | `rig.daemon` + `rig.connect` |
| **unit** (already ported, 9) | keys, duration, protocol, ptyfile, input-parse, mouse-parse, send-paste (wrapping), terminal-queries (strip), env-isolation (`build_spawn_env`) | stay in `pty-testkit`/crate unit tests |
| **not portable** (51) | 39 TUI widget/framework files, tui.test.ts (drives the Node manager), screenshot.test.ts (in-process xterm), pty-handle, pty-pane, pty-root (TUI part), ratatui-compat (in-process `Session.server`) → re-expressed as **protocol** cases in `ratatui_compat.rs` (SCREEN payload checks through the socket, same scenarios), codex-integration, remote-* (need `fabric`; run as **cli** with a fake `fabric` shim on PATH that prints a socket path — the Node tests do exactly this, port the shim), disk-layout-docs (Node doc lint) | listed in `docs/conformance.md` with the reason |

**Decision-gated assertions.** A test that depends on a recorded decision (B1.3) is written
twice: `#[test] fn x_node()` gated on `rig.is_node()` (parsed from `pty --version`, `+` and no
`-rust`) and `fn x_rust()` gated on `rig.is_rust()`. Both point at the decision file in their
doc comment. Everything else has one body and no gate. The count of gated tests is the parity
debt and is printed by `conformance-map`.

**Fixtures.** `tests/fixtures/parity/{screens,shapes}.json` stay Node-owned and byte-identical
(a test `cmp`s them against the Node checkout when `PTY_NODE_CHECKOUT` is set). New fixtures for
issue #4 live in `conformance/fixtures/` (Rust-owned, versioned `v1`, JSON):
- `bytes-split.json`: each UTF-8 scalar of a sample (`é`, `€`, `😀`, CJK) split at every byte
  boundary across DATA frames; expected plain screen.
- `escape-split.json`: `ESC[31m`, `ESC[?1049h`, OSC 0 title, CSI-u, split at every byte.
- `raw-bytes.json`: DATA with 0x80–0xff raw; expected: **Node** re-encodes as UTF-8 (`toString()`
  then `write`), **Rust** writes bytes. This is decision 0001 (below); the fixture records both.
- `attach-identity.json`: run `--id a`, exit, run `--id a` again; a new attach reaches the
  replacement (`STATUS.daemon.pid` differs); no generation needed.
- `late-events.json`: client keeps an old socket open across a replacement; frames from the
  old socket after the new attach must not appear in the new attempt's sequence.
- `frame-limits.json`: 100 kB DATA intact; declared length `MAX+1` → connection dropped
  (server) / `InvalidData` (client); 3 packets in one write.
- `slow-reader.json`: one attached client never reads; a second client still receives DATA
  within 1 s (daemon per-client queues; the Node daemon relies on socket buffers — the fixture
  states 1 MB of output as the bound).
- Each fixture has a loader in `conformance/tests/fixtures.rs` and is also readable by the TS
  package (B3) so both harnesses run it.

**Decision records.** `docs/decisions/NNNN-<slug>.md`, one per observable difference, template:

```
# NNNN <title>
Status: accepted | superseded by NNNN
Node behavior: ...
Rust behavior: ...
Why: ...
Client effect: ...
Test: conformance/tests/<file>.rs::<fn> (gated) — and the fixture if any
Migration / negotiation: none | ...
```
Known records to write during the build: 0001 raw DATA bytes (UTF-8 re-encode vs raw);
0002 ANSI serialization differs byte-wise, equivalent after re-parse (proof: parse both with
libghostty, compare plain+styles); 0003 DA2 string (`>0;382;0c` Node vs libghostty
`>1;0;0c`) — **Rust answers DA2 itself with the Node string** (override libghostty's reply),
so no record if the override works; 0004 OSC 10/11/4 answers — Rust answers itself with Node's
constants, no record if it works; 0005 plain-text trailing-space/trailing-row rule (pinned by
fixtures; expect no difference, record only if one appears); 0006 emoji/grapheme width
(libghostty unicode tables vs xterm default); 0007 reflow on resize; 0008 `scrollbackUsed`
counting. Records 0003/0004 are written only if the override fails. Rule: **a difference the
CLI can hide (query answers, trimming) is fixed in Rust, not recorded.**

**Done condition for "100% parity"** (checkable):
1. `PTY_TEST_BIN=<node> cargo test -p pty-conformance` green (proves the suite).
2. `PTY_TEST_BIN=<rust> cargo test -p pty-conformance` green, with every `_node`/`_rust`-gated
   pair pointing at an accepted decision record; `conformance-map` prints the gated count and
   `docs/conformance.md` lists every Node suite as cli/protocol/unit/not-portable with a reason.
3. The mixed-fleet rig (B6) green in both directions.
4. Every consumer row in `docs/parity.md` §2 has a named check in B6 that ran green.
5. `docs/parity.md` status column has no `missing`/`partial` except items marked dropped/deferred.

**Dependencies.** `pty-protocol` crate (half A) for the `Conn` helper; nothing else — the suite
can be written against the Node binary from day one, before any Rust feature lands. **Write the
suite first, file by file, and watch it go red against Rust; then build half A to green.**

---

## B2. Rust testkit: daemon-backed sessions

**Goal.** `pty-testkit::Session` gains the Node `Session.server` mode.

**Files.** `crates/pty-testkit/src/{session.rs,server_session.rs,screenshot.rs,keys.rs}`.

**Design.**
- `Session::server(cmd, args, ServerOptions{name,rows,cols,cwd,env}) -> Result<Session>` spawns
  the daemon through `pty-client::spawn_session` (the same code `pty run -d` uses, in-process —
  no CLI fork), connects with `pty-client::Connection`, and holds its own libghostty
  `Terminal` to render SCREEN/DATA (so `screenshot()` shape is identical in both modes).
- `attach()` writes ATTACH and resolves when the first SCREEN has been parsed (no fixed delay);
  `reconnect()` = destroy + reconnect + `terminal.reset()` + attach; `Session::connect_to_existing
  (&Session, rows, cols)`; `resize()` sends RESIZE, `rows()/cols()` follow GEOMETRY;
  `has_exited()`, `exit_code()`, `name()`, `close()` (kill daemon only if this session owns it).
- Defaults: `wait_for_*` get `_default` variants with 10 000 ms and a 50 ms poll to match
  Node's numbers; explicit-timeout variants stay.
- `keys::resolve_key` accepts `+`, `-`, `_` separators and `C-` (port of `keys.ts:20-64`,
  already pinned in `tests/keys.test.ts`); `press("ctrl-c")` works.
- Screenshot: `lines` right-trim rule follows decision 0005's outcome; both modes share
  `capture()`.

**Done.** `tests/server_session.rs` ports `screenshot.test.ts`'s server-mode cases (attach,
reconnect replay, resize min-wins via a second client, immediate attach, exit code) against the
Rust daemon; `cargo test -p pty-testkit` green; docs in `crates/pty-testkit/README.md`.

**Depends on** half A's `pty-client` and daemon GEOMETRY/sync work.

---

## B3. TypeScript testing package (`packages/pty-testing`)

**Location.** `pty-rust/packages/pty-testing/` (npm name `@compoundingtech/pty-testing`).
Inside pty-rust because its engine is this binary and its fixtures are shared with B1;
the Node repo stays a reference. A root `package.json` with `workspaces: ["packages/*"]`.

**Engine contract.** The package never links native code. It needs a `pty` binary:
`PTY_BIN` env → else `pty` on PATH → else `<pkg>/bin/` if a platform build is bundled later
(not now). It refuses a binary whose `--version` lacks `-rust` unless `PTY_TESTING_ALLOW_NODE=1`
(the Node binary works too, but the point is one engine).

**API (TypeScript, ESM, no deps beyond Node built-ins; vitest is a peer for the runner only):**
```ts
export interface Screenshot { lines: string[]; text: string; ansi: string }
export interface SpawnOptions { rows?; cols?; cwd?; env?: Record<string,string>; name?: string }
export class Session {
  static async spawn(command: string, args?: string[], opts?: SpawnOptions): Promise<Session>
  static async connect(name: string, opts?: {rows?; cols?; root?: string}): Promise<Session>  // attach to an existing session
  static async connectToExisting(s: Session, opts?): Promise<Session>
  readonly name: string; readonly root: string; get rows(); get cols(); get hasExited(); get exitCode()
  sendKeys(s: string): void; type(s: string): void; press(key: string): void   // key table = Node keys.ts (ported to TS here, or `pty send --seq key:` semantics)
  async screenshot(): Promise<Screenshot>          // PEEK plain + PEEK ansi (two frames on a command socket)
  async waitForText(t, timeoutMs = 10000): Promise<Screenshot>; waitForAbsent; waitFor(pred, ms, desc)
  resize(rows, cols): void; async reconnect(): Promise<void>; async attach(): Promise<void>
  async close(): Promise<void>                      // pty rm (kills) + tempdir cleanup when owned
}
```
- `spawn` = create a per-Session temp `PTY_ROOT` (`/tmp/pt-XXXX`), run
  `pty run -d -e --no-display-name --id <8 chars> --cwd <cwd> [--env K=V]* --rows R --cols C -- cmd
  args` (`--rows/--cols` is the Rust extension; with a Node engine the package sets rows/cols by
  ATTACH instead), wait for `<id>.sock`, open one attached socket (ATTACH rows×cols) whose
  DATA is drained into nothing but whose GEOMETRY/EXIT update state, and open short-lived
  command sockets for PEEK. `screenshot()` = PEEK plain → `lines`/`text` (split, pop trailing
  empties), PEEK ansi → `ansi`. `sendKeys` writes DATA on the attached socket.
- Framing: `src/protocol.ts` is a 120-line reimplementation of the 5-byte frame + `PacketReader`
  (same as Node's; MIT, same authors) — no import from the Node package.
- `waitFor*` poll every 50 ms; error text `Timed out after Nms waiting for "..."\nScreen:\n...`
  (same as Node so existing tests port by search-and-replace of the import).
- vitest: `vitest.config.ts` example + `setup/isolate.ts` that scrubs `PTY_*` env; the package
  exports `pty-testing/vitest` with a global-setup that kills leaked daemons from `/tmp/pt-*`.
- Executable docs: `docs/testing.md` with ```ts test``` fences run by `scripts/verify-docs.ts`
  (copy the Node script's approach: extract fences, run under vitest).
- Build/publish: `tsc` to `dist/`, `npm publish` from CI on a tag `pty-testing-vX`; consumers
  `npm i -D @compoundingtech/pty-testing` and need `pty` on PATH (nix: the pty-rust package).
- Fixtures: the package's own suite runs B1's `conformance/fixtures/*.json` screen cases
  (`bytes-split`, `escape-split`) through this API, so a screenshot from TS equals one from
  Rust for the same fixture.

**Done.** `npm test` green in `packages/pty-testing` against the Rust binary; the Node
`tests/screenshot.test.ts` cases (ls, colors, vim, nano, resize+tput, CJK/emoji, alt screen,
multi-client, high throughput) ported to `packages/pty-testing/test/screenshot.test.ts` and
green; `verify-docs` green; one downstream repo (st2 or evals) runs a smoke test with it.

**Depends on** half A `run` flags (`-e`, `--env`, `--rows/--cols`), daemon GEOMETRY.

---

## B4. TUI library `pty-tui` and the session manager

**Crate.** `crates/pty-tui` (library) + the manager as a module of the `pty` binary
(`crates/pty/src/interactive/`). Deps: `ratatui 0.29+`, `crossterm 0.29` (kitty flags + SGR
mouse + bracketed paste confirmed), `libghostty-vt`, `pty-client`, `pty-terminal`.

**Ratatui gives:** `Buffer`/`Cell` with fg/bg `Color::{Rgb,Indexed,Reset}` (palette index
preserved — the Node buffer's `fgIndex` requirement), diff-based rendering, `Layout` with
`Constraint::{Length,Min,Percentage,Fill}` (flex rows/columns/spacers), `Block` with 4 border
sets + title + bottom title (the Node panel with `footerTitle`), `Paragraph` with wrap and
spans, `List`/`Table`/`Tabs`/`Gauge`/`Sparkline`/`BarChart`/`Scrollbar`/`Clear` (overlay), and
`crossterm` raw mode/alt screen/mouse/paste/kitty. Synchronized output (`?2026`) via
`crossterm::terminal::{BeginSynchronizedUpdate,EndSynchronizedUpdate}`.

**We add (`pty-tui`):**
- `PtyPane` widget: renders a `pty-terminal::Snapshot` (typed cells from libghostty
  `GridRef::cell()/style()`, `Row::is_wrapped()`, cursor, alt-screen, kitty flags, mouse mode)
  into a ratatui `Buffer` with palette indices preserved; border/title/focus color; selection
  highlight with scroll translation; returns the cursor position for the host to set.
- `Theme` (13 slots) + 9 semantic tokens → `ratatui::style::Color`; built-in themes incl.
  `terminal` (all `Reset`); `theme_to_palette()` for embedded terminals (sets libghostty
  default palette via `set_default_color_palette`).
- `FocusStack` (ratatui has none): scopes with `active()` predicate, innermost-first key/mouse
  dispatch.
- `fuzzy::score` (port of `fuzzy.ts`).
- `LineEdit` (readline single-line: word motion, ctrl+a/e/u/w/k, inverse cursor cell).
- `App` runner: enter/leave terminal, `pause()/resume()` that hands the tty to an in-process
  `pty-client::attach` and forces a full redraw on resume, 1 s tick, resize, ctrl+c → exit 130,
  overlay compositing via `Clear`.
- `ScrollRegion` + grouped selectable list with section headers (selection counts items only).

**Widget map (28 Node widgets):**
| ratatui built-in (use as is) | table, tabs, sparkline, bar-chart, progress-bars/gauge, code-block (Paragraph + line numbers), message (Paragraph in Block), badge (styled Span), breadcrumbs (Line of Spans), toast (Paragraph + Clear), help-overlay (Table in Clear) |
| port now (manager needs them) | virtual-list (windowed List), form/LineEdit, select (dropdown), confirm, command-palette (needs fuzzy + LineEdit), pty-pane, tree glyphs are trivial |
| defer until the agent program needs them | date-picker, markdown, text-area (multi-line composer), stream-view, command-registry, prompt-bar, toolbar, accordion, action-list-item |
This split is the scope question for the maintainer: the manager needs only list, LineEdit, panel,
footer, overlay. Everything in "port now" beyond that is for the agent program and can move to
"defer".

**Session manager (`pty` no-arg / `i` / `interactive`)**, feature by feature from parity.md §9:
nesting guard text; list with `▸`, `●/○`, `displayName (id)`, `[permanent]`, inline non-reserved
tags, `~` cwd, command, `(exited 2h ago)`; fuzzy filter with running bonus and `host/session`
syntax; relay hosts via `pty-relay ls --json` (10 s, async) grouped with headers; keys
`↑↓ ⏎ esc q ctrl+c ctrl+g`; attach-and-return with filter preserved; one-key create (random id,
`$SHELL`, `$HOME`, `--filter-tag` tags); restart of exited/vanished; `--preselect-new`; theme
file `<root>/theme`; 1 s refresh paused during attach; remote attach/spawn by shelling to
`pty-relay connect` with the app paused.

**Done.** `pty` with no args passes a port of `tests/tui.test.ts` (driven through
`pty-testkit::Session::spawn` of the Rust binary: list rendering at 60/80/120/200 cols,
filter, kitty CSI-u escape, attach/detach cycles without doubled keystrokes, external
create/exit/tag refresh, `--preselect-new`, `--filter-tag`), in `crates/pty/tests/interactive.rs`.

**Depends on** B5 (`pty-terminal` snapshot) and half A's client crate.

---

## B5. Embedding API (`pty-terminal` + `pty-client`)

**Goal.** One `TerminalHandle` for attached sessions and spawned children (issues #1, #3), used
by the CLI (`attach`, `peek -f`, testkit), by `pty-tui::PtyPane`, by deskset, and by Fractal.

**Crates.** `pty-protocol` (frames only, no I/O), `pty-client` (registry paths, `Connection`,
`spawn_session`, `attach` loop, reconnect, typed events), `pty-terminal` (the actor).

**Design.**
- `pty-terminal::TerminalActor`: owns the `!Send` libghostty `Terminal` on its own thread;
  input via `mpsc::Sender<Cmd>` (`Write(bytes)`, `Resize`, `Reset`, `SetPalette`,
  `Snapshot(reply)`), output via `broadcast`-style `Sender<Event>` (`Dirty{rev}`,
  `Title`, `Bell`, `Exited(code)`, `Geometry{rows,cols}`). `Snapshot` is a plain `Send` struct:
  `rows: Vec<RowSnap { cells: Vec<CellSnap>, wrapped: bool }>` for the requested viewport
  (`scroll_offset`), `cursor: (row, col, visible)`, `alt_screen`, `mouse_mode`, `kitty_flags:
  Vec<u8>`, `bracketed_paste`, `scrollback_len`, `base_row`, `title`. `CellSnap { text:
  CompactString, fg/bg: ColorSnap::{Default, Indexed(u8), Rgb}, bold, dim, italic, underline,
  inverse, wide: Narrow|Wide|Spacer }` built from `GridRef::cell()/style()` + `Row::is_wrapped()`.
  Snapshots are cached per `rev`; `PtyPane` asks only when `rev` changed.
- `TerminalHandle::attach(SessionRef{root, id}, AttachOptions{rows, cols, readonly})` → connects,
  ATTACH, feeds SCREEN/DATA into the actor, GEOMETRY updates size, EXIT → `Exited`. Every
  attempt gets a local `AttemptId`; frames tagged with an older id are dropped before they reach
  the actor (issue #3 late-event rule). `reconnect()` starts a new attempt. Root+id is the
  identity; generation is exposed on `SessionInfo` only as diagnostics.
- `TerminalHandle::spawn(cmd, args, SpawnOptions{rows, cols, cwd, env, scrollback})` → owns the
  child via `portable-pty`; `close()` kills and reaps.
- Both return the same `TerminalHandle { write, resize, snapshot(offset), events(), rev(),
  kill/close, exited, cols, rows }`.
- The daemon itself is the third user of `TerminalActor` (half A): one implementation of
  screen serialization, query answers, and mode tracking.
- deskset: `pty-wire` maps 1:1 onto `pty-protocol` + `pty-client::{Connection, registry,
  stats}` (deskset's `AttachedSession` on tokio → `pty-client` is sync; offer
  `pty-client::tokio` feature with an `AsyncConnection` wrapper, because deskset is tokio).
  `pty-cli` (typed wrappers over the executable: run/kill/rm/restart/rename/tag/list/stats/
  events follower, env scrub of `PTY_SESSION*`, stdin null) becomes `pty-client::cli` — the
  same argv builders the Rust `pty` binary uses, so they cannot drift. Deskset then depends on
  `pty-client` by git path and deletes its two crates.
- Fractal (issue #3): needs `attach`/`spawn`, typed cells, wrapped flags, cursor, modes,
  scrollback reads by offset, explicit attach readiness (first SCREEN parsed), reconnect
  semantics, `!Send` boundary hidden. All covered above. No `create_tty`.

**Done.** `crates/pty-terminal/tests`: snapshot correctness against fixtures (bytes-split,
escape-split, altscreen, kitty stack, wide chars); `crates/pty-client/tests`: attach identity +
late-event rejection fixtures (B1) against the Rust daemon; `PtyPane` renders a snapshot into a
ratatui buffer with indices preserved (port of `pty-handle.test.ts` and `pty-pane.test.ts`
cases); deskset builds against `pty-client` in a branch (its own owner does that; we provide
the crate and a migration note in `docs/embedding.md`).

---

## B6. Cutover

**Packaging first.** `flake.nix` in pty-rust: `rustPlatform.buildRustPackage` (the st2 pattern,
`cargoLock.lockFile`), `nativeBuildInputs = [ zig_0_15 ]`, and the Ghostty source that
`libghostty-vt-sys` fetches at build time provided as a fixed-output derivation exported via
`GHOSTTY_SRC` (check the sys crate's build script env var; if it has none, vendor the source
into the derivation and patch `build.rs` in a nix `postPatch`). Outputs: `packages.default` =
`pty` with completions installed (`installShellCompletion` of the three vendored files),
`checks.conformance` = B1 against the built binary, `checks.completions` = generator output ==
vendored files. Version = `0.13.<n>-rust+<short-sha>` from `Cargo.toml` + git (build.rs, like
st2's `LocalStamp`).

**st2 flake.** `pty.url = "github:compoundingtech/pty-rust/<sha>"`; every
`pty.packages.${system}.default` reference stays valid (same attribute). The
`pty-fleet-contract` check (lines 302–341) runs the Node package's vitest from
`lib/pty/node_modules` — rewrite it to run `pty list --json` against the built binary only
(the vitest half belongs to the Node repo). One commit in st2, on a branch, reported with SHA.

**Staged rollout on the first host (the operator runs these; each step has a check):**
1. `ST2_PTY_BIN=<rust build>` on one agent (the existing mechanism):
   check = `st2 agents --json --enrich` shows it `running`, `pty list --json` shows tags and
   `createdAt`, a DING round-trip works, `st2 doctor` clean, no presentation-drift `metadata
   patch` per pass (grep the supervisor log for `metadata patch`).
2. All agents on the first host via the catalog env, still with the Node `pty` on PATH for `list/kill/rm`:
   mixed fleet for one day; check = every agent reconciles, the supervisor reports no restarts
   caused by liveness misreads (the `run.rs` Indeterminate path).
3. Rust `pty` on PATH (nix profile / st2 flake): remove `ST2_PTY_BIN`; check = same as 1 for the
   whole fleet plus the eval cells below.
4. The other hosts: same three steps, after the first host has run a week.
Rollback at any step: unset `ST2_PTY_BIN` / revert the flake input / `nix profile rollback`;
sessions hosted by the other binary keep running because the registry is shared — `list`,
`kill`, `rm` work across implementations (this is what B6's rig proves before step 1).

**Consumer verification runs (all must be green before step 3):**
- st2: `cargo test` in the st2 worktree with `PATH` pointing at the Rust `pty`
  (`tests/pty.rs`, `tests/atomic_pty_snapshot.rs`, `tests/eval_run_e2e.rs` use the real
  binary); `st2 eval ./cells/pty-send-peek/`, `./cells/pty-attach-only/`,
  `./cells/pty-attach-machine-stream/` from the evals repo (the last two are marked blocked on
  Node PRs; run them and record which assertions the Rust binary meets — the machine-stream
  cell is the acceptance test for `--attach-stream-fd-v1` + `remote-serve --stdio`).
- pty-relay: `integration/*.test.ts` with `pty` on PATH = Rust (`remote-ops`, `tags`, `events`,
  `ssh-ls`, `e2e`); these are the relay's own suites and need no change.
- ding + smalltalk: `ding` integration tests (`tests/integration.rs`) and smalltalk
  `tests/integration/ding.test.ts` with `PTY_SESSION_DIR` set — proves the deprecated alias and
  `peek --plain` / `send --paste` shapes.
- deskset: `cargo test -p pty-wire -p pty-cli` against the Rust binary before its migration
  (their tests pin Node 0.12 shapes: `list --json`, `stats --json`, `events --all --json`).
- fabric: `fabric expose pty-remote --exec -- pty remote-serve --stdio` on one host, `pty list
  --remote <host>` from another.

**Mixed-fleet rig** (`conformance/tests/mixed.rs`, needs two binaries): `PTY_TEST_BIN` +
`PTY_TEST_BIN_PEER`. Cases: peer `run -d` then ours `list --json` (fields, status, pid),
`peek`, `send --seq`, `stats --json`, `attach` sequence `GEOMETRY→SCREEN→DATA→EXIT` through
the peer daemon, `metadata patch` on a peer-written file preserves unknown fields (assert
`generation`/`recovery` survive a Rust rewrite), `kill`, `rm`, lock contention (`<id>.lock` held
by the peer's pid → ours reports busy), `gc` sweep of a peer-preserved exit. Run both
orientations in CI on the Linux build host (the Node binary is on PATH there).

**Node package afterwards.** Reference only: its tests are the oracle for B1 and its
`tests/fixtures/parity` stays the owner of the shared fixtures until issue #4 moves ownership
here (then the Node repo vendors from us; that is a note to Johannes, not our change).

**Done.** Steps 1–3 completed on the first host with checks green; st2 flake merged pointing at pty-rust;
`docs/parity.md` §14 rows `have`; rollback exercised once on purpose at step 1.

---

## Order and parallelism

```
B1 harness + first 10 cli files (help, version, pty-root, display-name, list-filters, tags,
   send-paste, seq-delay, stats-cli, kill-wait)  ──────────────┐  runs against Node from day 1
half A: pty-protocol / pty-client / registry (locks, generation, events)   ← B1 red → green
B5 pty-terminal actor + TerminalHandle  ──┬──  half A daemon rewrite uses it
B1 protocol files (integration, effective-geometry, screen-replay, scrollback, sanitize,
   attach-stream, ratatui-compat)          │  ← green when the daemon lands
B2 testkit server mode ────────────────────┤  after pty-client + daemon GEOMETRY
B3 TS package ─────────────────────────────┤  after run flags + GEOMETRY; independent of B4/B5
B4 pty-tui + manager ──────────────────────┘  after B5; independent of B2/B3
B1 remaining cli files (gc, up-down, tag-multi, events, exec, restart, rm, attach prompts,
   remote via fake fabric) ← as each verb lands in half A
B6 packaging (flake) ← any time after the workspace split; st2 flake + rollout ← last, after
   B1 green on both binaries and the mixed rig green.
```
Parallel lanes once half A's crate split exists: (1) half A verbs + B1 cli files in lockstep,
(2) B5 → B4, (3) B3, (4) B6 packaging. B2 is small and fits in lane 1 after the daemon lands.
