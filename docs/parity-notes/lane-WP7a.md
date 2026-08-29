# Lane WP7a — CLI dispatcher and the file-operation verbs (crate pty, src/cli/**)

Read lane-common.md, plan-core.md "WP7", node-cli-surface.md sections 1, 2.0 (dispatch only), 2.6, 2.7, 2.10-2.22,
2.25, 5, and the matching parts of section 6. Worktree: `wp7a`. Branch: `wp7a` (off `parity` after lane A is merged).

You own: `crates/pty/src/main.rs`, `crates/pty/src/cli/mod.rs` (dispatcher), `crates/pty/src/cli/argv.rs` (a small
cursor helper), `crates/pty/src/cli/{ask,list,tag,tag_multi,emit,rename,metadata,gc,rm,up,down,events,version,
deferred}.rs`, `crates/pty/tests/cli_*.rs` for those verbs. Lanes D (help.rs, completions.rs) and WP7b (run, attach,
exec, peek, send, stats, restart, kill) add their own files; leave `todo!()`-free stubs that print
`pty <cmd>: not implemented yet` for the WP7b verbs so the binary always builds.

1. `main.rs` + `cli/mod.rs` (cli.ts:670-760): `process title pty`; `--root <path>` scanned across the whole argv,
   first occurrence, missing/dash value → `pty: --root requires a path (e.g. pty --root /var/lib/pty-eval list)` exit 1,
   sets PTY_ROOT and splices both tokens; root-length backstop (three-line text from node-cli-surface.md 1.2 step 3)
   before dispatch; subcommand = first non-dash token skipping the token after `--filter-tag`; empty / `i` /
   `interactive` → interactive (for now: print `pty interactive: not implemented yet` exit 1 unless WP-TUI landed —
   call `interactive::run(opts)` behind a function that WP-TUI replaces); `dispatchArgs` filters `--preselect-new`
   and `--force`; per-command help: `args[1]` is `-h`/`--help` AND `help::command_help(cmd)` exists → print to stdout,
   exit 0 (aliases a/ls/remove); the switch with every command word and alias; `version`/`--version`/`-v`/`-V`;
   `help`/`--help`/`-h`; default → `which pty-<cmd>` (external `which`), found → spawn with inherited stdio and the
   unfiltered args, exit with its status (?? 1); else stderr `Unknown command: <cmd>` then the FULL usage on STDOUT,
   exit 1. Every error path: message on stderr, exit 1 (never 2 except completions). A `CliError` type whose Display
   is the exact message; `main` prints it and exits 1.
2. `deferred.rs`: `recover`, `evidence`, `test` → stderr `pty <cmd>: not available in this build. See docs/parity.md.`
   exit 1 (their `--help` still prints the vendored Node help).
3. `list.rs` (2.7 + 6.8): parsing (extract_filter_tags first; `--status` enum with its error; `--older-than`/
   `--newer-than` durations with the error text; `--remote [<peer>]`; `--json`, `--tags`, `--summary`; other tokens
   ignored); filters; sort by `displayName ?? name`; JSON array with the exact key order (name, status, pid, command,
   cwd, createdAt, exitCode, exitedAt, tags?, displayName?), `--json --summary` shape, `{local, remote}` when remote
   (remote itself is WP8: for now `--remote` produces an empty host group list — leave a hook `remote::list_hosts`);
   text mode exactly per 2.7 (sections, SGR codes, `[permanent]`/`[flapping]`, `#k=v`, `~` paths, timeAgo,
   `No active sessions.`, summary lines).
4. `tag.rs` (2.15, 6.12), `tag_multi.rs` (2.16 incl. its own help via help fixture `tag-multi.txt`), `emit.rs` (2.17),
   `rename.rs` (2.18), `metadata.rs` (2.19: `patch --id`, stdin JSON, `{changed, metadata}` line, all error texts),
   `rm.rs` (2.13: refuse running, 7 s wait, generation CAS, texts), `events.rs` (2.6: --all/--recent/--json/--wait/-t,
   follow with SIGINT → exit 0, format via lane A), `up.rs`/`down.rs` (2.21/2.22: bind by the (ptyfile, ptyfile.session)
   tag pair, tag sync with `updated tags: ...`, `-removed`, gc bookkeeping strip, `● <label> (started|already running|
   unchanged (already running))`, `✗ <label>: <msg>`, footers, `Unknown session[s]: ...` / `Available: ...`, manifest
   errors; spawning goes through `daemon::launch::spawn_daemon` — until WP5 lands, call the existing `spawn_session_daemon`
   through a thin adapter and note it), `gc.rs` (2.14 minus respawn/flapping/abandoned: raw debris, orphan kill
   (`Killed orphan child: <name> (parent <p> missing|dead)`, skips), sweep (`Removed:`, `Kept (keep tag): <n> — remove
   the keep tag to reap it`), `pruneOrphanLayoutTags` (`Pruned orphan tags on <n>: #k`), `-n/--dry-run` variants and
   the `(Dry run — no changes made.)` footer, `Nothing to clean up.` / `Cleaned up <parts>.` with only the parts that
   still exist (orphan children, reap skips, stale sessions, orphan tags), `--print-launchd-plist [--interval N|=N]`
   exact XML with label rules and `<root>/gc.log`; `--idle-days`/`--fast-fail-*` are accepted and ignored? NO: they
   belonged to the dropped features — reject nothing, accept and ignore silently, note it in docs/parity.md §12 row),
   `version.rs`, `ask.rs` (`[Y/n]` readline, `n` declines).
5. Behavior hooks WP7b needs: `cli::resolve_ref(ref) -> String` (not found → `Session "<ref>" not found.` exit 1;
   ambiguity text from registry), `cli::require_ref(args, usage_text)`.

Tests: `crates/pty/tests/cli_<verb>.rs` with the section-6 literals for list, tags, tag-mutate, tag-bulk, tag-multi,
metadata-events (CLI half), events-emit, events (CLI half), rename (display-name.test.ts:215-311), rm (rm-kill-ephemeral
rm half), up-down, up-name-decouple, gc (gc.test.ts minus respawn/flapping/abandoned; gc-parent-child), pty-root
(--root, notices, length), version, unknown-command forwarding (put a `pty-hello` script on PATH), help interception.
Where a test needs a running daemon use the existing `run -d` (it works for plain commands today).
