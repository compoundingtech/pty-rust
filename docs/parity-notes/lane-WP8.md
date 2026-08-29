# Lane WP8 — remote over fabric

Read lane-common.md, plan-core.md "WP8", node-daemon-protocol-disk.md section 4, node-cli-surface.md 2.8 and the
`--remote` parts of 2.2, 2.4, 2.5, 2.7. Worktree: `wp8`. Branch: `wp8` (off `parity` after WP7b is merged).

You own: `crates/pty/src/remote.rs` (server side: `remote-serve --stdio`), `crates/pty/src/cli/remote_serve.rs`,
the `--remote` branches inside `cli/{list,peek,send,attach}.rs` (small, clearly delimited edits), and
`crates/pty/tests/remote_*.rs`. Client-side dial lives in `pty_core::client::remote` (lane C).

1. `remote-serve --stdio` only (`--socket` is dropped: print the Node usage block from cli.ts:1331-1339 to stderr,
   exit 1, when `--stdio` is absent — that is what Node does for a missing `--socket` value too): read one
   `\n`-terminated JSON line from stdin; `{"op":"list"}` → one line `{"sessions":[{name,status,command?,cwd?,tags?,
   displayName?}]}` (fields only when set; `command` = displayCommand) and exit 0; `{"op":"route","name":"<ref>"}`
   → `get_session` (ambiguity text → `{"error":"<msg>"}`; missing → `{"error":"session \"<ref>\" not found"}`),
   connect to `<name>.sock` (connect error → the same not-found error), write `{"ok":true}\n`, forward any bytes that
   followed the request line, splice stdin↔socket and socket↔stdout until either side closes, exit 0; malformed →
   `{"error":"malformed request"}`; unknown → `{"error":"unknown op: <op>"}`. Reads ambient PTY_ROOT.
2. `--remote <peer>` on `list` (host group `{label, sessions, error}` via `fetch_remote_list`; `fabric dial` failure
   → error string, exit 0; JSON `{local, remote:[...]}`; text group rendering per 2.7), bare `list --remote` →
   `pty-relay ls --json` (5 s; ignore failures), `peek --remote` (no `--wait` → `pty peek --wait is not supported with
   --remote yet.`), `send --remote`, `attach --remote` (nesting guard applies; `attach` with the reconnect callback
   = re-dial; `RouteRefusedError` → clean stop with `[<name> session ended]`).
3. Tests: a stub `fabric` script on PATH that prints the path of a Unix socket served by `pty remote-serve --stdio`
   (spawn it with a socketpair or via `socat`-free Rust helper: a listener that, per connection, spawns
   `pty remote-serve --stdio` with the connection as stdin/stdout), then port tests/remote-fabric.test.ts:86-270
   (list JSON shape with `error:null`; dial failure → `error` string and `sessions:[]`; peek routes; missing →
   `/not found/`; ambiguous → exact ambiguity text; send --seq; attach replays and forwards input) and
   remote-exec-bridge.test.ts:97-122 (handler exits when the interaction ends), remote-reconnect.test.ts:142-240
   (reconnect only on a loud close; resumes after an outage with `reconnecting` shown; kill on the remote → route
   refused → `session ended`).
