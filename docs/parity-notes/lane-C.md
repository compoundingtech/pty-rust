# Lane C — WP6 client operations (crate pty-core, module client)

Read lane-common.md first. Worktree name: `laneC`. Branch: `laneC`.

You own: `crates/pty-core/src/client/**` (replace today's `client.rs` with a module directory: mod.rs, attach.rs,
stream.rs (fd-v1), peek.rs, send.rs, connection.rs, stats.rs, remote.rs, tty.rs, sanitize.rs), `crates/pty-core/src/protocol.rs`
(add GEOMETRY = 10 with encode/decode, drop the ATTACH flag byte extension and `ATTACH_FLAG_GEOMETRY_NEUTRAL`,
keep unknown-type preservation and the 32 MiB cap), `crates/pty-core/tests/client_*.rs`, `crates/pty-core/tests/protocol.rs`.
Registry: keep calling `pty_core::registry::{socket_path, read_metadata, ...}` by their current names — lane A is
restructuring that module concurrently and keeps the names stable. Do not edit registry files.
Deps you may add to pty-core: `signal-hook` (SIGWINCH), `tokio` behind an optional `tokio` feature (for
`AsyncConnection`, used later by deskset), nothing else heavy.

Deliverables (plan-core.md "WP6"; node-daemon-protocol-disk.md 1.10-1.13; node-cli-surface.md 2.2, 2.4, 2.5, 2.9, 1.7, 1.8):
1. `attach.rs` — `attach(AttachParams{name, socket: UnixStream, on_detach, on_exit, reconnect: Option<Box<dyn FnMut() ->
   Option<UnixStream>>>, stream_fd: Option<i32>}) -> AttachOutcome`: raw mode only if stdin is a tty; ATTACH with
   `stdout` size (TIOCGWINSZ) or 24×80; stdin → DATA; detach key 0x1c with kitty `ESC[92;5u` normalized; single tap
   detaches after 300 ms, double tap forwards 0x1c; SIGWINCH → RESIZE; DATA → stdout; SCREEN → `ESC[2J ESC[H` + payload;
   EXIT → `TERMINAL_SANITIZE + ESC[999;1H + "\r\n[<name> exited with code N]\r\n"`, return code; detach → DETACH packet,
   `TERMINAL_SANITIZE + ESC[999;1H + "\r\n[detached]\r\n"`; errors: ENOENT/ECONNREFUSED/ECONNRESET/EPIPE →
   `Session "<name>" not found or not running.` (or `Remote session ...`), else `Connection error: <msg>`; malformed packet
   → `pty client: dropping connection — <msg>`. Reconnect loop (client.ts:431-442, 706-749): backoff
   100,250,500,1000,2000,5000,10000 then 15000 cap, `PTY_RECONNECT_MAX_ATTEMPTS`, status lines
   `\r\n[reconnecting… — Ctrl-\ or Ctrl-C to stop]\r\n`, refusal → `[<name> session ended]` exit 0 (1 in fd mode),
   cap → `[<name>: connection lost — re-run `pty attach --remote` to reconnect]`.
2. `stream.rs` — `--attach-stream-fd-v1` (client.ts:415-429, 467-471, 493-523, 596-641; tests/attach-stream.test.ts):
   `validate_attach_stream_fd(fd)` with the two error texts; re-frame GEOMETRY/SCREEN/DATA/EXIT to the fd, stdout
   receives nothing; per-socket expectation GEOMETRY first then SCREEN with the two `daemon does not support attach
   stream v1 (...)` texts; empty DETACH written on local detach; EXIT ends the stream; disconnect before EXIT →
   `pty attach: machine stream truncated before EXIT: <err|connection closed>` exit 1; fd write error →
   `pty attach: machine stream descriptor <fd> failed: <msg>`; backpressure (pause socket reads until the fd drains);
   never close the fd. Reconnect status lines go to stderr in fd mode.
3. `peek.rs` — one-shot `peek(name, plain, full)`: SCREEN payload, then (unless plain) `TERMINAL_SANITIZE + ESC[999;1H`,
   then `\n`; `follow` (raw mode if tty, DATA → stdout with `strip_ansi` when plain, Ctrl+\ single tap → `[detached]`
   exit 0, EXIT → `\r\n[<name> exited with code N]\r\n` exit N); `peek_wait(name, patterns, timeout, plain)` polling
   `peek_screen(plain)` every 200 ms, any-of match, `lastLines` fallback when the connection fails and metadata has
   exitedAt, exact diagnostics (`Timed out after <sec>s waiting for "<p>".`, `Session "<name>" exited (code <c|?>) without
   matching "<p>".` + `Last output:`), patterns rendered as `"a" or "b"`. `strip_ansi` helper.
4. `send.rs` — keep pacing (300 ms default, `round(sec*1000)`), `--paste` = `ESC[200~` / `ESC[201~` as separate DATA
   packets around the whole payload, exit after `finish` (write all, shutdown write, wait for EOF ≤ 2 s).
5. `connection.rs` — `SessionConnection::connect(name, rows, cols)` (ATTACH on connect, resolves on first SCREEN),
   `effective_rows/cols` from GEOMETRY, `write`, `press(key)`, `resize`, `disconnect` (DETACH then close), `on_data`,
   `on_exit`; `send_data(name, items, {delay_ms, paste})`; `peek_screen(name, {plain, full})` with a 5 s deadline;
   optional `AsyncConnection` under the `tokio` feature with the same surface (deskset uses tokio).
6. `stats.rs` — `query_stats(name)` STATUS with a 2 s timeout (`Timeout querying stats for "<id>"`), invalid JSON →
   `Invalid stats response from "<id>"`; keep `StatsResult` types but add `connections: Option<Vec<ConnectionStats>>`
   and remove `geometry_neutral`.
7. `remote.rs` — `dial_and_route(peer, name) -> Result<UnixStream, RemoteError>` via `PTY_FABRIC_BIN` (default
   `fabric`) `dial <peer> pty-remote` (10 s; empty stdout → `fabric dial <peer> returned no socket`), write
   `{"op":"route","name":"<ref>"}\n`, wait ≤ 10 s for the ack line (`route handshake timed out`, `bad route response: <msg>`,
   `{"error":..}` → `RouteRefusedError`), bytes after the ack are pushed back into the stream; `fetch_remote_list(peer)`
   parsing `{"sessions":[...]}` at EOF.
8. `sanitize.rs` — `TERMINAL_SANITIZE` (exact bytes, already present) and `CURSOR_TO_BOTTOM`; `tty.rs` — RawMode,
   terminal_size, is_tty helpers.

Tests: protocol GEOMETRY round-trips + the legacy STATUS body without `connections` (tests/protocol.test.ts:262-345);
attach-stream literals from tests/attach-stream.test.ts (drive the functions against a fake daemon you write in the
test: a UnixListener that scripts packet sequences — legacy SCREEN-first daemon → nonzero and 0 bytes on the fd;
DATA before SCREEN → exit 1 with `[GEOMETRY]` on the fd; clean close without EXIT → truncated; EPIPE on fd; reconnect
keeps one stream with a fresh GEOMETRY/SCREEN); sanitize byte string equality with client.ts:37-55; send framing
(tests/send-paste.test.ts:121-219 bytes as seen by the fake daemon); peek_wait diagnostics. Real-daemon runs come
later from the conformance suite; the fake daemon makes this lane independent of the daemon rewrite.
