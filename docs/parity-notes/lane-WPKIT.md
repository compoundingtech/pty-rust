# Lane WP-KIT — Rust testkit server mode (crate pty-testkit)

Read lane-common.md, plan-verify-libs.md "B2", node-testing-tui.md section 1 (the Node API and semantics).
Worktree: `wpkit`. Branch: `wpkit` (off `parity` after WP5 and lane C are merged).

You own: `crates/pty-testkit/**` (session.rs may be split into spawn.rs + server.rs; keep the public API).

1. `Session::server(command, args, ServerOptions{name, rows, cols, cwd, env}) -> io::Result<Session>`: spawns a
   daemon through the same path `pty run -d` uses (call `pty_core`'s spawn helper if lane WP5 exposed one in a
   library crate; otherwise run the built `pty` binary from `CARGO_BIN_EXE_pty`-equivalent found via `PTY_BIN` env or
   PATH — document which), under the ambient PTY_ROOT (tests set a temp one), connects with
   `pty_core::client::SessionConnection` semantics but feeds SCREEN/DATA into its own `TerminalActor` (lane B) so
   `screenshot()` is byte-identical to spawn mode; `attach()` resolves on the first parsed SCREEN (no fixed delay);
   `reconnect()` (close socket, 100 ms, reset actor, reconnect, attach); `Session::connect_to_existing(&Session,
   rows, cols)`; `resize(rows, cols)` sends RESIZE and `rows()/cols()` follow GEOMETRY; `has_exited()`, `exit_code()`,
   `name()`, `close()` kills the daemon only when owned (`pty kill` + `rm` semantics via the registry, not SIGKILL).
2. Defaults: `wait_for_text_default`, `wait_for_absent_default`, `wait_for_default` (10 000 ms, 50 ms poll; the first
   check after the first sleep, like Node) alongside the explicit-timeout variants; error message format unchanged.
3. `keys::resolve_key` (pty-core) accepts `+`, `-`, `_` separators and the `C-` prefix with Node's error texts
   (keys.ts:20-64; tests/keys.test.ts) — coordinate: lane C may have done this; if so, only add tests.
4. Executable docs: `crates/pty-testkit/README.md` with the API and examples; a doctest per public method.

Tests: `crates/pty-testkit/tests/server_session.rs` porting the server-mode cases of tests/screenshot.test.ts
(attach shows prior output; reconnect replays screen incl. cursor and scrollback; resize + `tput cols`; second client
via connect_to_existing sees the same output and min-wins geometry; immediate attach; exit code; alt-screen replay;
high throughput 600 lines) against the Rust daemon. All existing testkit tests stay green.
