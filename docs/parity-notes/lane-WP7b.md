# Lane WP7b — the socket verbs: run, attach, exec, peek, send, stats, restart, kill

Read lane-common.md, plan-core.md "WP7", node-cli-surface.md 2.1-2.5, 2.9-2.11, 5.1, 5.3-5.5, and section 6 for
those verbs. Worktree: `wp7b`. Branch: `wp7b` (off `parity` after WP5, WP7a and lane C are merged).

You own: `crates/pty/src/cli/{run,attach,exec,peek,send,stats,restart,kill}.rs` and `crates/pty/tests/cli_<verb>.rs`
for them. Use `daemon::launch::spawn_daemon` (WP5), `pty_core::client::*` (lane C), `pty_core::registry::*` (lane A),
`cli::{resolve_ref, ask}` (WP7a).

1. `run.rs` (2.1 whole entry, 6.2): flags `-d/--detach`, `-a/--attach`, `-e/--ephemeral`, `--isolate-env`,
   `--no-display-name`, `--force`, `--id`, `--name`, `--cwd`, `--tag` (repeatable, `Invalid tag format: "<tok>". Use
   --tag key=value`), `--env` (`Invalid env format: "<tok>". Use --env KEY=VALUE`), `--unset-env` (`Invalid env key:
   "<tok>". Use --unset-env KEY`), `--rows`/`--cols` (Rust extension, kept); the flag loop stops at the first
   unrecognized token or `--`; legacy positional before `--` is DROPPED → treat any token before `--` as the usage
   error `Usage: pty run [--id <id>] [--name <displayName>] [-d] [-a] -- <command> [args...]`; no command → same
   usage; `displayCmd` = as typed; `resolve_command` (`Command not found: <cmd>`); nesting rules (PTY_SESSION set,
   no -d/--force): `-a` on a running target → the three-line refusal; else stderr `Already inside pty session
   "<s>", running directly.` and run in place (spawn + wait, exit with its status, env minus unset plus env
   overlays); outside: `validate_name`, `Session id "<id>" is already in use.` (unless -a), random id with 8
   attempts, display name precedence (`--no-display-name` | `--name` validated → `Invalid displayName: <msg>` |
   auto), `PTY_CREATION_LOCK_OWNER_PID` handoff, event lock then creation lock with their texts, running + `-a` →
   `Session "<id>" already running, attaching.` then attach, running → `Session "<id>" is already running. Use "pty
   attach <id>" to connect.` exit 1, gone → cleanup and reuse cwd/tags/extraEnv/unsetEnv/displayName unless
   overridden (tags replaced wholesale when any --tag given), spawn, stdout `Session "<id>" created.`, `-d` → exit 0
   else attach (exit 0 on detach, session code on exit). rows/cols default from the CLI's stdout size or 24×80.
2. `attach.rs` (2.2, 6.3): `-r/--auto-restart`, `--no-restart`, `--force`, `--remote <peer>` (WP8 supplies the
   dial; until then `--remote` → `pty attach --remote <peer>: fabric not available` exit 1), `--attach-stream-fd-v1
   <fd>`; validation order and texts exactly; nesting guard BEFORE ref resolution (four-line text); restart policy
   never/always/prompt; `handleDeadSession` (lastLines indented, `Session "<name>" exited with code <c|unknown>.`,
   `Command was: <displayCommand> <args...>` with the duplicated-args quirk, `Restart? [Y/n] `, restart via
   spawn_daemon with persisted rows/cols/ephemeral/isolateEnv/extraEnv/unsetEnv/env and scrub_env ST_AGENT/ST_ROOT,
   `Session "<name>" restarted.`, then attach); running → `client::attach`.
3. `exec.rs` (2.3, 6.4): `--` required; PTY_SESSION / PTY_SESSION_GENERATION / metadata / resolve / event lock /
   ptyfile / generation-mismatch / other-status texts; rewrite command/args/displayCommand under lock with
   expected_generation; `session_exec` event; spawn + wait (not exec), exit with its status.
4. `peek.rs` (2.4, 6.5): leading flag loop (`-f/--follow`, `--plain`, `--full`, `--wait` repeatable, `-t/--timeout`
   float), gone-session paths (lastLines + `\n` exit 0; `Session "<name>" has <vanished|exited> with no saved output.`
   exit 0), `--remote` hook, `client::peek` / `follow` / `peek_wait`.
5. `send.rs` (2.5, 6.6): `--remote` pulled from anywhere, ref, `--paste` removed from anywhere, `--with-delay` only
   as the first token after the ref, positional vs `--seq` exclusivity, typo flags, `--seq` value resolution through
   `parse_seq_value` with the KEY_SPEC_HELP errors, `Nothing to send.`, `Unexpected argument: <tok>`, then `client::send`.
6. `stats.rs` (2.9, 6.9): `--json`, `--all`, optional ref; gone shapes; `query_stats`; `printStats` text block exactly;
   no-ref aggregate incl. `No running sessions.`, parallel queries, `--all` gone sections.
7. `restart.rs` (2.10, 6.16): `-y`, `--force`, unexpected argument, stateful-agent guard (regexes + text mentioning
   `convoy`), running prompt `Session "<name>" is running. Kill and restart? [Y/n] `, SIGTERM + cleanup_socket +
   200 ms, cleanup_all, tags minus gc bookkeeping, spawn with persisted options + scrub, `Session "<name>" restarted.`,
   nested note on STDOUT `  (not attached: already inside pty session "<s>". Pass --force to attach anyway.)`, attach.
8. `kill.rs` (2.11, 6.13): not running → `Session "<name>" is not running. Use "pty rm <name>" to remove it.` exit 1;
   strip `strategy` when permanent; SIGTERM; wait 7 s (`Failed to kill session "<name>": daemon PID <pid> is still
   running after 7s. Socket <root>/<name>.sock may still be owned.` exit 1); cleanup_socket; `Session "<name>"
   killed.`; ptyfile note lines.

Tests: section-6 literals for spawn-options, nesting, nesting-prevention, display-name (run/attach parts),
attach-no-restart, exec, peek-wait, send-paste, seq-delay, stats-cli, restart-* (all four), kill-wait,
rm-kill-ephemeral (kill half), exit-reap (CLI-observable parts), integration.test.ts CLI-level cases. Use
`pty_testkit::Session::spawn` of the built binary for tty-bound flows (attach, prompts, `peek -f`).
