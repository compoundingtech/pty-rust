# Lane A — WP2 registry and metadata, then WP3 events log (crate pty-core)

Read lane-common.md first. Worktree name: `laneA`. Branch: `laneA`.

You own: `crates/pty-core/src/registry/**` (replace today's `registry.rs` with a module directory),
`crates/pty-core/src/events/**` (new), `crates/pty-core/tests/registry_*.rs`, `crates/pty-core/tests/events_*.rs`,
plus one-line module registrations in `crates/pty-core/src/lib.rs`. Add crate deps you need to
`crates/pty-core/Cargo.toml` (notify 8 for the follower, sha2/hex if needed, rand for tmp names — prefer
std + getrandom-free code where easy). Keep every public name that other code uses today stable:
`registry::{session_dir, ensure_session_dir, socket_path, pid_path, metadata_path, read_metadata,
write_metadata, read_pid, write_pid, pid_alive, list_sessions, SessionInfo, SessionMetadata, cleanup,
session_exists, validate_name, generate_id, resolve_ref}` — extend, rename only with a re-export shim,
because lane C (client) and the daemon call them concurrently in other worktrees.

## WP2 deliverables (see plan-core.md "WP2" and node-daemon-protocol-disk.md section 2 for every rule)
1. `registry/metadata.rs`: `SessionMetadata` with every Node field: generation, daemonPid, recovery (opaque
   `serde_json::Value`, preserved, never written by us), command, args, displayCommand, cwd, rows, cols,
   ephemeral, isolateEnv, extraEnv, unsetEnv, env, createdAt (ISO-8601 with milliseconds and `Z`), tags,
   displayName, lastAttachAt, exitCode, exitedAt, lastLines; plus `#[serde(flatten)] extra: Map` so unknown
   fields round-trip. A `write_metadata_publication()` that emits Node's key order for the daemon's first
   write (generation, daemonPid, recovery?, command, args, displayCommand, cwd, rows, cols, ephemeral,
   createdAt, tags?, displayName?, isolateEnv?, extraEnv?, unsetEnv?, env?), pretty JSON 2-space.
2. `registry/atomic.rs`: `atomic_write(path, bytes)` → `<path>.tmp.<pid>.<16 hex>` + rename, unlink on
   failure; readers everywhere skip names containing `.tmp.`. Remove the old `.json.tmp`/`.pid.tmp` scheme.
3. `registry/lock.rs`: `acquire_file_lock(path) -> Option<LockGuard>` exactly per sessions.ts:2293-2336
   (O_CREAT|O_EXCL 0600, write own pid; EEXIST → read holder pid → alive (kill 0 ok or EPERM) → None;
   dead/garbage → unlink and retry the exclusive create ONCE); release = unlink. Event lock
   `<name>.events.lock` with a waiting variant (≤ 5 s, 10 ms poll) and the sync error text
   `Session id "<name>" event log is busy. Retry the operation.`. `with_both_locks(name, f)` taking the event
   lock then the creation lock. `is_lock_owned_by_pid(name, pid)` for PTY_CREATION_LOCK_OWNER_PID.
4. `registry/mutate.rs`: `mutate_metadata_under_lock(name, f, MutateOptions{expected_generation,
   expected_metadata}) -> MutateStatus::{Busy, Missing, GenerationMismatch, Stale, Unchanged, Changed}`
   (sessions.ts:347-398); rewrites the whole parsed object so unknown fields survive; `patch_metadata_by_id`
   (displayName/tags patch, validation texts in node-cli-surface.md 2.19, returns {changed, metadata}) that
   emits `metadata_change` with only touched keys; `set_display_name`, `update_tags(name, set, remove)`
   emitting `display_name_change` / `tags_change` with full previous/value maps; no-op emits nothing.
5. `registry/list.rs`: `list_sessions()` per sessions.ts:895-1013 (scan `.sock` first, then orphan `.json`;
   pid = sidecar `.pid`, else metadata.daemonPid only when `recovery.processStartToken` equals the live
   process start token — Linux `/proc/<pid>/stat` field 22 as `linux:<n>`, macOS `ps -o lstart=` as
   `darwin:<s>`; dead pids probed by socket connect under one shared 500 ms budget, concurrently; statuses
   running/exited/vanished exactly as Node; sorted by name; NEVER creates, unlinks, or repairs).
   `get_session(ref)` with the ambiguity error text (sessions.ts:1351-1363); `all_session_names()`.
   `has_process_exited_for_reap`, `wait_for_process_exit(pid, ms)` (50 ms poll).
6. `registry/names.rs`: `validate_name` (Node's five messages), `validate_display_name` (four messages),
   `random_session_name` (8 chars from `23456789abcdefghjkmnpqrstuvwxyz`, `byte % 31`), `auto_display_name`
   (cli.ts:651-668 incl. sanitize), `short_path` (~), `time_ago`.
7. `registry/tags.rs`: reserved-key rule, `matches_all_tags`, `extract_filter_tags`, `KEEP_TAG`,
   `is_keep_requested` (any value except false/0/no/off after trim+lowercase), `should_reap_at_exit`
   (keep > ephemeral > permanent > PTY_REAP_ON_EXIT), `GC_BOOKKEEPING_KEYS` (strategy.status,
   strategy.consecutive-fast-fails, strategy.last-respawn-at, strategy.command-hash), `strip_gc_bookkeeping`.
8. `registry/cleanup.rs`: `cleanup_socket`, `cleanup_all`, `cleanup_owned_socket/all(name, {generation, pid})`
   with the generation CAS (sessions.ts:2243-2266), unlink order socket, pid, events, `.recovery/<name>.revision.json`,
   json last. Delete `<name>.screen` support entirely (FinalScreen, screen_path, write/read_final_screen) and
   update the one caller in the daemon and client (`peek` on a gone session will use `lastLines` later in lane C;
   for now make `client::peek` return the io error when the socket is gone — lane C rewrites that function).
9. `registry/root.rs`: `session_dir()` with the PTY_SESSION_DIR deprecation notices (exact texts,
   once per process, silenced by PTY_ROOT_LEGACY_SILENT), `root_length_check()` returning Node's three-line
   message when `byteLength(root)+14 > 104`.

Done for WP2: tests in `crates/pty-core/tests/` porting tests/security-fixes.test.ts:47-87 (lock steal rules),
tests/atomic-writes.test.ts (200 rewrites never yield unparseable JSON; concurrent writers; no `.tmp.` leftovers),
tests/metadata-events.test.ts:169-202 (unknown field `futureRecoveryCapability` survives), display-name
validation cases, list liveness cases from tests/list-filters.test.ts:119-210 (fabricated json/pid files),
and a round-trip test: take a metadata file written by the Node daemon (spawn one with the Node `pty` under a
temp PTY_ROOT), rewrite it through `update_tags`, and assert the only differences are the tag map and the event.

## WP3 deliverables (plan-core.md "WP3"; node-daemon-protocol-disk.md 2.5; node-cli-surface.md 5.6)
`events/mod.rs`: `Event { session, type, ts (ms epoch), ...payload }` with the type constants and typed payload
builders for bell, title_change, notification (osc9/osc99/osc777), focus_request, cursor_visible, session_start,
session_exit, session_exec, session_respawn, session_abandoned, session_flapping, user.*, display_name_change,
tags_change, metadata_change; `append_event`/`append_event_sync` under the event lock; retention (≥ 1000 lines
→ keep last 500; daemon writers check every 100 appends, one-shot writers when file ≥ 40000 bytes) as an atomic
rewrite; `clear_events`; `read_recent_events(name, n=50)`; `validate_user_event_type` (four messages);
`format_event` (`[HH:MM:SS] <session>: ...`, local time, bodies per node-cli-surface.md 5.6);
`EventWriter` (queue + one writer thread, `flush()`); `events/follow.rs`: `EventFollower` on the notify crate
(existing files from EOF, newly created files from offset 0, size shrink → restart at 0, `--all` directory watch,
poll fallback every 250 ms so tests are deterministic).
Done for WP3: ports of tests/events.test.ts:107-410 and tests/events-emit.test.ts:92-303 literals green.

Report the SHA when WP2 is done and again when WP3 is done (two commits or more).
