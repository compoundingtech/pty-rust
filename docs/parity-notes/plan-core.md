# Core plan: crates, daemon, registry, CLI, remote, packaging

Reference documents (scratchpad): `node-cli-surface.md` (CLI), `node-daemon-protocol-disk.md`
(daemon/protocol/disk/VRS), `rust-port-and-st2.md` (Rust today + st2 call sites). Node source
citations below are `pty/src/<file>:<lines>` in `~/src/github.com/compoundingtech/pty`.

## 0. Shape decisions (settled here, not menus)

**Workspace.** One Cargo workspace, four crates. Smaller than issue #1's five: `pty-protocol` and
`pty-client` merge, because every client-side consumer (CLI, testkit, deskset, TUI pane) needs both
framing and registry+socket lifecycle, and nothing needs framing alone.

```
Cargo.toml                 workspace (members below), version 0.13.0-rust
crates/pty-core/           framing + registry + locks + events + metadata + keys/paste/duration/
                           input/queries/ptyfile + client ops (attach loop, peek, send, status,
                           SessionConnection) + reconnect.  NO libghostty.  This is what deskset's
                           pty-wire + pty-cli become.
crates/pty-terminal/       the libghostty actor: owns Terminal (!Send) on one thread, typed
                           snapshots (cells, wrapped flags, cursor, modes, kitty stack, scrollback),
                           VT/plain serialization, query answers, terminal events. Used by the
                           daemon, the testkit, and the TUI pane/embedding handle.
crates/pty-testkit/        Session (spawn + daemon-backed), Screenshot, waits. Depends on both.
crates/pty/                the `pty` binary: CLI + daemon (`__daemon`) + remote-serve.
```

Rationale: `pty-core` has no Zig dependency, so deskset and the TS-package helper can build it in
seconds; `pty-terminal` isolates the only `!Send` object and the only C dependency; the daemon is
CLI-internal (Node keeps `server.ts` in the package too) and nothing else spawns it directly.

**CLI parsing: hand-rolled, module per command, one shared `Argv` cursor helper.** Not clap. Node's
grammar is irregular in ways clap cannot express without fighting it: `--root` consumed from anywhere
before dispatch; per-command "consume leading dash tokens, first other token is the ref, trailing
flags ignored" loops (`peek`, `events`); `--with-delay` only valid as the first token after the ref;
`--paste` removed from anywhere; `--flag=value` only in `gc`; unknown tokens silently ignored in
`list`; error texts that name the exact token. The pinned literals in `node-cli-surface.md` §6 are
the acceptance test; a bespoke parser that mirrors `cli.ts` loop-for-loop reproduces them with no
translation layer. Help text is vendored verbatim (§3), so clap's help generation buys nothing.

**Daemon concurrency: keep the actor thread; add a timer channel.** The `!Send` terminal stays on the
`daemon::run` thread. All ordering problems become trivial because `vt_write` is synchronous: when
the actor decides to cut a SCREEN, every PTY byte it has received is already parsed. The 80 ms
settle is an actor-owned deadline serviced with `recv_timeout`.

**Locks: implement Node's file-lock protocol exactly.** Rust and Node writers share `$PTY_ROOT`
during migration; a Rust writer that skips `<name>.lock` can interleave with a Node
`mutateMetadataUnderLock` and lose its write. Same `O_CREAT|O_EXCL`, holder pid, single stale steal.

**Version:** `0.13.0-rust+<short-sha>` from `build.rs` (`git rev-parse --short HEAD`), overridable
by `PTY_BUILD_SHA` for nix. `pty version` prints it; `pty --version`, `-v`, `-V` alias.

## 1. Work packages

### WP1 — Workspace restructure (owner: one agent, first, alone)
Goal: move code into the four crates with no behavior change; every existing test still green.
- Create `crates/pty-core/src/{lib,protocol,registry,keys,paste,duration,input,queries,ptyfile,
  client}.rs` from today's `src/`. Create `crates/pty-terminal/src/{lib,actor,snapshot,serialize}.rs`
  from `src/screenshot.rs` + the terminal parts of `daemon.rs`/`session.rs`. Create
  `crates/pty-testkit/src/{lib,session,screenshot}.rs`. Create `crates/pty/src/{main,daemon,
  cli/mod}.rs` from `src/bin/pty.rs` + `src/daemon.rs`.
- Move `tests/*` under the crate that owns them (`cli_e2e.rs`, `parity*.rs`, `registry_liveness.rs`
  → `crates/pty/tests`; pure module tests → `crates/pty-core/tests`; terminal tests →
  `crates/pty-testkit/tests`). Keep `tests/fixtures/parity` at the repo root, referenced by path.
- Workspace `Cargo.toml` with `[workspace.package] version = "0.13.0-rust"`, `edition = "2024"`,
  `rust-version = "1.88"`. Add `build.rs` in `crates/pty` for the sha.
Done: `cargo test --workspace` = 173 green; `cargo build -p pty-core` needs no zig.

### WP2 — Registry and metadata (crate `pty-core`)
Goal: byte-compatible `$PTY_ROOT` with Node, safe for two writers.
- `registry/metadata.rs`: `SessionMetadata` with every Node field (`generation`, `daemon_pid`,
  `recovery: Option<serde_json::Value>` (opaque, preserved), `command`, `args`, `display_command`,
  `cwd`, `rows`, `cols`, `ephemeral`, `is_isolate_env`, `extra_env`, `unset_env`, `env`,
  `created_at` (ms precision `2026-08-29T10:00:00.123Z`), `tags`, `display_name`, `last_attach_at`,
  `exit_code`, `exited_at`, `last_lines`) plus `#[serde(flatten)] extra: BTreeMap<String, Value>` so
  unknown fields round-trip. Key order on write follows Node (`node-daemon-protocol-disk.md` §2.4):
  use a `serde_json::Map` builder, not struct order, for the daemon's publication write.
- `registry/atomic.rs`: `atomic_write(path, bytes)` → `<path>.tmp.<pid>.<16 hex>` + rename; unlink on
  failure. Readers skip any name containing `.tmp.`.
- `registry/lock.rs`: `acquire_file_lock(path) -> Option<LockGuard>` per `sessions.ts:2293-2336`
  (EEXIST → read pid → alive(kill 0 or EPERM) → false; dead/garbage → unlink → retry once). Event lock
  `<name>.events.lock` with the async wait ≤ 5 s and sync `event log is busy` error text
  (`events.ts:228-295`). Helper `with_both_locks(name, f)` enforcing order events → creation.
- `registry/mutate.rs`: `mutate_metadata_under_lock(name, f, {expected_generation,
  expected_metadata}) -> Busy|Missing|GenerationMismatch|Stale|Unchanged|Changed`
  (`sessions.ts:347-398`). All CLI mutations (`tag`, `rename`, `metadata patch`, `exec`, `kill`'s
  strategy strip) go through it and emit the matching event.
- `registry/list.rs`: `list_sessions()` per `sessions.ts:895-1013`: scan `.sock` first, then
  orphan `.json`; `read_pid` = sidecar pid, else `daemon_pid` only when `recovery.processStartToken`
  equals the live token (`/proc/<pid>/stat` field 22 on Linux, `ps -o lstart=` on macOS); dead pids
  probed by connect with a shared 500 ms budget; statuses exactly as Node; sorted by name; never
  mutates. `get_session(ref)` with the ambiguity error text (`sessions.ts:1351-1363`).
- `registry/names.rs`: `validate_name`, `validate_display_name`, `random_session_name` (alphabet
  `23456789abcdefghjkmnpqrstuvwxyz`, 8 chars, 8 attempts), `auto_display_name` (`cli.ts:651-668`).
- `registry/tags.rs`: reserved keys, `matches_all_tags`, `KEEP_TAG`/`is_keep_requested`
  (`sessions.ts:1020-1044`), `should_reap_at_exit` (existing, fix `keep` semantics), gc bookkeeping
  key list (still stripped by `restart`/`up` for Node-written sessions).
- `registry/cleanup.rs`: `cleanup_owned_socket/all(name, {generation, pid})` with generation CAS
  (`sessions.ts:2243-2266`); unlink order socket, pid, events, revision, json last.
- Remove `<name>.screen` and `FinalScreen`.
Reference: `node-daemon-protocol-disk.md` §2 (all), `sessions.ts` lines cited there.
Done: a Node daemon's `<name>.json` read then rewritten by Rust `tag` is byte-equal except the tag
diff and the appended event; the lock tests from `tests/security-fixes.test.ts:47-87` and
`tests/atomic-writes.test.ts` pass ported to Rust; `pty list --json` on a mixed directory (one Node
daemon, one Rust daemon, one vanished, one exited) matches Node's output field for field.

### WP3 — Events log (crate `pty-core`)
Goal: `<name>.events.jsonl` identical in envelope, types, retention, and follow semantics.
- `events/mod.rs`: `Event { session, type, ts, ..payload }`, the type constants and payload
  structs (`events.ts:10-22, 42-191`), `append_event` / `append_event_sync` under the event lock,
  retention (≥ 1000 lines → keep 500; daemon checks every 100 appends; one-shot writers when file
  ≥ 40000 bytes) as an atomic rewrite; `clear_events` at daemon start; `read_recent_events(n=50)`;
  `validate_user_event_type`; `format_event` (`events.ts:548-604`, local time `HH:MM:SS`).
- `events/follow.rs`: `EventFollower` on `notify` (existing files from EOF; new files from 0; size
  shrink → restart at 0; `--all` directory watch).
Done: ported `tests/events.test.ts` and `tests/events-emit.test.ts` literals green; Node
`pty events -f` shows a Rust daemon's `bell`/`title_change`/`session_exit` lines.

### WP4 — Terminal actor (crate `pty-terminal`)
Goal: one owner of the libghostty `Terminal`; typed reads; Node-equivalent serialization; query
answers; terminal events. Serves daemon, testkit, and (later) the TUI pane.
- `actor.rs`: `TerminalActor` runs on the thread that creates it; API is synchronous methods
  (`write(&[u8])`, `resize`, `snapshot()`, `serialize(SerializeOpts)`, `plain(viewport|full)`,
  `modes()`, `take_pty_replies()`, `take_events()`). The daemon and testkit already run their loop
  on that thread; the TUI later wraps it in a thread + channel (`pty-terminal::handle`, WP-TUI).
- `queries.rs`: install callbacks at construction: `on_device_attributes` → primary
  `ConformanceLevel` for `?62;22c`, secondary `DeviceType(0), firmware 382, rom 0` (→ `>0;382;0c`),
  tertiary default; `on_xtversion` → `"pty(0.8)"`; `set_default_fg_color(0xc0c0c0)`,
  `set_default_bg_color(0)`, `set_default_color_palette(all zeros)` so OSC 10/11/4 `?` are answered
  with Node's constants; `on_bell`, `on_title_changed`, OSC 9/99/777 via the `osc::Parser` on the
  raw stream (libghostty has no notification callback; tap OSC 9/99/777 before `vt_write`, same
  place `strip_terminal_queries` runs). Cursor-visibility transitions and focus-request (`?1004h`)
  come from diffing `mode()` before/after each `vt_write`. Kitty stack: `kitty_keyboard_flags()`
  is one value, not a stack — keep a daemon-side stack by scanning `CSI > n u` / `CSI < u` in the
  input stream (the existing Node approach).
- `serialize.rs`: `serialize_for_replay` = `Format::Vt` + modes + cursor + kitty, with the Node mode
  prefix (`server.ts:1065-1082`) prepended by the daemon from its own tracked flags, not from
  libghostty, so a Node client sees the same leading bytes. `plain_viewport()` and `plain_full()`
  implement Node's row selection (`server.ts:1269-1293`): viewport = rows `baseY..len`, full = all;
  trailing empty rows trimmed. Implement via `grid_ref` row walk with `with_trim(true)` per row,
  not `Format::Plain` of the whole buffer, so `--full` vs default is honoured.
- `snapshot.rs`: `CellGrid { rows: Vec<Vec<Cell>>, wrapped: Vec<bool>, cursor, base_y, len }` from
  `grid_ref`/`Row::is_wrapped`/`Cell` (palette index via `bg_color_palette`/style). This is the
  `readCells`/`readWrappedFlags` contract for the pane and the embedding handle.
- `strip.rs`: keep `strip_terminal_queries` (existing) and extend to Node's exact set.
Reference: `node-daemon-protocol-disk.md` §1.6, §1.7, §3.8, §3.9, §6.
Done: `tests/terminal-queries.test.ts:93-149` responses match byte for byte; the Node fixtures
`screens.json` pass with viewport semantics; a `ratatui-compat`-style replay (alt screen + kitty
stack + ECH/CUF backgrounds) restored on a real terminal shows the same picture as Node's replay.
Risk: `Format::Plain` may right-trim differently from xterm's `translateToString(true)`; decide by
the shared fixtures, and record the decision if libghostty keeps a trailing cursor-cell space.

### WP5 — Daemon (crate `pty`, `src/daemon/*`)
Goal: Node's per-session daemon semantics over the existing actor loop.
- `daemon/launch.rs`: `pty run` builds a `DaemonConfig` (all metadata fields) and spawns
  `<self> __daemon` with the config as JSON on an inherited pipe (fd 3), not argv, so nothing
  leaks to `ps` and the shape equals `PTY_SERVER_CONFIG` (`spawn.ts:169-184`). `setsid`, stdio
  null, stderr captured for the `Daemon process exited immediately (code N).\n<stderr>` message.
  Readiness = socket exists → metadata `daemonPid == child.pid` AND a `session_start` line with
  `ts >= createdAt` (`spawn.ts:225-236`), 30 s. Set `process title` (`prctl(PR_SET_NAME)` Linux,
  `setproctitle` absent on macOS → skip) `pty-daemon`; CLI sets `pty`.
- `daemon/env.rs`: `build_child_env` per `server.ts:131-209` (replacement / inherited / isolated,
  `unsetEnv` then `extraEnv`, force `PTY_SESSION` + `PTY_SESSION_GENERATION`, `TERM` default),
  `describe_invalid_cwd` texts (`server.ts:236-260`). Child spawn `/bin/sh -c 'exec "$@"' sh <cmd>
  <args>` with `command` resolved absolute (`spawn.ts:372-393` `resolve_command` in `pty-core`).
- `daemon/clients.rs`: `Client { role: Command|Writable|Readonly, rows, cols, attach_seq, phase:
  Live|Settling{deadline, generation}, queued: Vec<Packet> }`. Messages: add `ClientPeek{full}`,
  `Tick`. Handlers per `server.ts:931-1063`: ATTACH (<4 bytes ignored; sizeMatched computed first;
  role Writable; `attach_seq = ++counter`; `negotiate_size()`; send GEOMETRY to this socket if no
  resize happened; stamp `lastAttachAt` via `mutate_metadata_under_lock` best-effort; schedule cut
  with `REDRAW_SETTLE_MS = 80` when child alive and (resized or `now - last_resize < 80ms`), else
  immediate; after the cut, `nudge_redraw` when `!exited && !size_matched`), PEEK (role Readonly,
  negotiate, GEOMETRY, cut with plain/full flags, no alt-screen prefix), DATA (only `!exited &&
  role != Readonly`), RESIZE (only Writable with `attach_seq > 0`), STATUS (any), DETACH (end).
- Cut design (replaces Node's `terminal.write("", cb)`): a client in `Settling` receives no DATA;
  the actor's `recv_timeout` wakes at the earliest deadline; on wake (or immediately when the
  condition is "immediate") it serializes from the live terminal — which already contains every
  byte received — sends SCREEN, sets `Live`, then sends EXIT if `exited`. Because DATA and SCREEN
  are produced on the same thread from the same state, the baseline is exact by construction.
  Supersession: a new ATTACH/PEEK on a settling socket replaces the pending cut (generation++).
- `daemon/geometry.rs`: `negotiate_size()` = per-axis min over Writable clients with `attach_seq >
  0`; on change resize terminal, `broadcast_geometry` to Writable+Readonly sockets, then resize the
  PTY, set `last_resize_time` (`server.ts:1158-1190`). Zero writers → unchanged.
- `daemon/status.rs`: `StatsResult` with `clients.connections[]` (`server.ts:1084-1156`), command
  sockets excluded, metadata re-read per STATUS, resources via `/proc` (Linux) or `ps -o rss=,pcpu=`
  (macOS) — reuse `stats.rs`. Drop `geometryNeutral`.
- `daemon/lifecycle.rs`: generation `randomBytes(16).hex`; publication order = ensure dir → clear
  events → unlink stale sock → bind (umask 077, chmod 600) → write pid → write metadata (Node key
  order) → `session_start {tags?}`. Exit: `code = signal ? 128 + signal : status` (use
  `ExitStatus` raw wait status via `libc::waitpid`, not portable-pty's `exit_code()`), broadcast
  EXIT (post-cut queue for settling clients), `save_exit_metadata` under lock with
  `expected_generation` (retry 10 ms ≤ 400 ms on Busy/Stale), `lastLines` = all rows trimmed, last
  200; `session_exit {exitCode, signal?}`; shutdown 500 ms later. External kill (SIGTERM/SIGINT):
  snapshot descendants with start tokens (`process-tree.ts`), SIGHUP child, TERM descendants ≤ 1.5 s
  then KILL ≤ 0.5 s, wait child ≤ 2 s then SIGKILL; `PTY_SHUTDOWN_DEADLINE_MS` backstop (5 s) with
  the exact stderr line; reap decision re-reads on-disk tags and refuses if generation differs
  (`server.ts:1510-1524`); reaping = `cleanup_owned_all`, else `cleanup_owned_socket`.
  `PTY_SPAWNER_PID` poll every 5 s. Use the `signal-hook` crate for the handler → channel message
  instead of the raw `libc::signal` + atomics.
- `daemon/events.rs`: forward actor events (bell, title, notification, focus, cursor_visible) to the
  `EventWriter` (promise-chain equivalent: a dedicated writer thread with a queue, flushed at exit).
Reference: `node-daemon-protocol-disk.md` §1.3–1.9, §3.
Done: the `tests/integration.test.ts` order cases (§1.4: `[GEOMETRY, SCREEN]`, post-cut DATA before
EXIT, exit-during-sync, supersession, PEEK cancels ATTACH), `effective-geometry.test.ts` (§1.5),
`exit-signal` (137 + signal 9), `shutdown-backstop`, `spawner-pid-watchdog`, `kill-releases-socket`
(3-deep tree, leaf ignores HUP/TERM) all green as Rust tests driven through a socket client from
`pty-core`; Node `pty attach --attach-stream-fd-v1` against a Rust daemon yields `[GEOMETRY, SCREEN,
…, EXIT]`.

### WP6 — Client operations (crate `pty-core::client`)
Goal: Node's client behaviors, reusable by the CLI, testkit, and deskset.
- `client/attach.rs`: raw mode only if tty; ATTACH with `stdout` size or 24×80; SIGWINCH → RESIZE;
  SCREEN → `\x1b[2J\x1b[H` + payload; EXIT → `TERMINAL_SANITIZE + ESC[999;1H + "\r\n[<name> exited
  with code N]\r\n"`; detach text; error mapping `ENOENT|ECONNREFUSED|ECONNRESET|EPIPE` →
  `Session "<name>" not found or not running.`; `attach_stream_fd_v1` re-framing with the
  GEOMETRY-before/SCREEN-before checks, DETACH on local detach, truncation and EPIPE texts,
  backpressure (`client.ts:415-429, 467-471, 493-523, 596-641`); reconnect loop with the backoff
  table and `PTY_RECONNECT_MAX_ATTEMPTS` (`client.ts:431-442, 706-749`).
- `client/peek.rs`: one-shot (plain / ANSI + sanitize + `\n`), follow (`stripAnsi` when plain,
  Ctrl+\ single tap), `peek_wait` (200 ms poll, `lastLines` fallback, exact diagnostics).
- `client/send.rs`: existing pacing; `--paste` framing; typo-flag rejection stays in the CLI.
- `client/connection.rs`: `SessionConnection` (ATTACH on connect, resolves on first SCREEN,
  effective rows/cols from GEOMETRY, `resize/write/press/disconnect`) — the deskset and testkit API.
- `client/stats.rs`: `query_stats` 2 s.
Done: `tests/attach-stream.test.ts` literals (§1.11) green against the Rust daemon; `tests/sanitize`
byte string equal; deskset's `pty-wire` tests pass when its `protocol`/`session` modules are
replaced by `pty-core` re-exports (a branch in deskset, not merged by us).

### WP7 — CLI (crate `pty`, `src/cli/*`, one file per command)
Goal: every command, flag, text, and exit code in `node-cli-surface.md` §1–3, minus dropped/deferred.
- `cli/main.rs`: `--root` scan (first occurrence, anywhere), root-length backstop (three-line text),
  subcommand detection skipping `--filter-tag` values, interactive dispatch (empty/`i`/`interactive`,
  `--preselect-new`, `--force` filtered from dispatch args but visible to commands), per-command
  `-h/--help` only as `args[1]`, the `switch`, git-style forwarding via `which pty-<cmd>`, `Unknown
  command:` + usage on stdout, exit 1. Every thrown error → message on stderr, exit 1.
- `cli/help.rs`: `usage()` and `COMMAND_HELP` vendored verbatim from §3 (a test compares against
  `tests/fixtures/help/*.txt` extracted from the Node repo at `500eab2`).
- `cli/completions.rs`: `pty completions <shell>` prints the vendored `completions/pty.{fish,bash,
  zsh}` copied from Node byte for byte; exit 2 rules. (Regenerating from a spec is not needed while
  the command tree is frozen to Node's; revisit if the tree diverges.)
- `cli/run.rs` (§2.1 whole entry incl. legacy positional → dropped: no `Hint:`, tokens before `--`
  are an error `Usage: pty run ...`), `attach.rs` (§2.2, restart policies, dead-session prompt with
  `Command was:` quirk reproduced), `exec.rs` (§2.3), `peek.rs`, `send.rs`, `events.rs`, `list.rs`
  (§2.7 text layout with SGR, JSON key order, `--summary`, filters, sort by `displayName ?? name`),
  `stats.rs` (`printStats` block, gone shapes), `restart.rs` (guard regexes, prompt, tag strip,
  `scrubEnv`), `kill.rs`, `rm.rs`, `gc.rs` (debris, orphan kill, sweep, keep, prune, dry-run,
  footer parts, plist), `tag.rs`, `tag_multi.rs` (own help), `emit.rs`, `rename.rs`,
  `metadata.rs`, `up.rs`/`down.rs` (bind by the tag pair, tag sync lines), `version.rs`.
  `recover`/`evidence`/`test` print `pty <cmd>: not available in this build. See docs/parity.md.`
  exit 1 (documented absence).
- `cli/ask.rs`: `[Y/n]` prompt on stdin/stdout, `n` declines.
Reference: `node-cli-surface.md` §1, §2, §3, §5, §6; `rust-port-and-st2.md` §B (st2 needs
`already in use`, `not found`, `tags` in `list --json`, `metadata patch`, `--root`).
Done: a `tests/cli_contract.rs` suite that encodes every §6 literal (grouped per command) is green;
st2's `tests/pty.rs` + `tests/atomic_pty_snapshot.rs` + a live `st2 up` with `ST2_PTY_BIN` pointed
at the Rust binary run one agent through spawn → list → metadata patch → send → peek → kill → rm.

### WP8 — Remote (crate `pty`, `src/remote.rs`; client side in `pty-core::client::remote`)
- `remote-serve --stdio`: one JSON line, `list` → `{"sessions":[...]}`, `route` → `{"ok":true}\n` +
  bidirectional splice to `<name>.sock`, residual bytes forwarded, error shapes exact
  (`remote.ts:81-165`); exit 0 when the interaction ends.
- `--remote <peer>`: `fabric dial <peer> pty-remote` (`PTY_FABRIC_BIN`), 10 s, route line, ack,
  `RouteRefusedError`; `list --remote [<peer>]` host groups + JSON `{local, remote}`; bare
  `--remote` → `pty-relay ls --json` 5 s; `peek --remote` (no `--wait`), `send --remote`, `attach
  --remote` with the reconnect loop and status lines (`cli.ts:2019-2102, 2223-2247`).
Done: `tests/remote-fabric.test.ts` and `remote-exec-bridge` cases green with a stub `fabric` on
PATH that prints a socket path; the evals cell `pty-attach-machine-stream` passes with the Rust
binary on PATH.

### WP9 — Packaging
- `flake.nix` like st2's `rustPlatform.buildRustPackage` (`cargoLock.lockFile`), `PTY_BUILD_SHA`
  from `self.rev`, `installShellCompletion` from the vendored files, `mainProgram = "pty"`.
- libghostty-vt-sys under nix: its `build.rs` honours `GHOSTTY_SOURCE_DIR` (skips the git fetch) and
  `GHOSTTY_ZIG_SYSTEM_DIR` (zig package cache, skips zig's own fetches). Add a fixed-output
  `pkgs.fetchgit` of `ghostty-org/ghostty` at commit `a887df42c56f6de86c0fe6da9c4eeca37931e083`
  (the pin in `libghostty-vt-sys-0.2.1/build.rs:7`) and a second fixed-output derivation that runs
  `zig build --fetch` to populate the zig package dir; pass both via env; `nativeBuildInputs =
  [ pkgs.zig_0_15 ]`. Verify with `nix build` on the Linux build host; record the two hashes in the flake.
- `README.md`: rewrite for the workspace; state the deferred commands; edition 2024, Rust ≥ 1.88.
- st2 flake: switch `pty` input to this repo only at cutover (owned by the st2 side; out of scope
  here beyond noting it).
Done: `nix build .#pty` produces a binary whose `pty version` prints `0.13.0-rust+<sha>` and whose
`cargo test --workspace` gate ran in the sandbox (terminal tests need a PTY — they do run under nix
since `openpty` works in the sandbox; the socket tests need short `PTY_ROOT`, use `$TMPDIR`).

## 2. Order and parallelism

```
WP1 (restructure)  ── alone, ~1 day, blocks everything
   ├── WP2 registry ──┐
   ├── WP3 events ────┼── WP5 daemon ── WP8 remote
   ├── WP4 terminal ──┘       │
   └── WP6 client ────────────┴── WP7 CLI ── WP9 packaging (flake can start after WP1)
```
- After WP1, four agents in parallel with disjoint file ownership: A = WP2+WP3 (`crates/pty-core/
  src/registry/**`, `events/**`), B = WP4 (`crates/pty-terminal/**`), C = WP6 (`crates/pty-core/
  src/client/**`), D = WP9's flake + vendored help/completions fixtures + `cli/help.rs` +
  `cli/completions.rs` (no logic dependencies).
- Then WP5 (daemon) by one agent (touches `crates/pty/src/daemon/**` only) while WP7 starts on the
  file-op commands that need only WP2/WP3 (`list`, `tag`, `tag-multi`, `emit`, `rename`,
  `metadata`, `gc`, `rm`, `up`, `down`, `events`, `version`) in `crates/pty/src/cli/<cmd>.rs`.
- Then WP7's socket commands (`run`, `attach`, `peek`, `send`, `stats`, `restart`, `kill`, `exec`)
  once WP5 and WP6 land, then WP8.
- Merge rule: each package is one branch off `parity` (integration branch off `parity-map`),
  rebased and merged by the lead after `cargo test --workspace` and the package's done condition.

## 3. Risks and how the plan absorbs them

- **OSC 10/11/4 answers.** libghostty answers only with defaults set; WP4 sets them to Node's
  constants. Verify first thing in WP4 (30 min); if libghostty still stays silent, answer in the
  strip layer (we already intercept the bytes) exactly as Node does.
- **DA2/XTVERSION strings.** `on_device_attributes`/`on_xtversion` give full control; no risk left.
- **Plain trimming and viewport.** Row-walk serialization in WP4 gives exact control over trimming
  and the viewport/full split. The shared fixtures decide; one decision record if a trailing
  written space differs.
- **Reflow on resize.** Both engines reflow; cursor row/col after resize is pinned by Node tests
  (`integration.test.ts:1545-1600`). If libghostty places the cursor differently, that becomes a
  decision record with a fixture, not a blocker: the pinned position is the child's own redraw.
- **Actor under more message types.** All new state (client phases, deadlines, geometry, kitty
  stack) is plain data on the actor thread; only the signal handler and the descendant walk run
  elsewhere. `recv_timeout` keeps it single-threaded. No `Arc<Mutex<Terminal>>` anywhere.
- **Exact SCREEN baseline without a write callback.** Not needed: the actor parses synchronously,
  so "cut now" always sees every received byte; the settle deadline is the only asynchrony and it
  is actor-owned.
- **Locks shared with Node.** Same file, same protocol, same steal rule; the stale-steal test and a
  mixed-writer test (Node `pty tag` racing Rust `pty tag` 20×) in WP2's done condition.
- **Exit codes 2 → 1.** Every usage error moves to 1; `completions` keeps 2. Covered by WP7's
  contract suite.
- **`ps`/procfs on macOS.** Start tokens and resources need `ps` on darwin; keep both code paths
  and test the Linux one on the Linux build host, the darwin one on a macOS machine before cutover.
