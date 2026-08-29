# Lane WP5 — the daemon (crate pty, src/daemon/**)

Read lane-common.md, then plan-core.md "WP5" and node-daemon-protocol-disk.md sections 1.3-1.9, 2.9, 2.10, 3.
Worktree: `wp5`. Branch: `wp5` (off `parity` after lanes A, B, C are merged — check `git log` for them).

You own: `crates/pty/src/daemon/**` (split today's daemon/mod.rs into mod.rs, launch.rs, config.rs, env.rs,
clients.rs, geometry.rs, status.rs, lifecycle.rs, events.rs, tree.rs), the `__daemon` arm in main.rs, and
`crates/pty/tests/daemon_*.rs`. The CLI's `run` (lane WP7b) calls `daemon::launch::spawn_daemon(SpawnParams)`;
define that API here and keep it stable: `SpawnParams { name, command (resolved absolute), args, display_command,
cwd, rows, cols, ephemeral, tags, display_name, isolate_env, extra_env, unset_env, env (replacement), scrub_env:
Vec<String>, creation_lock_owner_pid, bind_to_spawner_lifetime }` → `Result<SpawnedDaemon{pid, generation}, SpawnError>`
with Node's error texts (`Daemon process exited immediately (code N).\n<stderr>`, `Timeout waiting for session
"<id>" to start`, `Timed out waiting for daemon publication for session "<id>".`).
Use lane A's registry (locks, metadata, events), lane B's `TerminalActor` (serialize, queries, snapshot, events),
lane C's protocol (GEOMETRY).

Deliverables:
1. `launch.rs`: config as JSON on inherited fd 3 to `<current_exe> __daemon` (no argv leak; `PTY_SERVER_CONFIG`
   shape from spawn.ts:169-184 plus `generation` omitted → daemon makes one), `setsid`, stdin/stdout null, stderr
   piped and collected; readiness = socket exists (stat every 50 ms + 100 ms settle) → metadata `daemonPid ==
   child.pid` AND a `session_start` event line with `ts >= createdAt` (spawn.ts:225-236); 30 s; on failure SIGTERM
   the child. `PTY_SPAWNER_PID` set when `bind_to_spawner_lifetime`. Process title `pty-daemon` via
   `prctl(PR_SET_NAME)` on Linux (the CLI sets `pty` in main.rs — do that too; 15-char limit).
2. `env.rs`: `build_child_env` per server.ts:131-209 (replacement | inherited | isolated allow-list
   PATH HOME USER LOGNAME SHELL TERM COLORTERM LANG TZ PWD TMPDIR PTY_ROOT PTY_SESSION_DIR + LC_*; delete
   PTY_SERVER_CONFIG; unsetEnv then extraEnv; force PTY_SESSION=<name> and PTY_SESSION_GENERATION=<generation>;
   TERM default xterm-256color when absent/empty; PWD = cwd) with the exclusivity error text; `describe_invalid_cwd`
   (five texts + `\nCannot start session "<id>" for command "<cmd>".`, server.ts:236-260); child spawned as
   `/bin/sh -c 'exec "$@"' sh <command> <args...>` with `command` already resolved (spawn.ts:372-393 lives in
   pty-core, add `resolve_command` there if lane C did not).
3. `clients.rs`: `Client { role: Command|Writable|Readonly, rows, cols, attach_seq, phase: Live|Settling{deadline,
   generation, kind: Attach|Peek{plain, full}}, queued: Vec<Vec<u8>>, tx }`. ATTACH (<4 bytes ignored; size_matched
   computed BEFORE negotiation; role Writable; attach_seq = ++counter; negotiate; GEOMETRY to this socket if no resize
   happened; lastAttachAt via mutate_metadata_under_lock best-effort with expected_generation; cut scheduled with
   REDRAW_SETTLE_MS=80 when child alive and (resized or now - last_resize < 80 ms) else immediate; after the cut
   nudge_redraw (cols-1 then back) when !exited && !size_matched), PEEK (role Readonly, negotiate, GEOMETRY if no
   resize, cut with plain/full flags, mode prefix WITHOUT the alt-screen entry), DATA (only !exited && role !=
   Readonly), RESIZE (only Writable with attach_seq > 0, payload ≥ 4; attach_seq = ++counter; negotiate), STATUS
   (any role, reply from status.rs), DETACH (end socket), close/error → remove + negotiate. Broadcast rules:
   Settling clients get no DATA/EXIT (their bytes land in the SCREEN); Cutting is not needed as a phase because the
   cut is synchronous on the actor thread: at the deadline serialize (SCREEN = prefix + Vt) and send, set Live,
   then send EXIT if exited. A newer ATTACH/PEEK on the same socket bumps generation and replaces the pending cut.
   The actor loop uses `recv_timeout` to the earliest client deadline.
4. `geometry.rs`: `negotiate_size()` per server.ts:1158-1190: per-axis min over Writable clients with
   attach_seq > 0; if both > 0 and either differs → resize terminal, broadcast GEOMETRY to Writable+Readonly
   sockets (before the PTY resize), resize PTY, last_resize_time = now, return true. Zero writers → unchanged.
5. `status.rs`: `StatsResult` per server.ts:1084-1156 with `clients.connections[]` (`{role:"writable", rows, cols,
   lastRequestSequence, constrains:{rows, cols}}` | `{role:"readonly", constrains:{rows:false, cols:false}}`),
   command sockets excluded from counts, metadata re-read per STATUS (uptimeSeconds = floor((now - createdAt)/1000)),
   resources via /proc (Linux) or `ps -o rss=,pcpu=` (macOS), `modes.kittyKeyboardFlags` = the stack as a list.
6. `lifecycle.rs`: generation = 16 random bytes hex; publication order: ensure dir 0700 → clear events → unlink stale
   sock → bind under umask 077 → chmod 0600 → write pid (plain) → publication metadata (Node key order, via lane A's
   writer) → `session_start {tags?}` → ready. Child exit: raw `waitpid` status → `code = signal ? 128+signal : status`;
   broadcast EXIT (Settling clients get it after their SCREEN); `save_exit_metadata` under lock with expected_generation
   (retry every 10 ms ≤ 400 ms on Busy/Stale; again after drain; again 2 s after the child is confirmed dead) with
   exitCode, exitedAt, lastLines = all rows trimmed, last 200; `session_exit {exitCode, signal?}`; clean shutdown
   500 ms later with exit status = child code. External kill (SIGTERM/SIGINT via signal-hook → channel): snapshot
   descendants with start tokens (tree.rs = port of process-tree.ts: `ps -axo pid=,ppid=` BFS, deepest first,
   token re-check before each signal, TERM ≤ 1500 ms then KILL ≤ 500 ms), SIGHUP the child, wait child ≤ 2 s else
   SIGKILL + ≤ 500 ms, save exit metadata until settled ≤ 2 s, flush events; `PTY_SHUTDOWN_DEADLINE_MS` (default
   5000) backstop with stderr `pty daemon "<name>": graceful shutdown exceeded <ms>ms — forcing exit (child reaped)`;
   reap decision re-reads on-disk tags, refuses if the on-disk generation differs, never reaps on an external kill
   unless ephemeral (server.ts:1481-1524); reaping = `cleanup_owned_all`, else `cleanup_owned_socket` (sock + pid
   only; json and events stay). `PTY_SPAWNER_PID` watchdog: integer > 1; dead at boot → shutdown; poll every 5 s.
7. `events.rs`: actor events (bell, title_change deduplicated, notification, focus_request, cursor_visible) →
   the `EventWriter` (lane A) with the daemon retention rule (check every 100 appends).
8. Remove: the `__daemon` argv config, the geometry-neutral flag handling, `<name>.screen`, the SIGHUP-then-SIGKILL
   500 ms watchdog (replaced by 6).

Tests (crates/pty/tests/daemon_*.rs, each driving the built binary via `env!("CARGO_BIN_EXE_pty")` + a socket client
from pty-core): the ordering cases of tests/integration.test.ts:423-852 (`[GEOMETRY, SCREEN]`; DATA during sync folded
into SCREEN; post-cut DATA before post-cut EXIT with exactly one EXIT; EXIT before cut / DATA after; exit during sync
`[GEOMETRY, GEOMETRY, SCREEN, EXIT]`; second ATTACH supersedes; PEEK cancels a pending ATTACH), roles 854-1214
(peek→attach and attach→peek transitions with STATUS counts; malformed 2-byte ATTACH changes nothing; peeker DATA and
RESIZE ignored; attach at current size sends no SIGWINCH within 150 ms), effective-geometry.test.ts (GEOMETRY index <
every SCREEN/DATA index; per-axis minima via `stty size`), exit-signal (137 + signal 9; `exit 5`), shutdown-backstop
(HUP-trapping child, deadline 300 ms → both dead ≤ 4 s), spawner-pid-watchdog, the 3-deep tree from
bin/pty-kill-releases-socket-test (leaf ignores HUP/TERM; socket released before `kill` returns), exit-event-race
(exactly one session_exit and one session_start), publication readiness (daemonPid + session_start).
Also run the Node client against your daemon: `node_pty attach --attach-stream-fd-v1 3 <id> 3>out.bin` must yield
`[GEOMETRY, SCREEN, ..., EXIT]` (the Node pty is on PATH as `pty`; point PTY_ROOT at your temp root).
