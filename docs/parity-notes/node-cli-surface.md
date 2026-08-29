# Node `pty` CLI surface inventory (for the Rust drop-in port)

Source tree: `<node-pty-checkout>` (read-only), package `@compoundingtech/pty` version `0.12.0` (package.json:3).
All `file:line` citations below are relative to that tree. `cli.ts` = `src/cli.ts`.

Conventions used in this document:
- "stdout"/"stderr" mean `console.log` / `console.error` unless a raw `process.stdout.write` is called out.
- "exit N" means `process.exit(N)`.
- `<ref>` = a user-supplied session reference (stable id OR displayName); `<name>`/`<id>` = the stable on-disk id.

---

## 1. GLOBAL

### 1.1 Process entry

- `bin/pty` (bin/pty:1-24): `#!/usr/bin/env node`; sets `process.title = 'pty'` (bin/pty:7); if `dist/cli.js` is missing prints `dist/cli.js not found. Run: npm run build` to stderr and exits 1 (bin/pty:16-19); otherwise `await import(dist/cli.js)` **in the same process** (no child, so inherited fds >= 3 are preserved for `--attach-stream-fd-v1`) (bin/pty:21-24).
- `src/cli.ts:80` sets `process.title = "pty"` again. The daemon sets `process.title = "pty-daemon"` (server.ts:1466).
- `main()` (cli.ts:670) is invoked at module load; `main().catch(err => { console.error(err.message); process.exit(1) })` (cli.ts:4132-4135). **Any thrown `Error` anywhere prints only `err.message` to stderr and exits 1.** This is the path for: ambiguous `<ref>` (sessions.ts:1358-1362), `pty evidence` argument errors (cli.ts:2897-2921), `pty exec` generation errors (cli.ts:1873-1875, 1895, 1927), `spawnDaemon` failures not caught locally (e.g. `pty run` daemon failing to start; cli.ts:1749-1756 is inside a `try/finally` with no catch), `readPtyFile` errors are caught locally, etc.
- A second bin, `pty-kill-releases-socket-test` (package.json:35; bin/pty-kill-releases-socket-test): a self-contained smoke test. Reads `PTY_TEST_BIN` (default `pty`), sets `PTY_ROOT=<tmp>/registry` and `PTY_ROOT_LEGACY_SILENT=1`, runs `pty run -d --id <session> --no-display-name -- node <self> --launcher ...`, `pty kill <session>`, `pty run -a -d --id ...`; prints `PASS <platform>: pty kill released the owned socket and replacement start completed` or `FAIL <platform>: <msg>[; surviving socket owner pid N]`; exit 0/1 (bin/pty-kill-releases-socket-test:102-170). Not required for a CLI port but documented for completeness.

### 1.2 argv dispatch (cli.ts:670-760)

Order of operations in `main()`:

1. `args = process.argv.slice(2)` (cli.ts:671).
2. **`--root <path>`** is scanned across the *whole* argv (`args.indexOf("--root")`, first occurrence only) regardless of position, i.e. `pty list --root /x` and `pty --root /x list` both work, and `pty send foo --root /x` would also consume it (cli.ts:677-686). If the next token is missing or starts with `-`: stderr `pty: --root requires a path (e.g. pty --root /var/lib/pty-eval list)`, exit 1 (cli.ts:680-683). Otherwise `process.env.PTY_ROOT = val` and both tokens are spliced out (cli.ts:684-685).
3. **Root length backstop** (cli.ts:703-717): `resolvedRoot = PTY_ROOT ?? PTY_SESSION_DIR`; if set and `byteLength(root, utf8) + 14 > 104` (14 = `/` + 8-char id + `.sock`), stderr:
   ```
   pty: PTY_ROOT is too long — <N> bytes; must be ≤ 90 bytes for the socket path to fit the 104-byte kernel limit.
     root: <root>
     Shorten the root (or use `pty --root <shorter-path>` for a one-off).
   ```
   exit 1. (`usable` = 104-14 = 90.)
4. **Subcommand detection** (cli.ts:726-733): first token that does not start with `-`, skipping the token following a `--filter-tag`.
5. If there is no subcommand, or it is `i` / `interactive` (cli.ts:738-742): `preselectNew = args.includes("--preselect-new")`, `interactiveForce = args.includes("--force")`, and every `--filter-tag k=v` pair is extracted (mutating `args`) via `extractFilterTags` (tags.ts:17-31); a `--filter-tag` without a following token containing `=` prints `--filter-tag expects "key=value"` to stderr, exit 1 (cli.ts:96-103).
6. `dispatchArgs = args.filter(a => a !== "--preselect-new" && a !== "--force")` (cli.ts:743). If `dispatchArgs` is empty -> run the interactive TUI (cli.ts:745-748). So `pty`, `pty --force`, `pty --preselect-new`, `pty --filter-tag k=v` all open the TUI.
7. `command = dispatchArgs[0]` (cli.ts:750). Note subcommands parse from `args` (which still contains `--force`), not `dispatchArgs`.
8. **Per-command help** (cli.ts:756-758): if `args[1]` is `-h` or `--help` **and** `printCommandHelp(command)` finds an entry (aliases `a`->`attach`, `ls`->`list`, `remove`->`rm` resolved; cli.ts:472-478) it prints that entry to stdout and returns (exit 0). Only the *first* token after the command counts, so `pty send <ref> --help` sends the literal text `--help`. Commands without a `COMMAND_HELP` entry (`interactive`, `i`, `help`, `version`, `completions`) fall through to their own handling.
9. `switch (command)` (cli.ts:760-1661). Cases: `interactive`, `i`, `run`, `attach`, `a`, `exec`, `peek`, `send`, `events`, `list`, `ls`, `remote-serve`, `stats`, `restart`, `kill`, `recover`, `gc`, `tag`, `tag-multi`, `emit`, `up`, `down`, `rename`, `metadata`, `evidence`, `rm`, `remove`, `test`, `completions`, `version`, `--version`, `-v`, `-V`, `help`, `--help`, `-h`, and `default`.
10. **`default` = git-style forwarding** (cli.ts:1641-1660): runs `which pty-<command>` (external `which` binary via `execFileSync`, stdout trimmed; any failure -> not found) (cli.ts:1643-1647). If found: `spawnSync(extPath, args.slice(1), { stdio: "inherit", env: process.env })` and `process.exit(result.status ?? 1)` (cli.ts:1650-1655). Note `args.slice(1)` is the *unfiltered* args after `--root` removal, so `--force`/`--preselect-new` tokens are forwarded, and `PTY_ROOT` set by `--root` is inherited by the extension. If not found: stderr `Unknown command: <command>`, then the full top-level usage on **stdout**, exit 1 (cli.ts:1657-1659).

Argument parsing style: every subcommand hand-parses `args`; there is no generic option parser. Unknown flags are generally either treated as positionals or terminate a flag loop (details per command below). There is no `--flag=value` support except in `gc` (`--interval=N`, `--idle-days=N`, `--fast-fail-window=N`, `--fast-fail-limit=N`).

### 1.3 Global flags / verbs

| Spelling | Behavior | Source |
|---|---|---|
| `--root <path>` | Sets `PTY_ROOT` for this invocation; any position; first occurrence only. | cli.ts:677-686 |
| `pty help`, `pty --help`, `pty -h` | Print top-level usage (section 3.1) to stdout; exit 0. | cli.ts:1634-1638 |
| `pty version`, `pty --version`, `pty -v`, `pty -V` | Print version to stdout; exit 0. | cli.ts:1626-1632 |
| `--preselect-new` | TUI only (pre-select "Create new session..."). Removed from `dispatchArgs` for all commands. | cli.ts:739,743 |
| `--filter-tag k=v` (repeatable) | Before the subcommand: TUI filter + inherited tags for created sessions. Only consumed here when the subcommand is empty/`i`/`interactive`; `list` and `tag-multi` parse their own. | cli.ts:741 |
| `--force` | TUI: bypass nesting guard. Also a per-command flag for `run`, `attach`, `restart`. | cli.ts:740,743 |

Version format (src/version.ts): `readPackageVersion()` reads `../package.json` relative to the module dir (`dist/` or `src/`), falling back to `"0.0.0"` (version.ts:13-20). `readGitShortSha()` returns `git rev-parse --short HEAD` run in the package root **only if** `<pkgroot>/.git` exists and the output matches `/^[0-9a-f]{4,}$/`, else `null` (version.ts:25-38). `formatVersion(v, sha)` = `sha ? `${v}+${sha}` : v` (version.ts:42-44). Printed with `console.log` (version.ts:47-49). So an npm-installed copy prints `0.12.0`; a git checkout prints e.g. `0.12.0+abc1234`.

### 1.4 Environment variables (complete list of `process.env` reads on the CLI/daemon path)

| Variable | Read at | Meaning |
|---|---|---|
| `PTY_ROOT` | sessions.ts:83; set by `--root` at cli.ts:684 | Canonical registry directory. Wins over `PTY_SESSION_DIR`. Default `~/.local/state/pty` (`path.join(os.homedir(), ".local","state","pty")`, sessions.ts:24). Directory is created `mode 0o700, recursive` on demand (sessions.ts:112-114). |
| `PTY_SESSION_DIR` | sessions.ts:84 | Deprecated alias. Used only when `PTY_ROOT` is unset/empty. First use prints once to stderr (raw write): `pty: PTY_SESSION_DIR is deprecated; use PTY_ROOT (same shape, canonical name).\n` (sessions.ts:101-105). If **both** set, prints once: `pty: both PTY_ROOT and PTY_SESSION_DIR are set — using PTY_ROOT (<root>); PTY_SESSION_DIR (<legacy>) is ignored (deprecated). For isolation, set PTY_ROOT.\n` (sessions.ts:91-97). Also referenced in the `validateName` error text (sessions.ts:61) and in the isolate-env allow-list (server.ts:139). |
| `PTY_ROOT_LEGACY_SILENT` | sessions.ts:91,101 | Any non-empty value suppresses both notices above. |
| `PTY_SESSION` | cli.ts:631, 888, 908, 1866, 2927, 2970, 3003, 3544, 3956 | Set by the daemon in the child's env to the session's stable id (server.ts:174,190,205); cannot be removed with `--unset-env` (server.ts:186-190 sets it after removals). Drives: nesting guard (`ensureNotNested`), nested `run` direct-exec, `exec`, `rename` single-arg / `--clear` default, `emit` default ref, `restart` "not attached" note. |
| `PTY_SESSION_GENERATION` | cli.ts:1871; server.ts:175,191,206 | Opaque generation token injected into the child env; **required** by `pty exec` (missing -> throws `pty exec: current session has no generation owner token; restart it before using pty exec.`, exit 1 via main catch). |
| `PTY_CREATION_LOCK_OWNER_PID` | cli.ts:1681-1686; spawn.ts:305-309 | Internal one-hop handoff: when the library falls back to shelling out to `pty run -d ...` it passes the PID that already holds the per-name creation lock; `cmdRun` checks `isLockOwnedByPid(name, pid)` and, if true, skips acquiring the event/creation locks; the var is deleted from `process.env` immediately (cli.ts:1686) so the daemon never inherits it. |
| `PTY_REAP_ON_EXIT` | sessions.ts:1094 (daemon side, server.ts:1523) | Daemon reads its own env (inherited from the `pty run` invocation). `false`/`0`/`no`/`off` (trimmed, case-insensitive) -> preserve finished non-permanent sessions; unset/anything else -> reap (shipped default). Overridden per session by `keep` tag (preserve) and `--ephemeral` (reap). |
| `PTY_SHUTDOWN_DEADLINE_MS` | server.ts:1535 | Daemon graceful-shutdown backstop, default 5000 ms; positive finite number overrides. |
| `PTY_SPAWNER_PID` | server.ts:1440 | Daemon watchdog: poll the PID every 5 s and shut down when it dies. Set only by the library `spawnDaemon({bindToSpawnerLifetime:true})` (spawn.ts:197); the CLI never sets it. |
| `PTY_SERVER_CONFIG` | spawn.ts:191 (set); server.ts:1468 (read); server.ts:185 (deleted from child env) | JSON config handed from `spawnDaemon` to the daemon process. Daemon entry requires `name` and `command`, else stderr `PTY_SERVER_CONFIG env var required`, exit 1 (server.ts:1469-1472). |
| `PTY_RECONNECT_MAX_ATTEMPTS` | client.ts:440 | `attach --remote` reconnect cap; positive integer, else unlimited. |
| `PTY_FABRIC_BIN` | remote.ts:16 | Path of the `fabric` binary for `--remote`; default `fabric`. |
| `PTY_REMOTE_SERVE_DEBUG` | cli.ts:2121 | Truthy -> `remote-serve --socket` logs `[remote-serve <iso-ts>] ...` lifecycle lines to stderr. |
| `PATH` | cli.ts:3236 | Baked into the launchd plist (`EnvironmentVariables.PATH`), default `/usr/bin:/bin:/usr/sbin:/sbin`. Also implicitly consulted by `which` for command resolution and forwarding. |
| `HOME`, `SHELL` | tui/interactive.ts:41,46 | TUI "create new session" defaults (dir picker start; shell defaults to `bash`). |
| `ST_AGENT`, `ST_ROOT` | cli.ts:3869 (`RESTART_SCRUBBED_ENV`), spawn.ts:194-196 | Deleted from the daemon env on `pty restart` and on the dead-session "Restart? [Y/n]" path so a restarted session does not inherit the restarter's bus identity. Not scrubbed on fresh `pty run`. |
| `TERM` | server.ts:150-156 | Child env: if absent after policy, set to `xterm-256color`. |
| `PTY_TEST_BIN` | bin/pty-kill-releases-socket-test:111 | Test-script only. |
| `PTY_VITEST_RUN_ROOT`, `TMPDIR` | tests/setup/vitest-global.ts | Test harness only. |

Daemon-side child env policy (server.ts:158-209), user-visible through `--env/--unset-env/--isolate-env`: see section 5.4.

### 1.5 Exit code conventions

- `0`: success; user answered `n` to a restart prompt (cli.ts:1838-1840, 3924-3926); detach from attach/peek -f (cli.ts:1860, 2011); `pty completions <shell>`/`--help` (completions.ts:730-745); `pty tag-multi --help` (cli.ts:3314-3317).
- `1`: every usage/validation error, "not found", ambiguous ref, connection failure, timeouts (`peek --wait`, `events --wait`), any uncaught exception (cli.ts:4132-4135).
- `2`: only `pty completions` with a missing or unknown shell (completions.ts:734-743).
- Session exit code: `pty attach`, `pty run` (attached), `pty peek -f`, `pty run -a` (attaching) exit with the session's exit code when the session process ends (`onExit: (code) => process.exit(code)`, cli.ts:1861, 2012, 2040, 2100). The daemon encodes a signal death as `128 + signal` (server.ts:578). Decode of a short EXIT payload yields `-1` (protocol.ts:120-125).
- Pass-through: forwarded `pty-<cmd>` extension status (`?? 1`) (cli.ts:1654); nested `pty run` direct exec status (`?? 1`) (cli.ts:917); `pty exec` child status (`?? 1`) (cli.ts:1938); `pty test` vitest status (`?? 1`) (cli.ts:4066).
- `process.exitCode = 1` (not immediate exit) for `pty kill` failures after the SIGTERM attempt (cli.ts:2644, 2661).

### 1.6 stdout vs stderr conventions

- Success output and data (`list`, `peek`, JSON, `tag` dumps, "Session ... created/killed/removed/restarted.", `gc` report, help) -> stdout.
- Errors, hints, deprecation notices, warnings -> stderr. Notable **stderr non-errors**: `Hint: use --name instead: ...` (cli.ts:843, 855); `Already inside pty session "<s>", running directly.` (cli.ts:907-909); `Warning: this session is managed by <toml>` block after `pty tag` writes (cli.ts:1530-1532); `Note: this session is managed by <toml>` / `The strategy tag will be restored on the next 'pty up'.` after `pty kill` (cli.ts:2668-2669); `Note: strategy tags will be restored on the next 'pty up'.` after `pty down` (cli.ts:3843); `pty up`/`pty down` per-session `  ✗ <label>: <error>` lines (cli.ts:3706, 3720, 3724, 3736, 3747, 3765, 3820, 3829); `pty tag-multi` per-session write errors (cli.ts:3483); `peek --wait` exit diagnostics (cli.ts:1976-1981).
- Exceptions: `Unknown command: X` -> stderr, then full usage -> **stdout** (cli.ts:1657-1658). `pty rename` usage on error paths -> stderr (`renameUsage`, cli.ts:2809-2813) but `pty rename --help` -> stdout. `pty restart` nested "not attached" note -> **stdout** (cli.ts:3958). `pty tag-multi` "0 sessions matched." -> stdout. `pty evidence` JSON -> `fs.writeFileSync(1, ...)` (cli.ts:2917, 2923). `pty completions` usage -> stdout for `--help`, stderr for errors (completions.ts:731, 735, 740-741).
- `attach --remote` reconnect status lines go to stdout normally, but to **stderr** when `--attach-stream-fd-v1` is active (client.ts:709).

### 1.7 TTY detection

- `attach` (client.ts:444-752): raw mode is enabled only when `stdin.isTTY` (client.ts:473-475); initial size = `stdout.rows ?? 24`, `stdout.columns ?? 80` (client.ts:581-582); the resize listener is installed only when `stdout instanceof tty.WriteStream` (client.ts:573-576); stdin is always `resume()`d (client.ts:572). No TTY is *required* — attaching with piped stdio works.
- `peek -f`: raw mode only if `stdin.isTTY` (client.ts:88, 94); on close, raw mode is reset if `stdin.isTTY && stdin.isRaw` (client.ts:180-182).
- `spawnDaemon` (spawn.ts:165-167): `rows = stdout.rows ?? 24`, `cols = stdout.columns ?? 80` of the **creating CLI's** stdout; persisted as `rows`/`cols` in metadata and reused by `restart`.
- Prompts (`ask`, cli.ts:4069-4078) use `readline` on stdin/stdout unconditionally (no TTY check); answer `n`/`N` (lowercased equals `"n"`) declines; anything else (including empty) proceeds.
- `pty list` text output always emits ANSI SGR codes (no isatty/NO_COLOR check) (cli.ts:2362-2364, 2378, 2396, 2407, 2415-2416, 2425, 2428, 2439-2440).
- `pty peek` (non-plain) writes `TERMINAL_SANITIZE + CURSOR_TO_BOTTOM` after the screen regardless of TTY (client.ts:131-134).
- The TUI is not TTY-gated in cli.ts; the `tui/` app handles it.

### 1.8 Signal handling in the client

- **Attach**: no SIGINT handler; in raw mode Ctrl+C is just byte `0x03` forwarded to the session. **Detach key** is Ctrl+\ (`0x1c`); the kitty-protocol encoding `\x1b[92;5u` is normalized to `0x1c` first (client.ts:20-31). First tap arms a 300 ms timer; a second tap within 300 ms cancels the detach and forwards a literal `0x1c` to the session (client.ts:540-569). On detach: sends a DETACH packet, restores the terminal, writes `TERMINAL_SANITIZE + "\x1b[999;1H" + "\r\n[detached]\r\n"` to stdout (client.ts:518-521) and the CLI exits 0 (cli.ts:1860). On session exit: `TERMINAL_SANITIZE + CURSOR_TO_BOTTOM + "\r\n[<name> exited with code <N>]\r\n"` then exit N (client.ts:651-656). `TERMINAL_SANITIZE` is the exact byte string at client.ts:37-55 (`\x1b[?1049l\x1b[?1l\x1b[?7h\x1b[?6l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1006l\x1b[?25h\x1b[?2004l\x1b[4l\x1b[r\x1b[0m\x1b[0 q\x1b>\x1b(B\x1b[<99u`).
- **SIGWINCH**: Node's stdout `resize` event -> `RESIZE` packet with `stdout.rows/columns` (client.ts:573-576).
- **`events` (follow and `--wait`)**: `process.on("SIGINT")` -> stop follower, exit 0 (cli.ts:4028-4031, 4044-4047); follow mode keeps the process alive with a 60 s interval (cli.ts:4050).
- **`remote-serve --socket`**: `SIGHUP` ignored (logged in debug) (cli.ts:2143); `SIGTERM`/`SIGINT` -> close server, unlink socket, resolve -> exit 0 (cli.ts:2152-2162); keep-alive interval (cli.ts:2142).
- **`remote-serve --stdio`**: no handlers; exits 0 when the single interaction ends (remote.ts:193-196).
- **`peek -f`**: Ctrl+\ (single tap, no double-tap logic) -> destroy socket, print `TERMINAL_SANITIZE + CURSOR_TO_BOTTOM + "\r\n[detached]\r\n"`, exit 0 (client.ts:90-103). All other input ignored. On session exit in follow mode: `\r\n[<name> exited with code N]\r\n`, exit N (client.ts:146-157).
- **Daemon** (for reference): `SIGTERM`/`SIGINT` -> external-kill shutdown, exit 0, metadata kept (server.ts:1598-1603).

### 1.9 `<ref>` resolution (shared)

- `getSession(ref)` (sessions.ts:1351-1363): `listSessions()`, exact `name === ref` wins; else all sessions with `metadata.displayName === ref`; 0 -> `null`; 1 -> that session; >1 -> **throws** `Session reference "<ref>" is ambiguous. Matching stable session IDs:\n  <id1>\n  <id2>\nUse a stable session ID instead.` (ids sorted ascending). The throw surfaces via `main().catch` as that message on stderr, exit 1.
- `resolveRef(ref)` (cli.ts:610-617): wraps `getSession`; `null` -> stderr `Session "<ref>" not found.`, exit 1; returns the stable name. Used by `attach`, `peek`, `send`, `events <ref>`, `restart`, `kill`, `rm`, `tag`, `emit`.
- `stats`, `rename`, `tag-multi` call `getSession` directly with their own not-found text (see per-command).
- `metadata patch --id`, `evidence --id`, `recover <name>` and `run --id` use the exact id only (`getSessionByName`, sessions.ts:1100-1103; `validateName`).
- `--remote <peer>` forms do **not** resolve locally; the ref is sent to the peer, which resolves it with the same `getSession` rules (remote.ts:132-143).

### 1.10 Session status model (what `list`/`stats`/etc. see)

`listSessions()` (sessions.ts:895-1013) scans `PTY_ROOT` once (returns `[]` if unreadable):
- For every `<name>.sock`: read `<name>.pid`; alive if the pid is alive (`kill(pid,0)`; EPERM counts as alive) **or** the socket is connectable (probed concurrently with a shared 500 ms budget, sessions.ts:611, 929-934). Alive -> status `exited` if `metadata.exitedAt` is set, else `running`. Dead pid -> status `vanished` if `exitedAt == null && exitCode == null`, else `exited` (only if metadata exists). Unreadable pid + unreachable socket -> reported `running` unless `exitedAt` set (defensive).
- For every `<name>.json` not seen above: if a pid (pidfile or `metadata.daemonPid`, sessions.ts:2086) is alive -> `running`/`exited` by `exitedAt`; else `vanished` when no `exitedAt` and no `exitCode`, otherwise `exited`.
- Result sorted by `name` ascending (ASCII `<`) (sessions.ts:1012). `pid` is `null` for dead sessions.
- `isGone(status)` = `exited || vanished` (sessions.ts:241-243).
- Listing never mutates the registry (README.md:229-230).

### 1.11 Name / displayName validation (shared)

- `validateName(name)` (sessions.ts:35-64), in order: empty -> `Session name cannot be empty.`; `.`/`..` -> `Invalid session name "<n>". Names cannot be "." or "..".`; `> 255` chars -> `Session name too long (max 255 characters).`; not `/^[a-zA-Z0-9._-]+$/` -> `Invalid session name "<n>". Names may only contain letters, numbers, dots, hyphens, and underscores.`; `byteLength(<root>/<name>.sock) > 104` -> `Session name "<n>" produces a socket path of <B> bytes, which exceeds the 104-byte kernel limit by <B-104>. Shorten the name or set PTY_SESSION_DIR to a shorter path.`
- `validateDisplayName(dn)` (sessions.ts:67-80), in order: `""` -> `Display name cannot be empty.`; `dn !== dn.trim()` -> `Display name must be trimmed.`; `Array.from(dn).length > 160` -> `Display name too long (max 160 Unicode scalars).`; matches `/[\p{Cc}\u2028\u2029]/u` -> `Display name must be single-line and contain no control characters.` (`/` and `\` allowed.)
- Random id (`randomSessionName`, cli.ts:642-648): 8 chars, alphabet `23456789abcdefghjkmnpqrstuvwxyz` (31 symbols; no `0 1 o i l`), each char = `randomBytes(8)[i] % 31` (slightly biased). Up to 8 attempts to avoid collision with `allSessionNames()`; failure -> `Could not generate a unique session id after 8 attempts.` exit 1 (cli.ts:944-953; also `pty up`, cli.ts:3730-3739).
- Auto displayName (`autoName`, cli.ts:651-668 + sanitize at cli.ts:971-973): `dirPart = basename(process.cwd())`; `cmdBase = basename(cmd as typed)`; `firstArg` = first arg not starting with `-` and `length < 30`; `argBase = basename(firstArg)` with last `.ext` stripped; if `argBase` matches `/^[a-zA-Z0-9._-]+$/` then `cmdPart = "<cmdBase>-<argBase>"` else `cmdBase`; result `"<dirPart>-<cmdPart>"`; then every char not in `[a-zA-Z0-9._-]` -> `-`, runs of `-` collapsed, leading/trailing `-` stripped. E.g. in `/home/u/myapp`, `pty run -- node server.js` -> `myapp-node-server`.

---

## 2. COMMANDS

Each entry: syntax, flags, parsing quirks, text output, JSON shape, exit codes, error strings, ref resolution, inside-session behavior. "Test-pinned" bullets (added in the per-command "Pinned by tests" subsections) cite the vitest files that assert the behavior.

### 2.0 `pty` / `pty i` / `pty interactive` — interactive TUI (cli.ts:84-93, 738-748, 761-765)

- Syntax: `pty [--preselect-new] [--filter-tag k=v ...] [--force]`, `pty i ...`, `pty interactive ...`.
- Nesting guard first (cli.ts:85-90): if `PTY_SESSION` is set and no `--force`: stderr
  ```
  pty interactive: already inside pty session "<s>".
    The interactive picker would render inside your current session and detach would route to the outer client.
    Detach first (Ctrl+\) and run `pty` from outside, or pass --force to open the picker anyway.
  ```
  exit 1.
- Lazily imports `./tui/interactive.ts` (so other commands work when cwd was deleted) and calls `runInteractive({ preselectNew, filterTags, force })` (cli.ts:91-92). `runInteractive` (tui/interactive.ts:725-767): applies `filterTags` (filters the list to sessions matching ALL tags and stamps them on sessions created from the TUI, tui/interactive.ts:628-639), lists sessions, pre-selects the `+ Create new session...` item when `preselectNew`, fetches pty-relay hosts in the background (`which pty-relay` then `pty-relay ls --json`, tui/interactive.ts:116-121), polls `listSessions()` every 1 s while the home screen is visible.
- Keys (README.md:38-43; tui/interactive.ts:455-465, 753-756): arrows navigate, typing filters (`host/session` syntax filters by relay host), Enter attaches, `q` quits (when the filter is empty), Esc clears the filter or quits, Ctrl+C quits (global), Ctrl+G cycles theme (stored in `$PTY_ROOT/theme`, docs/disk-layout.md:20). After a detach from a session entered via the list you return to the list.
- Create flow (tui/interactive.ts:625-658): random id (`randomSessionName` equivalent), `displayName` intentionally unset (like `--no-display-name`), command = `$SHELL` (default `bash`), cwd = picked dir (starts at `$HOME`), tags = the `--filter-tag` set; creation errors go to stderr (`Session "<name>" is being created by another process.` or the thrown message).
- Exit: 0 on quit. Not exercised by the CLI-level tests beyond dispatch.

### 2.1 `pty run` (cli.ts:767-982, `cmdRun` cli.ts:1664-1769)

Syntax: `pty run [flags] -- <command> [args...]`. Legacy forms without `--` still parse (see below).

Flags (parsed in a loop that stops at the first unrecognized token or at `--`, cli.ts:789-829; every value flag requires `i + 1 < args.length`, otherwise the token is treated as unrecognized and ends the loop):

| Flag | Type | Effect |
|---|---|---|
| `-d`, `--detach` | bool | Create in background; no attach; exit 0 after `Session "<id>" created.` |
| `-a`, `--attach` | bool | If a session with the same **id** exists and is running -> attach to it; if it exists and is gone -> recreate it reusing previous cwd/tags/env/displayName unless overridden; also suppresses the "id already in use" check (cli.ts:938). |
| `-e`, `--ephemeral` | bool | Force self-removal of the registry entry on any shutdown (incl. `pty kill`, `strategy=permanent`); `keep` tag still wins (sessions.ts:1069-1081). Persisted as `ephemeral: true`. |
| `--isolate-env` | bool | Child env restricted to allow-list (section 5.4). Persisted as `isolateEnv: true`. |
| `--no-display-name` | bool | displayName stays unset. |
| `--force` | bool | Bypass the nesting guard: create (and attach to) a nested session. |
| `--id <id>` | string | Pin the stable id. `validateName` (section 1.11); if it exists and not `-a`: stderr `Session id "<id>" is already in use.` exit 1 (cli.ts:938-941). |
| `--name <label>` | string | displayName; `validateDisplayName`; failure -> stderr `Invalid displayName: <msg>` exit 1 (cli.ts:963-968). |
| `--cwd <path>` | string | Passed **verbatim** to the daemon config (spawn.ts:174); relative paths are resolved by the daemon relative to the CLI's cwd (the daemon inherits it) and the relative string is what gets stored in metadata `cwd`. Invalid cwd surfaces as a daemon early-exit error (see below). |
| `--tag k=v` | repeatable | `indexOf("=") === -1` -> stderr `Invalid tag format: "<tok>". Use --tag key=value` exit 1 (cli.ts:800-804). Empty key allowed here (`=v`). Later duplicates overwrite. |
| `--env KEY=VALUE` | repeatable | `eq <= 0` (no `=` or empty key) -> `Invalid env format: "<tok>". Use --env KEY=VALUE` exit 1 (cli.ts:811-814). Later duplicates overwrite. Persisted as `extraEnv`. |
| `--unset-env KEY` | repeatable | empty or contains `=` -> `Invalid env key: "<tok>". Use --unset-env KEY` exit 1 (cli.ts:820-823); de-duplicated, order preserved. Persisted as `unsetEnv`. |

Command extraction (cli.ts:831-865):
- If a `--` exists at/after the loop index: tokens between the last parsed flag and `--` are a legacy positional: the first becomes `explicitDisplayName` (if `--name` not given) and stderr prints `Hint: use --name instead: pty run --name <tok> -- ...` (cli.ts:840-844). `cmd = args[dashDash+1]`, `cmdArgs = rest`.
- No `--`: `rest = args.slice(i)`; if no `--name` and `rest.length >= 2`: `displayName = rest[0]`, `cmd = rest[1]`, `cmdArgs = rest.slice(2)`, stderr `Hint: use --name instead: pty run --name <rest0> -- <cmd> <args...>` (trimmed end) (cli.ts:851-855). Otherwise `cmd = rest[0]`, `cmdArgs = rest.slice(1)`.
- No `cmd` -> stderr `Usage: pty run [--id <id>] [--name <displayName>] [-d] [-a] -- <command> [args...]` exit 1 (cli.ts:862-865).
- `displayCmd = [cmd, ...cmdArgs].join(" ")` **as typed** (before resolution) (cli.ts:868) -> metadata `displayCommand`.
- `cmd = resolveCommand(cmd)` (spawn.ts:372-393): absolute path must exist; a token containing `/` is `path.resolve`d and must exist; otherwise `which <cmd>` (external binary) trimmed. Failure -> stderr `Command not found: <cmd>` exit 1 (cli.ts:869-874). The resolved absolute path is stored as metadata `command` and exec'd by the daemon via `/bin/sh -c 'exec "$@"' sh <command> <args...>` (server.ts:539-548).

Inside a session (`PTY_SESSION` set) and neither `-d` nor `--force` (cli.ts:888-918):
- If `-a` and a lookup ref (`--id` else `--name`/legacy positional) resolves (`--id` via exact id; otherwise via `getSession(displayName)`) to a **running** session: stderr
  ```
  pty run -a: already inside pty session "<s>".
    Target session "<ref>" is already running; attaching would nest a client inside the current session.
    Pass --force to attach anyway, or detach first (Ctrl+\) and re-run from outside.
  ```
  exit 1 (cli.ts:894-905).
- Otherwise stderr `Already inside pty session "<s>", running directly.` and the command is run **in place** with `spawnSync(cmd, cmdArgs, { stdio: "inherit", env })` where `env = process.env` minus `--unset-env` keys plus `--env` overlays; exit with the child's status (`?? 1`) (cli.ts:907-917). `--cwd`, `--tag`, `--id`, `--name`, `--isolate-env`, `-e` are ignored on this path. (README.md:174-178.)

Outside a session (or `-d`/`--force`):
1. `existingNames = allSessionNames()` (ids of every live or retained session).
2. Id selection as in the table; random ids use up to 8 attempts (cli.ts:944-953).
3. displayName precedence: `--no-display-name` -> none; `--name`/legacy positional -> validated; else auto label (section 1.11) (cli.ts:960-975).
4. `cmdRun(...)` (cli.ts:1664-1769):
   - Lock handoff: `PTY_CREATION_LOCK_OWNER_PID` (section 1.4). Otherwise acquire the event lock (`Session "<id>" event log is busy. Try again.` exit 1, cli.ts:1690-1693) then the creation lock (`Session "<id>" is being created by another process. Try again.` exit 1, cli.ts:1695-1701).
   - If the session is `running`: release locks; with `-a` -> stdout `Session "<id>" already running, attaching.` then attach (exit = session exit code / 0 on detach) (cli.ts:1715-1719); without -> stderr `Session "<id>" is already running. Use "pty attach <id>" to connect.` exit 1 (cli.ts:1720-1723).
   - If the session is gone (exited/vanished): its files are removed (`cleanupAllWhileLocked`) and `cwd`, `tags`, `extraEnv`, `unsetEnv`, `displayName` from the old metadata are reused when the new invocation did not supply them (`tags` reused only if no `--tag` given at all; same for env lists) (cli.ts:1726-1748).
   - `spawnDaemon({ name, command, args, displayCommand, cwd: --cwd ?? previous, ephemeral, tags, creationLockOwnerPid, displayName?, isolateEnv?, extraEnv?, unsetEnv? })` (cli.ts:1749-1756). `spawnDaemon` (spawn.ts:138-243): rows/cols from the CLI's stdout (`?? 24/80`), `cwd` defaults to `process.cwd()`, spawns `node dist/server.js` detached with `PTY_SERVER_CONFIG` JSON, waits for `<id>.sock` (default 30 s, `DEFAULT_START_TIMEOUT_MS`) and then for metadata with `daemonPid === child.pid` plus a `session_start` event line with `ts >= createdAt` (spawn.ts:224-236). Failure messages (thrown -> stderr, exit 1): `Daemon process exited immediately (code <N|unknown>).\n<daemon stderr>` or `Daemon process exited immediately (code N). Is the command valid?` (spawn.ts:219-222); `Timeout waiting for session "<id>" to start` (spawn.ts:354); `Timed out waiting for daemon publication for session "<id>".` (spawn.ts:233). Daemon-side cwd errors that appear inside the first message: `Working directory is empty.`, `Working directory does not exist: <cwd>`, `Working directory is not accessible: <cwd> (<err>)`, `Working directory is not a directory: <cwd>`, `Working directory is not searchable: <cwd>`, each followed by `\nCannot start session "<id>" for command "<cmd>".` (server.ts:236-260, 524-529). On failure the CLI SIGTERMs the child if it is still alive (spawn.ts:238-240).
   - stdout `Session "<id>" created.` (cli.ts:1762). `-d` -> return (exit 0). Else `doAttach(id)` (cli.ts:1856-1863): exit 0 on detach, session exit code on exit.
- Bundled-library fallback (`spawnViaCli`, spawn.ts:277-334) invokes the CLI as `pty run -d --id <name> [--name <dn> | --no-display-name] [--cwd <cwd>] [--ephemeral] [--isolate-env] [--env K=V ...] [--unset-env K ...] [--tag k=v ...] -- <command> <args...>` with `PTY_CREATION_LOCK_OWNER_PID` set — the Rust port must accept exactly that shape.

### 2.2 `pty attach` / `pty a` (cli.ts:984-1053, `cmdAttach` 1773-1806, `handleDeadSession` 1808-1854)

Syntax: `pty attach [-r|--auto-restart] [--no-restart] [--force] [--remote <peer>] [--attach-stream-fd-v1 <fd>] <ref>`.

Parsing (cli.ts:992-1010): flags may appear anywhere; `--remote` consumes the next token (only if one exists, otherwise `--remote` itself becomes the name); `--attach-stream-fd-v1` without a following token -> stderr `pty attach: --attach-stream-fd-v1 requires a file descriptor` exit 1; its value is `Number(token)`. The first token that is not a recognized flag is the ref (so an unknown `-x` becomes the ref); a second -> `pty attach: unexpected argument "<tok>"` exit 1.

Validation order:
1. No ref -> `Usage: pty attach [-r|--auto-restart|--no-restart] [--force] [--remote <peer>] <name>` exit 1 (cli.ts:1011-1014).
2. `-r` with `--no-restart` -> `pty attach: --auto-restart and --no-restart are mutually exclusive` exit 1 (cli.ts:1015-1018).
3. fd: `validateAttachStreamFdV1` (client.ts:416-429): not a safe integer or `< 3` -> `pty attach: --attach-stream-fd-v1 requires a dedicated inherited file descriptor >= 3 (got <fd>)`; `fstat`/zero-length write failure -> `pty attach: --attach-stream-fd-v1 descriptor <fd> is not writable: <detail>`; exit 1. Then `-r` with fd -> `pty attach: --attach-stream-fd-v1 and --auto-restart are mutually exclusive` exit 1 (cli.ts:1019-1030).
4. Nesting guard **before** ref resolution (cli.ts:1036-1042): without `--force` and with `PTY_SESSION` set, stderr
   ```
   pty attach: already inside pty session "<s>".
     Attaching now would nest a client inside the current session — detach keys route to the outer client and get tangled.
     Detach first (Ctrl+\) or, from inside pty-layout, use ^]n to pick a session.
     Pass --force to attach anyway (nested clients are usually a mistake).
   ```
   exit 1. Applies to `--remote` too.
5. `--remote <peer>`: `cmdAttachRemote` (cli.ts:2073-2102) — `fabric dial <peer> pty-remote` + route (10 s), failure -> stderr `pty attach --remote <peer>: <msg>` exit 1; then `attach()` over the routed socket with a reconnect callback (transport failure -> retry with backoff `100,250,500,1000,2000,5000,10000` ms then 15000 ms cap, unlimited attempts unless `PTY_RECONNECT_MAX_ATTEMPTS`; `RouteRefusedError` -> clean stop). Status lines (client.ts:709-746): `\r\n[reconnecting… — Ctrl-\ or Ctrl-C to stop]\r\n`; on refusal `TERMINAL_SANITIZE + CURSOR_TO_BOTTOM + "\r\n[<name> session ended]\r\n"` exit 0 (exit 1 and plain `[<name> session ended]\n` on stderr in fd mode); on attempt cap `...[<name>: connection lost — re-run `pty attach --remote` to reconnect]...` exit 1.
6. Local: `resolveRef` (section 1.9). Restart policy: fd set or `--no-restart` -> `never`; `-r` -> `always`; else `prompt` (cli.ts:1048-1049).

`cmdAttach` (cli.ts:1773-1806): `getSession` again (`Session "<name>" not found.` exit 1); `running` -> `doAttach`. Not running and policy `never` -> stderr `Session "<name>" is not running (status: <exited|vanished>).` exit 1 (cli.ts:1799-1802). Otherwise `handleDeadSession`:
- No metadata -> stderr `Session "<name>" exited (no metadata available).`, `cleanupAll`, exit 1 (cli.ts:1813-1817).
- If `lastLines` non-empty: stdout blank line, each line as `  <line>`, blank line (cli.ts:1820-1826).
- stdout `Session "<name>" exited with code <exitCode|unknown>.` then `Command was: <displayCommand> <args...>` (note: `displayCommand` already contains the args, so args are printed twice for sessions created by `pty run`, e.g. `Command was: node server.js server.js` — cli.ts:1832-1833) then a blank line.
- Policy `prompt`: `Restart? [Y/n] ` via readline; lowercase answer `n` -> exit 0 (cli.ts:1836-1841). Policy `always` skips the prompt.
- Restart: `cleanupAll(name)`, `spawnDaemon` with stored `command/args/displayCommand/cwd/tags/displayName` + persisted launch options (`rows`, `cols`, `ephemeral`, `isolateEnv`, `extraEnv`, `unsetEnv`, `env`; cli.ts:3874-3884) + `scrubEnv: ["ST_AGENT","ST_ROOT"]`; stdout `Session "<name>" restarted.`; attach (cli.ts:1845-1853). Flapping bookkeeping tags are NOT stripped on this path (unlike `pty restart`).

Attach client behavior (client.ts:444-752): sends `ATTACH(rows, cols)`; on `SCREEN` writes `\x1b[2J\x1b[H` + payload; `DATA` -> stdout; `EXIT` -> `TERMINAL_SANITIZE + CURSOR_TO_BOTTOM + "\r\n[<name> exited with code <N>]\r\n"`, exit N. Connection errors: `ENOENT|ECONNREFUSED|ECONNRESET|EPIPE` -> stderr `Session "<name>" not found or not running.` (or `Remote session "<name>" not found or not running.`) exit 1; other -> `Connection error: <msg>` exit 1; a close without error finishes with the last known exit code (0). Malformed packet -> stderr `pty client: dropping connection — <msg>`.

Machine mode `--attach-stream-fd-v1 <fd>` (client.ts:409-413, 469-471, 596-641; docs/client.md:406-421): stdout receives **nothing** (no `[detached]`, no screen); the fd receives framed packets (`[type u8][len u32BE][payload]`, protocol.ts:23-50) in the order `GEOMETRY -> SCREEN -> DATA*/EXIT`; a local detach writes a `DETACH` packet (empty payload) to the fd and to the daemon, then exits 0; `EXIT` ends the stream and the CLI exits with the session code. Non-stream packet types are dropped. Errors (stderr, exit 1): `pty attach: daemon does not support attach stream v1 (expected GEOMETRY before terminal events)`, `pty attach: daemon does not support attach stream v1 (expected SCREEN before DATA|EXIT)`, `pty attach: machine stream truncated before EXIT: <msg>` / `...: connection closed`, `pty attach: machine stream descriptor <fd> failed: <msg>`. Backpressure: socket paused until the fd drains. The fd is never closed by the CLI (`autoClose: false`).

### 2.3 `pty exec` (cli.ts:1055-1066, `cmdExec` 1865-1939)

Syntax: `pty exec -- <command> [args...]`. Requires a `--` at index >= 1 with at least one token after it, else stderr `Usage: pty exec -- <command> [args...]` exit 1.

Only meaningful inside a session:
- No `PTY_SESSION` -> `pty exec: not inside a pty session (PTY_SESSION not set).` exit 1.
- No `PTY_SESSION_GENERATION` -> thrown `pty exec: current session has no generation owner token; restart it before using pty exec.` (exit 1).
- No metadata -> `pty exec: session "<s>" metadata not found.` exit 1.
- `resolveCommand` failure -> `Command not found: <cmd>` exit 1.
- Event lock busy -> thrown `pty exec: session "<s>" event log is busy; retry the operation.`
- Session tagged `ptyfile` -> thrown `pty exec: session "<s>" is managed by <toml-path>. Edit the pty.toml to change the command instead.`
- Metadata mutation with `expectedGeneration`: on `generation-mismatch` -> `pty exec: session "<s>" belongs to a replacement generation; command was not run.`; other non-`changed` -> `pty exec: session "<s>" could not be updated (<status>); command was not run.` (statuses: `unchanged`, `busy`, `missing`, `stale`; sessions.ts:338-344).
- Success: metadata `command` (resolved abs path), `args`, `displayCommand` (`[cmd,...args].join(" ")` as typed) rewritten atomically; a `session_exec` event `{session, type:"session_exec", ts, previousCommand, command}` appended (cli.ts:1914-1920); then the command is run with `spawnSync(resolved, args, { stdio: "inherit", env: process.env })` and the CLI exits with its status (`?? 1`) — it is **not** an `execve` replacement; the `pty` process remains the parent (cli.ts:1934-1938).

### 2.4 `pty peek` (cli.ts:1068-1107, `cmdPeek` 1992-2014, `cmdPeekWait` 1941-1990, `cmdPeekRemote` 2019-2042)

Syntax: `pty peek [-f|--follow] [--plain] [--full] [--wait <text>]... [-t|--timeout <sec>] [--remote <peer>] <ref>`.

Parsing (cli.ts:1076-1084): a loop consumes leading tokens starting with `-`; an unrecognized dash-token ends the loop and becomes the ref. Flags after the ref are ignored (they are never read). `--wait` is repeatable (any-of match). `-t` is `parseFloat` seconds; `<= 0`/NaN -> no timeout.
- No ref -> `Usage: pty peek [-f] [--plain] [--full] [--wait <pattern>] [-t <seconds>] [--remote <peer>] <name>` exit 1.
- `--remote` + `--wait` -> `pty peek --wait is not supported with --remote yet.` exit 1. Remote otherwise: dial error -> `pty peek --remote <peer>: <msg>` exit 1; then `peek()` over the routed socket.
- Local ref resolved with `resolveRef`.

`cmdPeek` (no `--wait`):
- Session gone (exited/vanished): if `metadata.lastLines` non-empty -> stdout `lastLines.join("\n") + "\n"`, exit 0; else stderr `Session "<name>" has <vanished|exited> with no saved output.`, exit **0** (cli.ts:1994-2004). (`lastLines` = last 200 lines captured at exit, sessions.ts:234, server.ts:1295-1308.)
- Running: `peek()` (client.ts:76-194): sends `PEEK` with bit0=plain, bit1=full (protocol.ts:87-92). One-shot: on `SCREEN` writes the payload, then (if not `--plain`) `TERMINAL_SANITIZE + "\x1b[999;1H"`, then `"\n"`, destroys the socket, exits 0. `--follow`: keeps streaming `DATA` (stripped of ANSI when `--plain`, via `stripAnsi`), raw-mode stdin if TTY, Ctrl+\ detaches (`[detached]`), session exit prints `\r\n[<name> exited with code N]\r\n` and exits N. Errors: `Session "<name>" not found or not running.` / `Connection error: <msg>` exit 1; close before any screen (one-shot) -> not-found message exit 1.

`cmdPeekWait`: loop every 200 ms: `peekScreen({plain:true})`; if any pattern is a substring -> print the plain screen (or, if not `--plain`, a freshly fetched ANSI screen) + `"\n"`, exit 0. If the connection fails and metadata has `exitedAt` and `lastLines`: match against `lastLines.join("\n")` -> print + `"\n"` exit 0; else stderr `Session "<name>" exited (code <exitCode|?>) without matching "<p>".` then `Last output:` and `  <line>` per line (if any), exit 1. Timeout (checked at loop top, `Date.now() - start > timeoutMs`) -> stderr `Timed out after <sec>s waiting for "<p>".` exit 1; multiple patterns render as `"a" or "b"`. Transient connection errors without exit metadata keep polling.

### 2.5 `pty send` (cli.ts:1109-1217, client.ts:221-288)

Syntax: `pty send [--remote <peer>] <ref> "<text>"` | `pty send <ref> [--with-delay <sec>] [--paste] --seq <chunk> [--seq key:<name>]...`.

Parsing order (cli.ts:1112-1199):
1. `--remote <peer>` is pulled from **anywhere** after `send`; missing value -> `pty send --remote requires a <peer>.` exit 1.
2. First remaining token = ref; missing -> `Usage: pty send [--remote <peer>] <name> "text"  or  pty send <name> --seq "text" --seq key:return` exit 1.
3. `--paste` is removed from anywhere after the ref.
4. `--with-delay <sec>` is recognized **only as the first token after the ref** (post `--paste` removal); `parseFloat`; NaN or negative -> `--with-delay requires a non-negative number (seconds).` exit 1. Elsewhere it is an unexpected argument.
5. `hasSeq = includes("--seq")`; `hasPositional = first token exists and does not start with "--"` (a single-dash token like `-x` is positional text). Both -> `Cannot mix positional text with --seq flags.` exit 1.
6. Any of `--enter`, `--newline`, `--return`, `--cr` -> `Unknown flag "<a>". Use `--seq "<text>" --seq key:return` to send text followed by Enter.` exit 1.
7. `--seq` mode: every token must be `--seq <value>`; missing value -> `--seq requires a value.` exit 1; other token -> `Unexpected argument: <tok>` exit 1. Values go through `parseSeqValue` (keys.ts:166-171): `key:<spec>` -> `resolveKey(spec)` (section 5.1; errors thrown -> stderr message, exit 1), otherwise the literal string.
8. Positional mode: exactly one token allowed; extra -> `Unexpected argument: <tok2>` exit 1. Data = `[text]`.
9. Nothing -> `Nothing to send.` exit 1.
10. Delay: `resolveSeqDelayMs` (client.ts:226-228): absent -> **300 ms**; explicit -> `Math.round(sec*1000)` (so `--with-delay 0` = no gap).
11. Local: `resolveRef` then `send()`; remote: dial (`pty send --remote <peer>: <msg>` exit 1) then `send()` over the tunnel.

`send()` (client.ts:231-288): after connect, if `--paste` and data non-empty writes `DATA("\x1b[200~")`; then each item as its own `DATA` packet, sleeping `delayMs` **between** items (before item i>0 only when delayMs is non-zero); then `DATA("\x1b[201~")` if paste; `socket.end()`; exit 0 on `finish`. No implicit newline is ever added. Errors -> `Session "<name>" not found or not running.` / `Remote session ...` / `Connection error: <msg>` exit 1; a close before `finish` -> not-found message exit 1. Silent on success.

### 2.6 `pty events` (cli.ts:1219-1248, `cmdEvents` 3965-4051)

Syntax: `pty events [--all] [--recent] [--json] [--wait <type>] [-t|--timeout <sec>] [<ref>]`.

Parsing (cli.ts:1226-1234): leading dash-tokens consumed (`--all`, `--recent`, `--json`, `--wait <type>`, `-t/--timeout <sec>` via `parseFloat`); an unknown dash-token ends the loop and becomes the ref. `!all && !ref` -> `Usage: pty events [--all] [--recent] [--json] [--wait <type>] [-t <seconds>] [<name>]` exit 1. A ref is resolved with `resolveRef` (even with `--all`).

- `--recent`: requires a ref (`--recent requires a session name.` exit 1, even with `--all`); reads the last 50 lines of `<name>.events.jsonl` (`readRecentEvents`, events.ts:415-423; unreadable file -> empty); none -> stdout `No recent events for "<name>".` exit 0; else one line per event: `JSON.stringify(event)` with `--json`, otherwise `formatEvent(event)` (section 5.6). Exit 0.
- Follow with a ref: re-checks existence (`Session "<name>" not found.` exit 1).
- `--wait <type>`: requires a ref (`--wait requires a session name.` exit 1); watches the file from its current EOF; the first event whose `type === <type>` is printed (json/formatted) and the process exits 0; `-t > 0` -> after the timeout stderr `Timed out after <t>s waiting for "<type>" event.` exit 1; SIGINT -> exit 0.
- Follow (no `--wait`): `EventFollower` on the named session (from EOF) or, with `--all`, on every existing `*.events.jsonl` (from EOF) plus newly created files (from offset 0 so `session_start` is included) via a directory watch (events.ts:430-546). Prints every event; runs until SIGINT (exit 0); keep-alive timer every 60 s. A truncated file (size shrinks) restarts from offset 0.

### 2.7 `pty list` / `pty ls` (cli.ts:1250-1316, `cmdList` 2165-2446)

Syntax: `pty list [--json] [--tags] [--filter-tag k=v]... [--remote [<peer>]] [--status running|exited|vanished] [--older-than <dur>] [--newer-than <dur>] [--summary]`.

Parsing: `extractFilterTags` first (bad -> `--filter-tag expects "key=value"` exit 1). Then one pass over the rest: `--status <v>` must be exactly `running`/`exited`/`vanished` (missing value also fails) -> `--status expects one of: running, exited, vanished` exit 1; `--older-than`/`--newer-than <dur>` via `parseDuration` (section 5.2) -> `<flag> expects a duration like 30s, 5m, 2h, 1d` exit 1; `--remote` sets `remote=true` and takes the next token as the peer only if it exists and does not start with `-`. Remaining tokens: `--json`, `--tags`, `--summary` by presence; **any other token is silently ignored**. Flags may appear in any order and repeat (last wins for status/durations).

Filtering (cli.ts:2178-2198): tags AND-match (`matchesAllTags`, tags.ts:38-46); status equality; age filter anchored on `exitedAt ?? createdAt` (`ageMs = now - anchor`); `--older-than X` keeps `ageMs >= X`; `--newer-than X` keeps `ageMs <= X`; sessions with no metadata timestamps are excluded when either age flag is set. Sort: by `displayName ?? name` ascending using JS string comparison (cli.ts:2204-2208).

Remote (cli.ts:2223-2247): `--remote <peer>` -> `execFileSync(FABRIC_BIN, ["dial", peer, "pty-remote"], {timeout: 10000})` -> socket path -> `fetchRemoteList` (`{op:"list"}` line; 10 s); any failure becomes `error` for that host group (`fabric dial <peer> returned no socket`, `remote list timed out`, `bad remote response: <msg>`, or the peer's `error`). Bare `--remote` -> `which pty-relay` then `pty-relay ls --json` (5 s); if status 0 and non-empty stdout it is parsed as the host array `{label, sessions:[...], error}`; all failures silently ignored.

`--json` (cli.ts:2287-2312):
- Array of objects, each (key order as emitted): `name: string`, `status: "running"|"exited"|"vanished"`, `pid: number|null`, `command: string|null` (= `metadata.displayCommand`), `cwd: string|null`, `createdAt: string|null` (ISO), `exitCode: number|null`, `exitedAt: string|null`, `tags: {k:v}` **only if metadata has `tags`** (even `{}`), `displayName: string` **only if truthy**.
- With `--remote` (bare or peer) and at least one host group: `{"local": [...], "remote": [{"label": string, "sessions": [{name, status, command?, cwd?, tags?, displayName?}], "error": string|null}]}`.
- `--json --summary`: `{"total": n, "byStatus": {"running": n, "exited": n, "vanished": n}, "oldest": {"name","status","ageSeconds","displayName"?}|null, "newest": {...}|null}` — oldest/newest anchored on `createdAt` only; `ageSeconds = max(0, floor((now-ts)/1000))` (cli.ts:2252-2285).
- Single line via `JSON.stringify` (no pretty print).

Text mode (cli.ts:2314-2445):
- `--summary`: `No matching sessions.` when total 0; else `<N> session|sessions — <parts>` where parts are `<n> running`, `<n> exited`, `<n> vanished` joined by `, ` (only non-zero, in that order); then `oldest: <label> (<status>, <formatDuration(age)>)` and `newest: ...` (newest only if its name differs from oldest). `label` = `<displayName> (<name>)` or `<name>`.
- Empty (no sessions and no remote hosts): `No active sessions.`
- Sections in order, separated by a blank line only when a previous section printed: 
  - `Active sessions:` then per running session: `  <label>[marker][tags] (pid: <pid>) — <cwd> — \x1b[2m<cmd>\x1b[0m` with `label = "\x1b[1;36m<dn>\x1b[0m \x1b[2m(<name>)\x1b[0m"` or `"\x1b[1;36m<name>\x1b[0m"`; `cmd = displayCommand` or `unknown` when no metadata; `cwd = shortPath(cwd)` or empty.
  - `Exited sessions:` then `  <label(bold \x1b[1m)>[marker][tags] (exited with code <exitCode|?>, <timeAgo(exitedAt)|unknown>) — <cwd> — \x1b[2m<cmd>\x1b[0m`.
  - `\x1b[33mVanished sessions (no exit record — killed or crashed):\x1b[0m` then `  ⚠ <label(bold-yellow \x1b[1;33m)>[marker][tags] (vanished, started <timeAgo(createdAt)|unknown>) — <cwd> — \x1b[2m<cmd>\x1b[0m`.
  - Remote host groups: blank line, then `\x1b[1m<label>\x1b[0m \x1b[31m(error: <error>)\x1b[0m` or `\x1b[1m<label>\x1b[0m (<n> sessions):` followed by `  ●|○ <label(cyan)>[marker][tags] — <cwd> — \x1b[2m<cmd>\x1b[0m` (● when `status === "running"`), sorted by displayName ?? name.
- `[tags]` = `" " + entries.map(k=v => "#k=v").join(" ")` for non-reserved keys (reserved = `ptyfile`, `ptyfile.session`, `ptyfile.tags`, `strategy`, any key starting with `:`; tags.ts:56-79) — `--tags` shows all. Note `strategy.status`, `strategy.fast-fail-*` etc. are **not** reserved (only exact `strategy`), so they show by default.
- `[marker]` = ` \x1b[31m[flapping]\x1b[0m` when `strategy.status=flapping`, else ` \x1b[33m[permanent]\x1b[0m` when `strategy=permanent`, else empty (cli.ts:4102-4112).
- `shortPath` (cli.ts:4114-4119): `home` -> `~`, `home/...` -> `~/...`. `timeAgo` (cli.ts:4121-4130): `<n>s ago` (<60 s), `<n>m ago` (<60 m), `<n>h ago` (<24 h), `<n>d ago`.

### 2.8 `pty remote-serve` (cli.ts:1318-1343, `cmdRemoteServe` 2118-2163, remote.ts)

Syntax: `pty remote-serve --stdio` | `pty remote-serve --socket <path>`.
- `--stdio` anywhere in args -> `runRemoteServeStdio()` (remote.ts:193-196): serve exactly one control interaction over stdin/stdout, then `process.exit(0)`.
- Otherwise `--socket <path>` (value = next token); missing -> stderr:
  ```
  Usage: pty remote-serve (--stdio | --socket <path>)
  Serve the remote-access control protocol for a fabric peer to expose.
    --stdio          on-demand, spawned by fabric per dial (recommended):
                     fabric expose pty-remote --exec -- pty remote-serve --stdio
    --socket <path>  listening daemon (fabric expose pty-remote --socket <path>)
  Run in the same PTY_ROOT env as the sessions; put --socket OUTSIDE PTY_ROOT.
  ```
  exit 1 (cli.ts:1331-1339).
- Listening form: unlinks a stale socket, listens; stdout `pty remote-serve listening on <path>` (cli.ts:2130); listen error -> stderr `pty remote-serve: <msg>` exit 1; SIGHUP ignored; SIGTERM/SIGINT -> close + unlink + exit 0. `PTY_REMOTE_SERVE_DEBUG` adds `[remote-serve <iso>] ...` stderr lines (up/beforeExit/exit/uncaughtException/unhandledRejection/SIGHUP/shutdown).
- Control protocol (remote.ts:81-165): one JSON request line (`\n`-terminated). `{"op":"list"}` -> one line `{"sessions":[{name,status,command?,cwd?,tags?,displayName?}]}` (fields present only when set; `command` = `displayCommand`) then done. `{"op":"route","name":"<ref>"}` -> resolves `<ref>` with `getSession` (ambiguity error text becomes `{"error":"<msg>"}`), unknown -> `{"error":"session \"<ref>\" not found"}`; on success writes `{"ok":true}\n` then splices bytes bidirectionally to `<name>.sock`; a target connect error yields `{"error":"session \"<ref>\" not found"}`. Malformed JSON -> `{"error":"malformed request"}`; other ops -> `{"error":"unknown op: <op>"}`. Bytes after the request line are forwarded as the first frame.

### 2.9 `pty stats` (cli.ts:1345-1356, `cmdStats` 2448-2564, `printStats` 2566-2595)

Syntax: `pty stats [--json] [--all] [<ref>]`. Flags anywhere; the first non-flag token is the ref; additional tokens ignored.

- With a ref: `getSession` (ambiguity -> thrown message); `null` -> stderr `Session "<ref>" not found.` exit 1. Gone session: `--json` -> `{"name":<id>,"status":"exited"|"vanished","exitCode":n|null,"exitedAt":iso|null,"tags":{...}?}` exit 0; text -> `Session "<ref>" has vanished (no exit record — killed or crashed).` or `Session "<ref>" has exited (code <exitCode|?>).` (note: prints the ref as typed) exit 0. Running: `queryStats(stableId)` (STATUS packet, 2 s timeout -> `Timeout querying stats for "<id>"`; ENOENT/ECONNREFUSED -> `Session "<id>" not found or not running.`; else `Connection error: <msg>`; invalid JSON -> `Invalid stats response from "<id>"`) -> failure printed to stderr, exit 1. `--json` prints the daemon's `StatsResult` JSON verbatim on one line; text prints `printStats`.
- Without a ref: all sessions; if no running sessions and (`!--all` or no gone sessions) -> stdout `No running sessions.` exit 0. Stats queried in parallel. `--json` -> array of `StatsResult` objects (or `{"name":<id>,"error":<msg>}` per failed query) followed, with `--all`, by gone entries `{"name","status","exitCode","exitedAt","tags"?}`. Text: `printStats` blocks separated by blank lines; failed query -> `Session: <id>` / `  Error: <msg>`; with `--all` and gone sessions: blank line, `Exited sessions:` / `  <id> (exited with code <c|?>, <ago|unknown>)`, blank line, `Vanished sessions (no exit record):` / `  ⚠ <id> (started <ago|unknown>)`.
- `printStats` text (cli.ts:2566-2595), exact lines:
  ```
  Session: <name>
    Command:    <displayCommand|unknown>
    CWD:        <shortPath(cwd)|unknown>
    Uptime:     <formatUptime(uptimeSeconds)>
    Process:    running (pid <pid>)         | exited (code <exitCode>)[ (pid N)]
    CPU:        <cpuPercent.toFixed(1)>%    (only if process.resources)
    Memory:     <formatMemory(rssKb)>       (only if process.resources)
    Daemon:     pid <pid>[, <formatMemory(rssKb)>]   (only if daemon)
    Terminal:   <cols>x<rows>
    Cursor:     row <cursorY>, col <cursorX>
    Scrollback: <scrollbackUsed> / <scrollbackCapacity> lines
    Clients:    <total> (<attached> attached, <readOnly> readonly)
    Modes:      SGR mouse, cursor hidden, kitty keyboard (flags: a,b)  | none
  ```
  `formatMemory`: `<n> KB` (<1024), `<x.x> MB` (<1024 MB), `<x.xx> GB`. `formatUptime`: `unknown` for null, `<s>s`, `<m>m <s>s`, `<h>h <m>m`, `<d>d <h>h`.
- `StatsResult` JSON (client.ts:295-341; docs/client.md:342-386): `{ name, terminal:{cols,rows,cursorX,cursorY,scrollbackUsed,scrollbackCapacity}, process:{alive,exitCode|null,pid|null,resources:{rssKb,cpuPercent}|null}, daemon:{pid,resources|null}, clients:{total,attached,readOnly,connections?:[{role:"writable",rows,cols,lastRequestSequence,constrains:{rows,cols}}|{role:"readonly",constrains:{rows:false,cols:false}}]}, modes:{sgrMouse,cursorHidden,kittyKeyboard,kittyKeyboardFlags:number[]}, uptimeSeconds|null, createdAt|null }`. `scrollbackCapacity = rows + 10000` (server.ts:1129). Resources come from `ps -o rss=,pcpu= -p <pid>` (server.ts:217-232).

### 2.10 `pty restart` (cli.ts:1358-1382, `cmdRestart` 3886-3963)

Syntax: `pty restart [-y|--yes] [--force] <ref>`. Flags anywhere; first other token = ref; a second -> `pty restart: unexpected argument "<tok>"` exit 1; none -> `Usage: pty restart [-y] [--force] <name>` exit 1. Ref via `resolveRef`.

Steps: not found -> `Session "<name>" not found.` exit 1; no metadata -> stderr `Session "<name>" has no metadata — cannot restart.`, `cleanupAll`, exit 1. **Stateful-agent guard** (cli.ts:3850-3857, 3911-3919): if `tags.role === "agent"` (reason `role=agent tag`) or the joined `command args displayCommand` string matches `/(^|\s|\/)claude(\s|$)/` and `/(^|\s)--resume(\s|=|$)/` (reason `claude --resume command`), and no `--force`: stderr
```
Session "<name>" looks like a stateful agent (<reason>).
`pty restart` kills its in-progress work and can wedge a `claude --resume`. Cycle it through its supervisor (e.g. `convoy up`) instead — or pass --force to restart anyway.
```
exit 1. If running: prompt `Session "<name>" is running. Kill and restart? [Y/n] ` unless `-y`; answer `n` -> exit 0; then SIGTERM the daemon (errors ignored), `cleanupSocket`, sleep 200 ms. Then `cleanupAll(name)`, tags = stored tags minus `strategy.status`, `strategy.consecutive-fast-fails`, `strategy.last-respawn-at`, `strategy.command-hash` (cli.ts:4087-4100), `spawnDaemon` with stored `command/args/displayCommand/cwd/displayName` + persisted launch options + `scrubEnv: ["ST_AGENT","ST_ROOT"]` (spawn errors -> thrown, exit 1). stdout `Session "<name>" restarted.`. If `PTY_SESSION` is set and no `--force`: stdout `  (not attached: already inside pty session "<s>". Pass --force to attach anyway.)` and return (exit 0). Otherwise attach (exit 0 on detach / session code). Note `createdAt` is rewritten by the new daemon; the events file is recreated (a restart starts a fresh log).

### 2.11 `pty kill` (cli.ts:1384-1392, `cmdKill` 2618-2671)

Syntax: `pty kill <ref>`. `args.length < 2` -> `Usage: pty kill <name>` exit 1. Ref via `resolveRef`; extra args ignored.
- Not `running` or no pid -> stderr `Session "<name>" is not running. Use "pty rm <name>" to remove it.` exit 1.
- If `tags.strategy === "permanent"`, the `strategy` tag is removed first so `gc` will not respawn (cli.ts:2633-2638).
- `process.kill(pid, "SIGTERM")`; failure -> stderr `Failed to kill session "<name>".`, exit code 1 (via `exitCode`).
- Waits up to **7000 ms** for the daemon pid to exit (`waitForProcessExit`); timeout -> stderr `Failed to kill session "<name>": daemon PID <pid> is still running after 7s. Socket <root>/<name>.sock may still be owned.` exit code 1.
- `cleanupSocket(name)` (removes `.sock` + `.pid`), stdout `Session "<name>" killed.` exit 0. Metadata (`.json`, events) is **kept** (daemon treats SIGTERM as external kill; only `--ephemeral` reaps). If it was permanent and `tags.ptyfile` is set: stderr `Note: this session is managed by <toml>` and `The strategy tag will be restored on the next 'pty up'.`
- The daemon's external-kill shutdown terminates the child's descendant tree (`server.close({ terminateDescendants: true })`, server.ts:1559).

### 2.12 `pty recover` (cli.ts:1394-1409, `cmdRecover` 2673-2807)

Syntax: `pty recover <name> --snapshot <metadata.json>`. `name = args[1]`; `--snapshot` value = next token; either missing -> `Usage: pty recover <name> --snapshot <metadata.json>` exit 1. Errors are wrapped: stderr `pty recover: <message>` exit 1. Success: stdout `Session "<name>" registry recovered without restart.` exit 0.

Error messages (thrown inside `cmdRecover`/recovery.ts): `validateName` messages; `recovery file must be a bounded regular file` (symlink/non-regular/>1 MiB; recovery.ts:265-271); JSON parse errors; `snapshot does not advertise supported recovery` (protocol !== 1, missing secret/metadataRevision/generation/daemonPid); `PTY_ROOT must be an owned private non-symlink directory` / `PTY_ROOT recovery directory must be an owned private non-symlink directory` (mode & 0o077 must be 0, owned by uid; recovery.ts:100-118); `recovery root identity changed`; `daemon PID/start identity no longer matches the snapshot` (Linux: `/proc/<pid>/stat` starttime as `linux:<n>`; macOS: `ps -o lstart=` as `darwin:<s>`); `session "<name>" is being created by another process` (recovery lock); `recovery target is no longer empty`; `republished socket reached a different daemon`; `supporting daemon did not answer recovery request` (7 s deadline, request re-published every 250 ms, polled every 25 ms); `daemon recovery response authentication failed`; `daemon refused recovery` or the daemon's `error`; `daemon recovery response changed identity`; `republished metadata changed identity`. Files: `<root>/.recovery/<name>.request.json`, `<name>.result.json` (both removed in `finally`), lock `<root>/<name>.lock` containing `<daemonPid>\nrecovery:<identity>\n` (recovery.ts:261-263). Idempotent when the registry already exists with matching identity (prints the success line without a request).

### 2.13 `pty rm` / `pty remove` (cli.ts:1604-1613, `cmdRm` 3036-3087)

Syntax: `pty rm <ref>`. `args.length < 2` -> `Usage: pty rm <name>` exit 1. Ref via `resolveRef`.
- Not found -> `Session "<name>" not found.` exit 1. `running` -> stderr `Session "<name>" is still running. Use "pty kill <name>" first.` exit 1.
- Waits up to 7000 ms for the recorded daemon pid (`session.pid ?? metadata.daemonPid ?? <name>.pid`) to exit; timeout -> stderr `Session "<name>" daemon did not exit within 7s; not removed. Try again.` exit 1.
- `cleanupOwnedAll(name, { generation: metadata.generation ?? "", pid: daemonPid ?? -1 })` under the creation lock; if the generation changed meanwhile -> stderr `Session "<name>" was replaced while waiting; new generation was not removed.` exit 1.
- stdout `Session "<name>" removed.` exit 0. Removes `.sock`, `.pid`, `.json`, `.events.jsonl` (and recovery revision). Works on `keep`-tagged sessions (explicit removal beats keep).

### 2.14 `pty gc` (cli.ts:1411-1453, `cmdGc` 3089-3202, `printLaunchdPlist` 3224-3276, sessions.ts `gc` 1521-1724)

Syntax: `pty gc [-n|--dry-run] [--idle-days N | --idle-days=N] [--fast-fail-window N|=N] [--fast-fail-limit N|=N]` | `pty gc --print-launchd-plist [--interval N | --interval=N]`.
Parsing (cli.ts:1412-1446): `--dry-run`/`-n` by presence; `--print-launchd-plist` by presence; numeric flags accept both `--flag N` and `--flag=N`; `parsePositive`: `parseInt(raw,10)` must be finite and `> 0`, else stderr `pty gc: <flag> expects a positive integer (got "<raw>")` exit 1. Unknown tokens ignored. `--print-launchd-plist` short-circuits before any gc work.

Pass (sessions.ts:1521-1724), all sub-results printed in this order (cli.ts:3105-3150):
1. Raw-debris inventory (`.sock`/`.pid` with dead pid and missing/corrupt `.json`, not locked, socket unreachable) -> counted in `removed`.
2. **Orphan kill** (step 1): sessions with a `parent=<id>` tag, processed in name order; parent considered alive only if its metadata exists and its pid is alive; else SIGTERM + cleanup (`reapObservedSession`) -> `Killed orphan child: <name> (parent <parent> <missing|dead>)`. Lock contention etc. -> `Skipped orphan reap: <name> (<busy|stale|signal-failed|shutdown-timeout>, <before|after> signalling)`.
3. **Abandoned reap** (step 1.5): live `strategy=permanent` sessions whose `cwd` no longer exists (unless tag `strategy.abandon-if-cwd-gone=false`) -> `Abandoned: <name> (cwd-gone)`; or, when an idle threshold exists (`strategy.idle-days=N` tag, else `--idle-days N`) and `lastAttachAt` is older than N days -> `Abandoned: <name> (idle <ageDays>d)` (cwd-gone wins). Emits `session_abandoned` event before cleanup. Skips -> `Skipped abandoned reap: ...`.
4. **Permanent respawn** (step 2): exited/vanished `strategy=permanent` sessions. Flapping classifier (sessions.ts:1803-1895): effective window = tag `strategy.fast-fail-window` > `--fast-fail-window` > 60; limit = tag `strategy.fast-fail-limit` > `--fast-fail-limit` > 3; `strategy.command-hash` (16 hex chars of sha256 of `command\0args...`) change resets the counter and clears a flapping mark; already `strategy.status=flapping` -> `Skipped (flapping): <name> — remove strategy.status tag to retry`; a fast fail = `exitedAt - strategy.last-respawn-at < window`; counter reaching the limit -> tags `strategy.status=flapping`, `strategy.consecutive-fast-fails=<n>`, `strategy.command-hash` (+ keep `last-respawn-at`), `session_flapping` event `{counter,limit,window}`, line `Flapping: <name> (<n> fast-fails in <window>s, limit <limit>)`; otherwise respawn with tags `strategy.last-respawn-at=<iso>`, `strategy.consecutive-fast-fails=<n>`, `strategy.command-hash` -> `Respawned: <name>[ (pty.toml re-read)]` (suffix when the session has a `ptyfile` tag; the manifest is re-read for command/cwd/env/tags, falling back to stored metadata on read errors; `session_respawn` event appended) or `Respawn failed: <name> — <error>`.
5. **Sweep** (step 3): exited/vanished non-permanent sessions -> removed (`Removed: <name>`) unless `keep` (`Kept (keep tag): <name> — remove the keep tag to reap it`).
6. `pruneOrphanLayoutTags`: on running sessions, tag keys matching `/^:l(\d+)-[a-z0-9]+$/` whose pid is dead (or non-positive) are removed -> `Pruned orphan tags on <name>: #<key> #<key2>`.
Dry-run variants: `Would kill orphan child`, `Would abandon`, `Would respawn`, `Would flap`, `Would remove`, `Would prune`. Footer: no actions -> `Nothing to clean up.` / `Nothing would be cleaned up.`; else `Cleaned up <parts>.` / `Would clean up <parts>. (Dry run — no changes made.)` where parts (comma-joined, in order, only non-zero): `<n> orphan child|children`, `<n> abandoned`, `<n> reap skip|skips`, `<n> respawn|respawns`, `<n> respawn failure|failures`, `<n> flapping`, `<n> skipped-flapping`, `<n> stale session|sessions`, `<n> orphan tag|tags`. Exit 0 always (errors inside respawn are reported, not fatal).

`--print-launchd-plist` (cli.ts:3224-3276): writes to stdout (raw) an XML plist: Label `com.compoundingtech.pty.gc` when root == default dir, else `com.compoundingtech.pty.gc.<basename(root) with [^A-Za-z0-9._-]+ -> "-" and edge dashes stripped>`; `ProgramArguments` = `[process.execPath, process.argv[1], "gc"]` (i.e. the invoked launcher script path, e.g. `.../bin/pty`); `StartInterval` = interval (default 30); `RunAtLoad` true; `StandardOutPath`/`StandardErrorPath` = `<root>/gc.log`; `EnvironmentVariables` = `PATH` (from env or `/usr/bin:/bin:/usr/sbin:/sbin`) and `PTY_ROOT` = the resolved root. Values are XML-escaped (`& < >`). Exact template at cli.ts:3245-3274.

### 2.15 `pty tag` (cli.ts:1455-1539)

Syntax: `pty tag <ref>` (show) | `pty tag <ref> key=value... [--rm key]...` (write; any order).
- No ref -> `Usage: pty tag <name> [key=value...] [--rm key...]` exit 1. Ref via `resolveRef`.
- Parsing after the ref: `--rm` needs a next token (`pty tag: --rm requires a key (e.g. --rm role)` exit 1), empty -> `pty tag: --rm requires a non-empty key` exit 1; other tokens must contain `=` (`pty tag: invalid argument "<tok>". Use key=value or --rm key.` exit 1) with non-empty key (`pty tag: empty key in "<tok>". Tag keys must be non-empty.` exit 1); split on the first `=` (`foo=bar=baz` -> `foo` = `bar=baz`); duplicates: last wins; parse errors abort before any write.
- Show (no ops): metadata missing -> `Session "<ref>" not found.` exit 1; no tags -> stdout `No tags on "<name>".`; else one line per tag `  k=v` (insertion order). Exit 0.
- Write: `updateTags(name, updates, removals)` — updates applied, then removals (so `k=v --rm k` removes `k`); a no-op emits no `tags_change` event; an empty resulting map deletes the `tags` field. Output: `Tags cleared on "<name>".` or `Tags on "<name>":` followed by `  k=v` lines. If the session had a `ptyfile` tag before the write: stderr `\nWarning: this session is managed by <toml>` / `Running 'pty up' will sync tags from the toml and may overwrite this change.` / `To make it permanent, edit the pty.toml file directly.`. Errors from `updateTags` (e.g. busy lock: `Session id "<name>" event log is busy. Retry the operation.`) -> stderr, exit 1. Works on exited sessions.

### 2.16 `pty tag-multi` (cli.ts:1541-1544, 3300-3511)

Syntax: `pty tag-multi <selector> [--json] [-y|--yes] [ops...]`; selectors: `<ref>...` | `--filter-tag k=v` (repeatable, AND) | `--all`; ops: `k=v` | `--rm k`.
Parsing (cli.ts:3300-3389): `--all`, `--json`, `--yes`/`-y`; `-h`/`--help` prints the dedicated help (section 3.3) to stdout, exit 0; `--filter-tag` needs a value (`pty tag-multi: --filter-tag requires k=v`), containing `=` (`pty tag-multi: --filter-tag value "<v>" must be k=v`), non-empty key (`pty tag-multi: --filter-tag key must be non-empty`); `--rm` needs a value (`pty tag-multi: --rm requires a key (e.g. --rm role)`), non-empty (`pty tag-multi: --rm requires a non-empty key`); a token containing `=` is an update (empty key -> `pty tag-multi: empty key in "<tok>". Tag keys must be non-empty.`); anything else is a session ref. All exit 1. Selector count 0 -> `pty tag-multi: no selector — pass session names, --filter-tag k=v, or --all` exit 1; >1 -> `pty tag-multi: selectors are mutually exclusive — pick one of <names>, --filter-tag, --all` exit 1.
- Targets: names resolved up-front via `getSession` (`pty tag-multi: session "<ref>" not found.` exit 1 before any write); filter -> all sessions matching all tags; `--all` -> every session, but a write without `--yes` -> stderr `pty tag-multi: --all writes are destructive across <n> session(s). Re-run with --yes to apply.` exit 1.
- Read mode (no ops): `--json` -> `{"<id>": {tags}, ...}` (stable ids as keys; `{}` for untagged); text -> `0 sessions matched.` when empty, else per session `<id>: (no tags)` or `<id>:` + `  k=v` lines. Exit 0.
- Write mode: zero targets -> `{}` (json) or `0 sessions matched. No writes performed.`; else apply per session (each a separate atomic write/event), `--json` -> `{"<id>": {resulting tags}}`, text -> `<n> session(s) processed.`; per-session errors -> stderr `pty tag-multi: <id>: <msg>` and exit 1 after processing all.

### 2.17 `pty emit` (cli.ts:1546-1549, 3515-3579)

Syntax: `pty emit <type> [--json <payload>] [--text <string>]` | `pty emit <ref> <type> [--json ...] [--text ...]`.
- `-h`/`--help` anywhere -> prints the emit help to stdout, exit 0. `--json`/`--text` take the next token (only when one exists). Exactly 1 positional -> type, ref defaults to `$PTY_SESSION`; exactly 2 -> ref + type; otherwise the help is printed to **stdout** and exit 1 (cli.ts:3536-3541).
- No ref and no `PTY_SESSION` -> stderr `pty emit: no session ref given and not running inside a pty session` / `  tip: run inside a pty session, or: pty emit <session-ref> <type>` exit 1. Ref via `resolveRef`.
- `--json` must parse: `pty emit: --json payload is not valid JSON: <msg>` exit 1 (checked after ref resolution).
- Type validation (events.ts:201-217): empty -> `event type must be a non-empty string`; not starting with `user.` -> `custom events must start with "user." (got "<type>")`; exactly `user.` -> `event type "user." needs a suffix (e.g. "user.build-done")`; whitespace/control chars -> `event type may not contain whitespace or control characters`; printed to stderr, exit 1.
- Appends `{session, type, ts, data?, text?}` to `<id>.events.jsonl` (with retention: files >= 1000 lines are truncated to the last 500). Silent on success, exit 0.

### 2.18 `pty rename` (cli.ts:1589-1592, 2926-3034)

Syntax: `pty rename <new-display-name>` (inside a session) | `pty rename <ref> <new-display-name>` | `pty rename --show <ref>` | `pty rename --clear [<ref>]`.
Parsing: `--show`, `--clear`, `-h`/`--help` (-> help to stdout, exit 0; also handled by the central interceptor) anywhere; other tokens positional.
- `--show`: exactly one positional else stderr `pty rename --show requires exactly one ref.` + help (stderr) exit 1; `getSession` (ambiguity thrown); not found -> `Session "<ref>" not found.` exit 1; stdout `<displayName>` or `(no displayName; session is referenced by its id: <id>)`. Exit 0.
- `--clear`: 0 positionals requires `PTY_SESSION` (else `pty rename --clear with no ref requires being inside a pty session (PTY_SESSION not set).` + help, exit 1) and targets `$PTY_SESSION` (the id, no lookup); 1 positional -> `getSession`; >1 -> `pty rename --clear takes at most one ref.` + help, exit 1. `setDisplayName(id, null)`; stdout `Cleared displayName on "<id>".`; errors -> message exit 1.
- Set: 1 positional requires `PTY_SESSION` (else stderr `pty rename with a single arg is only allowed inside a pty session.` / `Outside, use: pty rename <ref> <new-display-name>` + help, exit 1); 2 positionals -> `getSession(ref)`; other counts -> help (stderr) exit 1. `validateDisplayName` failure -> `Invalid displayName: <msg>` exit 1. `setDisplayName(id, dn)` (`""` is treated as clear) -> stdout `Set displayName on "<id>" → "<dn>".` (with the `→` arrow). A no-op write emits no `display_name_change` event. Display names may duplicate other sessions' names/ids.

### 2.19 `pty metadata patch` (cli.ts:1594-1597, 2815-2873)

Syntax: `pty metadata patch --id <stable-id>` with one JSON object on stdin.
- `metadata patch -h|--help` -> the metadata help, exit 0. First arg not `patch` -> stderr `pty metadata: expected subcommand "patch".` / `  Usage: pty metadata patch --id <stable-id>` exit 1.
- `--id` requires a value (`pty metadata patch: --id requires a stable session id.`), only once (`pty metadata patch: --id may only be provided once.`); any other token -> `pty metadata patch: unexpected argument "<tok>".` + usage line; missing -> `pty metadata patch: missing required --id <stable-id>.`; all exit 1.
- stdin read fully (fd 0) and trimmed; empty -> `pty metadata patch: expected one JSON patch object on stdin.` / `  Example: printf '%s' '{"displayName":"Worker"}' | pty metadata patch --id a1b2c3d4` exit 1; parse error -> `pty metadata patch: invalid JSON on stdin: <msg>` exit 1.
- `patchMetadataById` (sessions.ts:559-567, 400-434): validation errors (`Metadata patch must be a JSON object.`, `Metadata patch has unknown field "<k>". Allowed fields: displayName, tags.`, `Metadata patch displayName must be a string or null.`, `Invalid displayName: <msg>`, `Metadata patch tags must be a JSON object.`, `Metadata patch tag keys must be non-empty.`, `Metadata patch tag values must be strings or null (invalid key: "<k>").`) run **before** the id lookup; then exact-id lookup only: `Session id "<id>" not found.`; lock busy: `Session id "<id>" event log is busy. Retry the operation.`. All -> stderr `pty metadata patch: <msg>` exit 1.
- Success: stdout one line `{"changed":true|false,"metadata":{...full SessionMetadata...}}`; a real change appends one `metadata_change` event whose `previous`/`value` contain only the changed `displayName` and tag keys (absent = `null`). Exit 0.

### 2.20 `pty evidence snapshot|remove` (cli.ts:1599-1602, 2875-2924)

Syntax: `pty evidence snapshot --id <stable-id>` | `pty evidence remove --id <stable-id> --expected-generation <opaque>`.
- First arg not `snapshot`/`remove` -> stderr `pty evidence: expected subcommand "snapshot" or "remove".` + two usage lines, exit 1 (this includes bare `pty evidence` and `pty evidence unknown ...`). `pty evidence <op> -h|--help` (exactly two args) -> leaf help to stdout, exit 0 (`pty evidence --help` uses the generic `evidence` entry via the central interceptor).
- Argument errors are **thrown** (stderr = message, exit 1, empty stdout): `pty evidence: --id requires a stable session id.`, `pty evidence: --id may only be provided once.`, `pty evidence: --expected-generation requires an opaque generation.`, `pty evidence: --expected-generation may only be provided once.`, `pty evidence: unexpected argument "<tok>".`, `pty evidence: missing required --id <stable-id>.`, `pty evidence snapshot: --expected-generation is only valid for remove.`, `pty evidence remove: missing required --expected-generation <opaque>.`; `validateName` failures (e.g. `/absolute`, `../traversal`, `nested/path`, `.`, `..`) and I/O errors also exit 1 with empty stdout.
- Semantic outcomes exit 0 with exactly one JSON line on stdout (written with `fs.writeFileSync(1, ...)`):
  - snapshot: `{"_tag":"snapshot","snapshot":{"name","generation","status":"exited"|"vanished","exitCode":n|null,"stream":"combined","tail":{"_tag":"present","lastLines":[...]}|{"_tag":"unavailable"}}}` or `{"_tag":"unavailable","reason":"missing"|"running"|"busy"|"generation-unavailable"|"invalid-metadata"}`.
  - remove: `{"_tag":"removed"}` | `{"_tag":"missing"}` | `{"_tag":"generation-mismatch"}` | `{"_tag":"not-terminal"}` | `{"_tag":"invalid-metadata"}` | `{"_tag":"busy"}`.
  - `invalid-metadata` covers malformed JSON, > 1 MiB files, directories, symlinks, non-string/empty `generation`, non-number `exitCode`, missing `exitedAt` with an `exitCode`, non-string `lastLines` entries, more than 200 `lastLines` (sessions.ts:1119-1240, 234-235). A removal unlinks `.sock`, `.pid`, `.events.jsonl`, the recovery revision, then `.json` last; a failure mid-way propagates (exit 1) and keeps `.json`.

### 2.21 `pty up` (cli.ts:1551-1568, 3594-3774, ptyfile.ts)

Syntax: `pty up [<dir>] [<name>...]`. Positional scan stops at the first token starting with `-`; the first token is treated as `<dir>` only if `<token>/pty.toml` is a file (`hasPtyFile`), otherwise it is a session name.
- `readPtyFile(dir ?? cwd)` errors -> stderr, exit 1: `No pty.toml found in <dir>`, `Invalid pty.toml in <dir>: <msg>`, `No sessions defined in <file>`, `Session "<label>" in <file> is missing a "command" field`, `Invalid session "<label>" in <file>: expected a table`, `"display_name" must be a non-empty string`, `"id" must be a non-empty string`, `"env" must be a table of string values`, `env.<k> must be a string`, `"cwd" must be a non-empty string`.
- Manifest schema (ptyfile.ts:7-133): top-level `prefix` (string); `[sessions.<key>]` with `command` (required string), `display_name` (default `<prefix>-<key>` or `<key>`), `id` (pinned stable id), `cwd` (absolute, or relative to the manifest dir; default = manifest dir), `tags` (values stringified), `env` (string table).
- Name filter: names match `displayName` or `<key>`; unknown -> stderr `Unknown session[s]: a, b` / `Available: key1, key2` exit 1.
- Binding: an existing session is "bound" when its tags have `ptyfile == <dir>/pty.toml` and `ptyfile.session == <key>` (identity is the tag pair, not the name).
- Bound and running: sync tags from the manifest: set `tomlTags = {...tags, ptyfile, "ptyfile.session", "ptyfile.tags": <sorted user keys joined by ",">}` where changed; remove keys listed in the old `ptyfile.tags` but no longer declared; also drop gc bookkeeping `strategy.status`, `strategy.consecutive-fast-fails`, `strategy.last-respawn-at`, `strategy.command-hash`. Output `  ● <label> (already running, updated tags: k=v, k2=v2, -removed)` (only non-`ptyfile*` updates listed, removals as `-key`) or `  ● <label> (already running)`; counted as skipped.
- Bound and gone: `cleanupAll` (error -> `  ✗ <label>: <msg>`, skipped).
- Spawn: id = manifest `id` (validated; in use -> `  ✗ <label>: id "<id>" is already in use.`) or random; `validateDisplayName(displayName)`; `spawnDaemon({ command: "/bin/sh", args: ["-c", <command>], displayCommand: <command>, cwd: sess.cwd ?? dir, tags: tomlTags, displayName, extraEnv: sess.env? })`; success -> `  ● <label> (started)`; failure -> stderr `  ✗ <label>: <msg>`.
- Footer: `All sessions already running.` when nothing started and everything was skipped; `Started <n> session[s].` when n > 0; nothing otherwise. Exit 0 even with per-session failures.

### 2.22 `pty down` (cli.ts:1570-1587, 3776-3845)

Syntax: `pty down [<dir>] [<name>...]` (same positional rules; unknown names are silently ignored). Manifest errors as above.
For each selected declared session bound by the tag pair: strip `strategy` if `permanent`; running -> SIGTERM the daemon (no wait), `  ○ <label> (stopped[, removed from supervision])`, `cleanupSocket`; failure -> stderr `  ✗ <label>: failed to stop`; gone -> `cleanupAll` -> `  ○ <label> (cleaned up)` (error -> `  ✗ <label>: <msg>`). Footer `No sessions to stop.` or `Stopped <n> session[s].`; when anything stopped was toml-managed: stderr `\nNote: strategy tags will be restored on the next 'pty up'.` Exit 0.

### 2.23 `pty test` (cli.ts:1615-1618, 4053-4067)

Runs `<pkg>/node_modules/.bin/vitest` with the given args (default `["run"]`), stdio inherited, exits with its status (`?? 1`). Help entry: `pty test [watch | -t "<pattern>"]`.

### 2.24 `pty completions <shell>` (cli.ts:1620-1624, completions.ts:728-746)

See section 4. `--help`/`-h` -> usage to stdout, exit 0; missing shell -> usage to stderr, exit 2; unknown -> stderr `pty completions: unknown shell: <shell>\n` + usage, exit 2; otherwise the script to stdout, exit 0.

### 2.25 `pty version` / `pty help` — see section 1.3.

---

## 3. HELP TEXT (verbatim)

All help goes to stdout via `console.log` (a trailing newline is appended). Template-literal escapes have been rendered below (`\\` -> `\`, `` \` `` -> `` ` ``). tests/help.test.ts pins: every command in `run attach exec peek send events list stats restart kill recover rm gc tag tag-multi emit rename metadata up down test remote-serve evidence` plus aliases `a ls remove` must, for `pty <cmd> --help`, exit 0, have stdout matching `/^Usage: pty /`, contain at least one line matching `/^ {2}pty /`, and not start with `[` (help.test.ts:13-51); `pty evidence snapshot --help` / `remove --help` must exit 0 with empty stderr, stdout matching `^Usage: pty evidence <leaf> `, containing `--id <stable-id>`, and `--expected-generation <opaque>` only for `remove` (help.test.ts:53-70); `pty send --help` must contain `key:ctrl+c`, `key:ctrl-c`, `key:C-c`, `_ separators` (help.test.ts:74-81); `pty run --help` must contain `--env KEY=VALUE`, match `/environment variable \(repeatable\)/`, contain `--unset-env KEY`, match `/inherited environment variable \(repeatable\)/` (help.test.ts:83-90); every `case "X":` label in cli.ts must be a documented command/alias or one of `interactive i help --help -h version --version -v -V completions` (help.test.ts:92-100); top-level `pty --help` must exit 0 and contain `pty <cmd> ` for every command (help.test.ts:102-112).

### 3.1 Top-level usage (`usage()`, cli.ts:480-603)

```
Usage:
  pty                                     Interactive session manager (fullscreen TUI)
  pty --preselect-new                     Open the TUI with "Create new session..." pre-selected
  pty --filter-tag key=value              Filter the TUI to sessions matching the tag (repeatable);
                                          new sessions inherit the tag

Create sessions:
  pty run -- <command> [args...]          Create a session and attach (random id + auto display label)
  pty run --id <id> -- <command>          Pin the on-disk id (sock / json filename; charset-validated)
  pty run --name <label> -- <command>     Set a trimmed, single-line display label (≤ 160 Unicode scalars)
  pty run --no-display-name -- <cmd>      Skip the friendly cwd+command label (just an id)
  pty run -d -- <command>                 Create in the background (detached)
  pty run -a -- <command>                 Create OR attach if a session with the same id already exists
  pty run -e -- <command>                 Ephemeral: auto-remove metadata on clean exit
  pty run --tag key=value -- <command>    Tag a session (repeatable)
  pty run --env KEY=VALUE -- <command>    Overlay child environment (repeatable; persisted for restart)
  pty run --unset-env KEY -- <command>    Remove inherited environment (repeatable; persisted for restart)
  pty run --cwd /path -- <command>        Run in a specific directory
  pty run --isolate-env -- <command>      Scrub the child env to a safe allow-list
                                          (intended for remote-reachable sessions)
  pty run --force -- <command>            Create even from inside another pty session (nested)

Attach & interact:
  pty attach <ref>                        Attach to an existing session (alias: pty a)
  pty attach --force <ref>                Attach even from inside another pty session (nested)
  pty attach -r <ref>                     Attach, auto-restart if the session is exited
  pty attach --no-restart <ref>            Attach only; fail if the session is not running
  pty attach --remote <peer> <ref>        Attach a session on a fabric peer (over fabric)
  pty exec -- <command> [args...]         Replace the current session's process (inside a session)
  pty send <ref> "text"                   Send raw text (no implicit newline)
  pty send <ref> --seq "text" --seq key:return   Send an ordered sequence of chunks / key events
                                          (0.3s gap between items by default)
  pty send <ref> --with-delay <sec> --seq ...    Override the gap; --with-delay 0 = straight stream
  pty send <ref> --paste "<big text>"     Wrap the payload in bracketed-paste markers
  pty send --remote <peer> <ref> "text"   Send to a session on a fabric peer (over fabric)

Observe:
  pty peek <ref>                          Print current screen and exit
  pty peek --plain <ref>                  Print current screen as plain text (no ANSI)
  pty peek --full <ref>                   Print full scrollback (not just the viewport)
  pty peek --wait "text" [-t N] <ref>     Wait until text appears (optional timeout in seconds)
  pty peek -f <ref>                       Follow output read-only (Ctrl+\ to stop)
  pty peek --remote <peer> <ref>          Peek a session on a fabric peer (over fabric)
  pty events <ref>                        Follow events from a session
  pty events --all                        Follow events from every session, interleaved
  pty events --recent <ref>               Print recent events and exit
  pty events --json <ref>                 Emit raw JSONL
  pty stats                               Live CPU / memory / PIDs for every session
  pty stats <ref>                         Live metrics for a single session
  pty stats --json                        Emit stats as JSON (one snapshot)
  pty list                                List sessions (text; alias: pty ls)
  pty list --json                         List sessions as JSON
  pty list --tags                         Include internal bookkeeping tags (ptyfile*, strategy.*)
  pty list --filter-tag key=value         Filter to sessions with the tag (repeatable, ALL must match)
  pty list --remote <peer>                List a fabric peer's sessions (over fabric)
  pty list --remote                       Include remote sessions via pty-relay (when installed)
  pty remote-serve --stdio                Serve remote access on-demand (fabric --exec spawns it per dial)
  pty remote-serve --socket <path>        Serve remote access as a listening daemon (being retired)

Modify:
  pty metadata patch --id <id>            Atomically merge displayName/tags from JSON stdin
  pty evidence snapshot --id <id>         Read exact-generation retained exit evidence as JSON
  pty rename <label>                      Inside a session: set its displayName
  pty rename <ref> <label>                Outside: set displayName on <ref>
  pty rename --show <ref>                 Print the current displayName
  pty rename --clear [ref]                Remove the displayName
  pty tag <ref>                           Show tags on a session
  pty tag <ref> key=value [key=value...]  Set tags
  pty tag <ref> --rm key [--rm key...]    Remove tags
  pty tag-multi <selector> [ops...]       Bulk read / write tags across sessions
                                          Selector (one of): --all | --filter-tag k=v | <ref>...
                                          Ops (any of): key=value | --rm key
                                          --all + write requires --yes
  pty emit user.<type> [--json <p>] [--text <s>]     Publish a user.* event (inside a session)
  pty emit <ref> user.<type> [...]        Same, targeting a specific session

Lifecycle:
  pty restart <ref>                       SIGTERM + respawn using stored metadata (prompts if running)
  pty restart -y <ref>                    Same, no prompt
  pty kill <ref>                          Terminate a running session and its descendants
  pty recover <name> --snapshot <file>    Rebind a supporting live daemon after registry unlink
  pty rm <ref>                            Remove an exited session's metadata (alias: pty remove)
  pty evidence remove --id <id> --expected-generation <opaque>
                                          Remove only the matching terminal generation
  pty gc                                  Reconciliation pass: orphan-kill, abandoned-reap,
                                          permanent-respawn, exited-sweep
  pty gc --dry-run                        Preview without changing anything (alias: -n)
  pty gc --idle-days N                    Also reap permanents with no attach in N days
  pty gc --fast-fail-window=N             Fast-fail window (seconds) for the respawn cap
                                          (default 60; per-session strategy.fast-fail-window wins)
  pty gc --fast-fail-limit=N              Consecutive fast fails before a permanent is flagged
                                          flapping (default 3; per-session tag wins)
  pty gc --print-launchd-plist [--interval=N]
                                          Print a launchd plist that runs 'pty gc' every N seconds
                                          (default 30); Label + logPath derived from PTY_ROOT

Multi (pty.toml):
  pty up                                  Start every session in ./pty.toml
  pty up <dir>                            Start sessions in <dir>/pty.toml
  pty up <name> [<name>...]               Start specific sessions from ./pty.toml
  pty down                                Stop every session in ./pty.toml
  pty down <dir>                          Stop sessions in <dir>/pty.toml
  pty down <name> [<name>...]             Stop specific sessions

Global:
  pty --root <path> <subcommand> [...]    Pin the state registry for this call (== PTY_ROOT env)
  pty help | pty --help | pty -h          Show this usage
  pty version | pty --version | pty -v    Print the version (<semver>+<short-sha>)
  pty test [watch | -t "pattern"]         Run the pty test suite (vitest passthrough)

Session references (<ref>): the on-disk id (validated: [A-Za-z0-9._-], ≤ 255 chars,
socket path ≤ 104 bytes), or a displayName. Stable ids always win; a displayName
resolves only when unique. Inside a session, most commands default to $PTY_SESSION
when the ref is omitted (see 'pty rename', 'pty exec', 'pty emit').

Env:
  PTY_ROOT                Registry dir (default ~/.local/state/pty). Canonical.
  PTY_SESSION_DIR         Deprecated alias for PTY_ROOT; still works, one-time notice.
  PTY_ROOT_LEGACY_SILENT  Suppress the PTY_SESSION_DIR deprecation notice.
  PTY_SESSION             Set by the daemon inside a session; drives nesting detection.

Detach from an attached session with Ctrl+\ (press twice to send Ctrl+\ to the child).
```
(Note the source has one extra space in the `pty attach --no-restart <ref>` line — reproduce it.)

### 3.2 Per-command help (`COMMAND_HELP`, cli.ts:109-451)

`pty run --help`:
```
Usage: pty run [flags] -- <command> [args...]

Create a session and attach to it (use -d to leave it running in the background).

Flags:
  --id <id>            Pin the on-disk id (sock/json filename; charset-validated, ≤ 104-byte sock path)
  --name <label>       Display label (trimmed, single-line, ≤ 160 Unicode scalars)
  --no-display-name    Skip the auto cwd+command label — just the id
  -d, --detach         Create in the background; don't attach
  -a, --attach         Create, OR attach if a session with the same id already exists
  -e, --ephemeral      Force self-removal at exit even for strategy=permanent
                       (non-permanent sessions already self-remove by default)
  --tag key=value      Tag the session (repeatable)
  --env KEY=VALUE      Overlay a child environment variable (repeatable)
  --unset-env KEY      Remove an inherited environment variable (repeatable)
  --tag keep=true      Exempt from reaping: keep metadata/logs after exit
  --cwd <path>         Working directory for the command
  --isolate-env        Scrub the child env to a safe allow-list (for remote-reachable sessions)
  --force              Create even from inside another pty session (bypass the nesting guard)

Examples:
  pty run -- node server.js
  pty run -d --name "API" --tag role=web --env PORT=3000 -- node server.js
```

`pty attach --help` (also `pty a --help`):
```
Usage: pty attach [-r|--no-restart] [--force] [--remote <peer>] [--attach-stream-fd-v1 <fd>] <ref>

Reconnect to a session (alias: pty a). Detach again with Ctrl+\.

Flags:
  -r, --auto-restart   Auto-restart the session if it has exited
  --no-restart         Attach only while the session is running; never prompt
                       or execute its stored command
  --force              Attach even from inside another pty session (nested)
  --remote <peer>      Attach a session on a fabric peer (over fabric); <ref> is
                       the session's name/id ON THE REMOTE
  --attach-stream-fd-v1 <fd>
                       Machine mode for a running session. Write ordered framed
                       GEOMETRY, SCREEN, DATA, and terminal EXIT or DETACH outcome
                       to inherited fd (>= 3); keep stdin/stdout controlling TTY

Examples:
  pty attach myserver
  pty attach -r myserver
  pty attach --no-restart myserver
  pty attach --remote hetzner myshell
```

`pty exec --help`:
```
Usage: pty exec -- <command> [args...]

Replace the current session's leaf process with a new command. Run INSIDE a
session (uses $PTY_SESSION); the session keeps its id and metadata.

Examples:
  pty exec -- codex
  pty exec -- bash -l
```

`pty peek --help`:
```
Usage: pty peek [-f] [--plain] [--full] [--wait <text> [-t <sec>]] [--remote <peer>] <ref>

Print a session's screen (or follow it, or wait for text) without attaching.

Flags:
  --plain              Plain text, no ANSI escapes (best for scripts / agents)
  --full               Full scrollback, not just the visible viewport
  -f, --follow         Follow output read-only (Ctrl+\ to stop)
  --wait <text>        Block until <text> appears on screen
  -t, --timeout <sec>  Timeout (seconds) for --wait
  --remote <peer>      Peek a session on a fabric peer (over fabric); <ref> is
                       the session's name/id ON THE REMOTE (--wait not yet supported)

Examples:
  pty peek --plain myserver
  pty peek --remote hetzner myserver
  pty peek --wait "Listening" -t 10 --plain myserver
```

`pty send --help`:
```
Usage: pty send <ref> "text"
       pty send <ref> --seq <chunk> [--seq key:<name>] ...
       pty send --remote <peer> <ref> "text"

Send text or key events to a session. Raw text is sent with NO implicit newline —
to send text followed by Enter, use --seq (see the second example).

Flags:
  --seq <value>        Ordered chunk or key event (repeatable). key:<name> sends a
                       key, e.g. key:return, key:ctrl+c, key:ctrl-c, key:C-c.
                       Modifiers also accept _ separators; names ignore case.
  --with-delay <sec>   Delay (seconds) between --seq items. DEFAULT 0.3s so a
                       trailing key:return doesn't race ahead of the program
                       parsing the text. --with-delay 0 = straight stream (no gap).
  --paste "<text>"     Wrap the payload in bracketed-paste markers
  --remote <peer>      Send to a session on a fabric peer (over fabric); <ref> is
                       the session's name/id ON THE REMOTE

Examples:
  pty send myserver "hello"
  pty send myserver --seq "git status" --seq key:return        # 0.3s gap by default
  pty send --remote hetzner myserver --seq "ls" --seq key:return
```

`pty events --help`:
```
Usage: pty events [--all | <ref>] [--recent] [--json] [--wait <type> [-t <sec>]]

Follow a session's event log (bell, title, notifications, tag/rename changes, user.* events).

Flags:
  --all                Follow every session, interleaved (omit <ref>)
  --recent             Print recent events and exit (don't follow)
  --json               Emit raw JSONL
  --wait <type>        Block until an event of <type> appears
  -t, --timeout <sec>  Timeout (seconds) for --wait

Examples:
  pty events myserver
  pty events --recent --json myserver
```

`pty list --help` (also `pty ls --help`):
```
Usage: pty list [--json] [--tags] [--filter-tag k=v] [--remote [<peer>]] [--status <s>] [--summary]

List sessions (alias: pty ls). User tags show by default.

Flags:
  --json               Emit JSON
  --tags               Include internal bookkeeping tags (ptyfile*, strategy.*)
  --filter-tag k=v     Only sessions with the tag (repeatable, ALL must match)
  --remote <peer>      Also list a fabric peer's sessions (over fabric; the peer
                       runs 'pty remote-serve' exposed as 'fabric expose pty-remote')
  --remote             Bare (no peer): include pty-relay hosts (when installed)
  --status <state>     Filter by status: running | exited | vanished
  --older-than <dur>   Only sessions older than a duration (e.g. 30m, 2h, 3d)
  --newer-than <dur>   Only sessions newer than a duration
  --summary            Print a one-line count summary instead of the list

Examples:
  pty list
  pty list --remote hetzner
  pty list --filter-tag role=web --json
```

`pty remote-serve --help`:
```
Usage: pty remote-serve (--stdio | --socket <path>)

Serve the remote-access control protocol so a fabric peer can expose pty and
other machines can 'pty <cmd> --remote <this-peer>'. Reads sessions from the
ambient PTY_ROOT — run it in the same env the sessions use. Two forms:

  --stdio            On-demand: serve ONE connection over stdin/stdout, then exit.
                     fabric spawns it per dial and owns accept + persistence +
                     roaming (a drop/reconnect reuses the SAME process). No
                     persistent pty daemon. The recommended fabric form.
  --socket <path>    Listening daemon: bind a Unix socket for a fabric peer to
                     expose. Pick a path OUTSIDE PTY_ROOT (a control socket inside
                     it is mis-scanned as a phantom session). Run it WRAPPED —
                     'setsid sh -c "…"', systemd, launchd — so pty is a CHILD of
                     the session leader (exec'd as a bare session leader without a
                     TTY it can exit on detach). Being retired in favor of --stdio.

Flags:
  PTY_REMOTE_SERVE_DEBUG=1   Env: log signal/exit/exception lifecycle to stderr

Examples:
  fabric expose pty-remote --exec -- pty remote-serve --stdio   # on-demand (recommended)
  pty remote-serve --socket ~/.local/state/pty-remote.sock      # listening daemon
  setsid sh -c 'pty remote-serve --socket ~/.local/state/pty-remote.sock' </dev/null &   # wrapped
  fabric expose pty-remote --socket ~/.local/state/pty-remote.sock   # expose the listening form
```

`pty stats --help`:
```
Usage: pty stats [--json] [--all] [<ref>]

Live CPU / memory / PIDs. Omit <ref> for every session.

Flags:
  --json               Emit stats as JSON (one snapshot)
  --all                Include every session (with an explicit <ref> given)

Examples:
  pty stats
  pty stats --json myserver
```

`pty restart --help`:
```
Usage: pty restart [-y] [--force] <ref>

SIGTERM the session's daemon and respawn it from stored metadata (command, cwd,
tags, displayName). Prompts first if it's still running.

Flags:
  -y, --yes            Skip the "kill and restart?" prompt
  --force              Attach after restart even from inside another pty session

Examples:
  pty restart myserver
  pty restart -y myserver
```

`pty kill --help`:
```
Usage: pty kill <ref>

Terminate a running session's daemon and exact descendant tree. Metadata is kept —
restart or `pty rm` it later.

Examples:
  pty kill myserver
```

`pty recover --help`:
```
Usage: pty recover <name> --snapshot <metadata.json>

Ask the original supporting daemon to republish an externally unlinked socket
and registry without signaling or restarting its PTY child.

The snapshot must have been captured from the same selected PTY_ROOT before
the registry was unlinked and must advertise a recovery capability.

Example:
  pty --root /state/pty recover myserver --snapshot ./myserver.json
```

`pty rm --help` (also `pty remove --help`):
```
Usage: pty rm <ref>

Remove an exited session's files (socket/pid/json/events) (alias: pty remove).
Won't remove a running session — kill it first.

Examples:
  pty rm myserver
```

`pty gc --help`:
```
Usage: pty gc [-n] [--idle-days N] [--fast-fail-window=N] [--fast-fail-limit=N]
       pty gc --print-launchd-plist [--interval=N]

One reconciliation pass: sweep exited/vanished, orphan-kill `parent=<name>` children,
reap abandoned permanents, respawn `strategy=permanent` sessions.

Non-permanent sessions remove themselves as they exit, so the sweep is a backstop:
it mainly catches `vanished` sessions, whose daemon was killed outright and so
never ran its own cleanup. Sessions tagged `keep` are never swept.

Flags:
  -n, --dry-run           Preview without changing anything
  --idle-days N           Also reap permanents with no attach in N days
  --fast-fail-window=N    Fast-fail window seconds (default 60; per-session tag wins)
  --fast-fail-limit=N     Consecutive fast fails before flapping (default 3; per-session tag wins)
  --print-launchd-plist   Print a macOS launchd plist that runs 'pty gc' on an interval
  --interval=N            Plist StartInterval seconds (default 30)

Examples:
  pty gc --dry-run
  pty gc --print-launchd-plist > ~/Library/LaunchAgents/com.compoundingtech.pty.gc.plist
```

`pty tag --help`:
```
Usage: pty tag <ref>                           Show tags
       pty tag <ref> key=value [key=value...]   Set tags
       pty tag <ref> --rm key [--rm key...]     Remove tags

Read or write tags on one session. Updates apply before removals.

Flags:
  --rm <key>           Remove a tag key (repeatable)

Examples:
  pty tag myserver role=web env=prod
  pty tag myserver --rm env
```

`pty tag-multi --help` — NOTE: two different texts exist. The central interceptor (`args[1] === "--help"`) prints the `COMMAND_HELP["tag-multi"]` entry below; a `-h`/`--help` appearing later in the argv (e.g. `pty tag-multi --all --help`) is handled by `parseTagMultiArgs` and prints `printTagMultiHelp()` (section 3.3) instead.
```
Usage: pty tag-multi <selector> [ops...]

Bulk read / write tags across many sessions.
  Selector (one of): --all | --filter-tag k=v (repeatable) | <ref>...
  Ops (any of):      key=value | --rm key

Flags:
  --all                Select every session
  --filter-tag k=v     Select sessions with the tag (repeatable)
  --rm <key>           Remove a tag key (repeatable)
  --json               Read mode: emit tags as JSON
  -y, --yes            Required to write when the selector is --all

Examples:
  pty tag-multi --filter-tag role=web env=prod
  pty tag-multi --all --json
```

`pty emit --help` (also printed by `cmdEmit` for `-h`/`--help` anywhere, and to stdout with exit 1 on wrong positional count):
```
Usage: pty emit <type> [--json <payload>] [--text <string>]
       pty emit <ref> <type> [--json <payload>] [--text <string>]

Publish a user.* event to a session's event log. Inside a session the ref
defaults to $PTY_SESSION. Types must start with "user." — "session_*", "state.*",
"bell", etc. are reserved.

Flags:
  --json <payload>     Attach a JSON payload
  --text <string>     Attach a text payload

Examples:
  pty emit user.build-done
  pty emit user.progress --json '{"pct": 40}'
  pty emit myserver user.tests-passed --json '{"n": 42}'
```
(Note the misaligned `--text` column — one space fewer — reproduce as is.)

`pty rename --help` (also printed to **stderr** by `renameUsage()` on error paths):
```
Usage: pty rename <new-display-name>          Inside a session: set displayName
       pty rename <ref> <new-display-name>    Outside: set displayName on <ref>
       pty rename --show <ref>                Show the current displayName
       pty rename --clear [ref]               Clear the displayName

displayName is a mutable, non-unique label; the session's stable id (name) never changes.
An ambiguous displayName must be replaced with one of the reported stable ids.

Examples:
  pty rename my-friendly-name
  pty rename webapp "Web Frontend"
  pty rename --show webapp
```

`pty metadata --help` (also `pty metadata patch --help`):
```
Usage: pty metadata patch --id <stable-id>

Atomically merge displayName and tags for one exact stable session id. Reads
one JSON object from stdin; it never resolves display-name aliases.

Patch fields:
  displayName   string to set, null to clear, omitted to preserve
  tags          object of string values to set and null values to remove

Examples:
  pty metadata patch --id a1b2c3d4 < patch.json
  printf '%s' '{"displayName":"Worker","tags":{"role":"worker"}}' | pty metadata patch --id a1b2c3d4
  printf '%s' '{"displayName":null,"tags":{"temporary":null}}' | pty metadata patch --id a1b2c3d4
```

`pty evidence --help`:
```
Usage: pty evidence snapshot --id <stable-id>
       pty evidence remove --id <stable-id> --expected-generation <opaque>

Read retained terminal evidence for one exact stable session generation, or
remove that generation after the caller has durably consumed the evidence.
Both operations emit exactly one tagged JSON document on stdout. Semantic
outcomes exit 0; invalid arguments and operational failures exit nonzero.

Examples:
  pty evidence snapshot --id a1b2c3d4
  pty evidence remove --id a1b2c3d4 --expected-generation 7f44b35e
```

`pty evidence snapshot --help` (cli.ts:454-460):
```
Usage: pty evidence snapshot --id <stable-id>

Emit one tagged JSON snapshot of retained terminal evidence for the exact
stable session id.

Example:
  pty evidence snapshot --id a1b2c3d4
```

`pty evidence remove --help` (cli.ts:461-467):
```
Usage: pty evidence remove --id <stable-id> --expected-generation <opaque>

Remove one terminal session only when it still carries the opaque generation
returned by an earlier snapshot. Emits one tagged JSON result.

Example:
  pty evidence remove --id a1b2c3d4 --expected-generation 7f44b35e
```

`pty up --help`:
```
Usage: pty up [<dir>] [<name>...]

Start sessions declared in a pty.toml. With no args, reads ./pty.toml and starts all.

Examples:
  pty up
  pty up ./backend
  pty up web worker
```

`pty down --help`:
```
Usage: pty down [<dir>] [<name>...]

Stop sessions declared in a pty.toml.

Examples:
  pty down
  pty down web
```

`pty test --help`:
```
Usage: pty test [watch | -t "<pattern>"]

Run the pty test suite (a thin vitest passthrough).

Examples:
  pty test
  pty test -t "peek"
```

### 3.3 `printTagMultiHelp()` (cli.ts:3489-3511; printed for `-h`/`--help` inside tag-multi's own parser, exit 0)

```
Usage:
  pty tag-multi <selector> [--json] [--yes] [<ops>...]

Selectors (pick one):
  <name>...                explicit list of session names or displayNames
  --filter-tag k=v         sessions matching tag (repeatable for AND)
  --all                    every session

Operations (presence flips command into write mode):
  k=v                      set tag k to v
  --rm k                   remove tag k

Flags:
  --json                   structured output (object: name → tags)
  --yes / -y               required with --all when ops are present

Examples:
  pty tag-multi --all --json
  pty tag-multi --filter-tag role=web env=prod
  pty tag-multi sess-a sess-b --rm temp-flag
  pty tag-multi --all --yes audit=2026-04-25
```

### 3.4 `pty completions` usage (completions.ts:710-721; stdout for `--help`, stderr otherwise; `console.log`/`console.error` add one more newline)

```
usage: pty completions <shell>

Print a shell completion script to stdout.

Shells:
  fish
  bash
  zsh

Examples:
  pty completions fish > ~/.config/fish/completions/pty.fish
  pty completions bash > /etc/bash_completion.d/pty
  pty completions zsh  > "${fpath[1]}/_pty"
```

### 3.5 Inline usage strings printed on argument errors (stderr, exit 1)

- run: `Usage: pty run [--id <id>] [--name <displayName>] [-d] [-a] -- <command> [args...]` (cli.ts:863)
- attach: `Usage: pty attach [-r|--auto-restart|--no-restart] [--force] [--remote <peer>] <name>` (cli.ts:1012)
- exec: `Usage: pty exec -- <command> [args...]` (cli.ts:1059)
- peek: `Usage: pty peek [-f] [--plain] [--full] [--wait <pattern>] [-t <seconds>] [--remote <peer>] <name>` (cli.ts:1087)
- send: `Usage: pty send [--remote <peer>] <name> "text"  or  pty send <name> --seq "text" --seq key:return` (cli.ts:1129)
- events: `Usage: pty events [--all] [--recent] [--json] [--wait <type>] [-t <seconds>] [<name>]` (cli.ts:1237)
- restart: `Usage: pty restart [-y] [--force] <name>` (cli.ts:1376)
- kill: `Usage: pty kill <name>` (cli.ts:1386)
- recover: `Usage: pty recover <name> --snapshot <metadata.json>` (cli.ts:1399)
- tag: `Usage: pty tag <name> [key=value...] [--rm key...]` (cli.ts:1458)
- rm: `Usage: pty rm <name>` (cli.ts:1607)
- remote-serve: see 2.8. metadata/evidence: see 2.19/2.20.

---

## 4. COMPLETIONS (`pty completions <shell>`, src/completions.ts)

- Shells: `fish`, `bash`, `zsh` (completions.ts:702-708). Output is generated from one declarative spec (`COMMANDS`, completions.ts:71-311; `GLOBAL_FLAGS` = `--root`, `--preselect-new`, `--filter-tag`, completions.ts:314-318) and written raw to stdout ending with `\n`.
- **The checked-in files `completions/pty.fish`, `completions/pty.bash`, `completions/pty.zsh` must be byte-identical to the generator output** (tests/completions.test.ts:81-89). A port can either embed those three files verbatim or reimplement the generator exactly. Header comment lines: fish `# fish completions for pty — generated by `pty completions fish`.` / `# Regenerate with: pty completions fish > completions/pty.fish` / `# (kept in sync with src/cli.ts; see src/completions.ts)`; bash `# bash completion for pty — generated by `pty completions bash`.` / `# Regenerate with: pty completions bash > completions/pty.bash`; zsh `#compdef pty` / `# zsh completion for pty — generated by `pty completions zsh`.` / `# Regenerate with: pty completions zsh > completions/pty.zsh`.
- Exit codes: `--help`/`-h` -> usage to stdout, 0; no shell -> usage to stderr, 2; unknown shell -> stderr `pty completions: unknown shell: <shell>\n` + usage, 2 (tests: `tcsh` -> status 2, stderr matches `/unknown shell/i`; `--help` -> status 0, stdout matches `/usage: pty completions/`).
- What they complete:
  - Top level: every command name and alias (`run attach a exec peek send events list ls stats restart kill recover rm remove gc tag tag-multi emit rename metadata evidence up down test remote-serve`) and the global flags.
  - Per command: the flag list from the spec (with short forms `-d -a -e` for run, `-r` attach, `-f -t` peek, `-t` events, `-y` restart/tag-multi, `-n` gc), enum values for `list --status` (`running exited vanished`), a free argument for `attach --attach-stream-fd-v1 <fd>` (fish `-x`, bash `"${prev}" == "--attach-stream-fd-v1"`, zsh `:fd:`), positional values `patch` for `metadata` and `watch` for `test`, and nested leaves for `evidence` (`snapshot` with `--id`; `remove` with `--id` and `--expected-generation`).
  - Dynamic session names for `attach peek send events stats restart kill rm tag tag-multi emit rename`: derived at completion time from `<root>/*.json` basenames where root = `$PTY_ROOT`, else `$PTY_SESSION_DIR`, else `$HOME/.local/state/pty` (fish `__pty_root`/`__pty_sessions`, bash `names=$(ls "${root}"/*.json | xargs -I{} basename {} .json)`, zsh `${root}/*.json(N:t:r)`).
  - Directory completion for `exec`, `up`, `down` (`takesPath`).
- Spec parity test: every key of `COMMAND_HELP` must appear as a command name or alias in the spec (completions.test.ts:54-62); `evidence` must be modeled with the two subcommands and their flags (completions.test.ts:64-77); every generated script must contain the `--env` flag for run (`-l env` in fish).
- `scripts/install-completions.sh` symlinks the checked-in files into Homebrew bash/zsh dirs and `~/.config/fish/completions/pty.fish` (npm script `install-completions`).

---

## 5. OTHER USER-VISIBLE SEMANTICS A PORT MUST REPRODUCE

### 5.1 Key chord notation (`--seq key:<spec>`, src/keys.ts)

- `parseSeqValue(v)`: if `v` starts with `key:` -> `resolveKey(v.slice(4))`, else the literal string (keys.ts:166-171).
- `resolveKey(spec)`: lowercased. Named keys (KEY_MAP, keys.ts:1-18): `return`/`enter` -> `\r`, `tab` -> `\t`, `escape`/`esc` -> `\x1b`, `space` -> ` `, `backspace` -> `\x7f`, `delete` -> `\x1b[3~`, `up` `\x1b[A`, `down` `\x1b[B`, `right` `\x1b[C`, `left` `\x1b[D`, `home` `\x1b[H`, `end` `\x1b[F`, `pageup` `\x1b[5~`, `pagedown` `\x1b[6~`. Modifiers `ctrl`, `alt`, `shift`, separated by any of `+ - _` (keys.ts:20-21); a leading `c-`/`C-` means ctrl (keys.ts:48-54, only index 0 and only with `-`, so `C+u` is an unknown modifier). Base keys: named keys or a single letter `a-z`.
- Letters: shift -> uppercase; ctrl -> `charCode - 96` of the (lowercased) letter (so `ctrl+u` -> `\x15`, `ctrl+c` -> `\x03`); alt -> prefix `\x1b`. Order: shift, ctrl, alt (keys.ts:113-131). E.g. `ctrl+alt+c` -> `\x1b\x03`.
- Named keys with modifiers: modifier param `1 + shift(1) + alt(2) + ctrl(4)`; `shift+tab` -> `\x1b[Z`; CSI `~` keys -> `\x1b[<n>;<mod>~` (e.g. `ctrl-alt-delete` -> `\x1b[3;7~`); CSI letter keys -> `\x1b[1;<mod><L>` (e.g. `shift+up` -> `\x1b[1;2A`, `ctrl_alt+shift_up` -> `\x1b[1;8A`); control-char keys use CSI-u: return/enter 13, tab 9, escape/esc 27, space 32, backspace 127 -> `\x1b[<code>;<mod>u` (e.g. `shift+return` -> `\x1b[13;2u`, `shift+backspace` -> `\x1b[127;2u`) (keys.ts:138-162).
- Errors (all `Error` -> stderr message, exit 1), with `KEY_SPEC_HELP` = `Use ctrl+u, ctrl-u, ctrl_u, or C-u; supported modifiers are ctrl, alt, and shift; supported keys are a-z, backspace, delete, down, end, enter, esc, escape, home, left, pagedown, pageup, return, right, space, tab, up.`:
  - `Ambiguous key spec "<spec>": it is both a named key and a modifier chord. <HELP>` (cannot currently trigger)
  - `Incomplete key spec "<spec>". <HELP>` (empty base or empty modifier, e.g. `ctrl-`)
  - `Unknown modifier: "<mod>" in key spec "<spec>". <HELP>` (e.g. `super+c`, `C+u`)
  - `Unknown key: "<base>" in key spec "<spec>". <HELP>` (e.g. `f99`)
- The whole `--seq` list is resolved before anything is sent (a bad spec after a valid chunk sends nothing).

### 5.2 Durations (`--older-than`/`--newer-than`, src/duration.ts)

`parseDuration`: `/^(\d+)\s*(s|m|h|d)$/i` on the trimmed input -> ms (`s`=1000, `m`=60000, `h`=3600000, `d`=86400000); anything else (compound `1h30m`, `5`, `s`, `5y`, `5w`, `5ms`, `-5m`, `1.5h`, empty) -> `null`. Whitespace: leading/trailing and between number and unit tolerated (`"  5m  "`, `"5 m"`); unit case-insensitive. `formatDuration(ms)` (used by `list --summary`): `<s>s` (<60 s), `<m>m` or `<m>m<s>s`, `<h>h` or `<h>h<m>m`, `<d>d` or `<d>d<h>h`; negatives clamp to `0s`.

### 5.3 `send` payload framing

Bracketed paste markers are `\x1b[200~` (start) and `\x1b[201~` (end) (paste.ts:14-16), sent once around the whole payload as separate DATA packets; `--paste` composes with `--seq` and `--with-delay`; no CR/LF translation; default 300 ms inter-item gap; `--with-delay 0` = no gap; `--with-delay N` = `round(N*1000)` ms.

### 5.4 Child environment policy (server.ts:131-209; README.md:312-327; docs/client.md:230-233)

- Default: full inheritance of the daemon env (which is the `pty run` invoker's env, minus `PTY_SERVER_CONFIG`, minus `ST_AGENT`/`ST_ROOT` on restart paths), then `unsetEnv` removals, then `extraEnv` assignments (later `--env` for the same key wins; an `--env` assignment beats an `--unset-env` of the same key regardless of flag order), then `PTY_SESSION=<id>` and `PTY_SESSION_GENERATION=<32-hex>` are forced, then `TERM` defaults to `xterm-256color` if absent.
- `--isolate-env`: start from only `PATH HOME USER LOGNAME SHELL TERM COLORTERM LANG TZ PWD TMPDIR PTY_ROOT PTY_SESSION_DIR` plus any `LC_*`, then the same removal/assignment/forced steps.
- `env` (library-only exact env) is mutually exclusive with the above (`ServerOptions.env is mutually exclusive with isolateEnv/extraEnv/unsetEnv. ...`).
- `extraEnv`, `unsetEnv`, `isolateEnv`, `ephemeral`, `rows`, `cols` are persisted in `<id>.json` and reused by `pty restart`, `attach -r`, `run -a` recreate and `gc` respawn. `pty.toml` `[sessions.X.env]` becomes `extraEnv`.

### 5.5 `--cwd` handling

`pty run --cwd <path>` passes the string verbatim (cli.ts:798, 1741, spawn.ts:174); default is the invoking `process.cwd()`. The daemon validates it (`describeInvalidCwd`, section 2.1) and spawns the child in it; `metadata.cwd` stores the given string. `pty up` resolves `cwd` relative to the manifest directory (`path.resolve(manifestDir, cwd)`), default = manifest dir. A `gc` abandoned-reap fires when `metadata.cwd` no longer exists (permanent sessions only).

### 5.6 Event text format (`formatEvent`, events.ts:548-604) — used by `pty events` without `--json`

Prefix `[<HH:MM:SS>] <session>:` where the time is `new Date(ts).toLocaleTimeString("en-US", { hour12: false })` (local time zone; note Node renders midnight hour as `24` in some versions). Bodies: `bell`; `title -> "<value>"`; `notification[ -- "<title>"][ <body>]`; `focus requested`; `cursor restored`; `started[ k=v k2=v2]`; `exited (code <n>)` or `killed by signal <sig> (code <n>)`; `exec <command> (was <previousCommand>)`; `respawned`; `abandoned (idle <n>d)` or `abandoned (<reason>)`; `display_name -> "<value>"|null (was "<previous>"|null)` (JSON-quoted); `tags -> k=v k2=v2|{} (was ...)`; `metadata -> <JSON value> (was <JSON previous>)`; default (user.* and unknown): `<type>[ "<text>"| <JSON data>]`.

### 5.7 On-disk contract touched by the CLI (docs/disk-layout.md)

`<root>/<id>.json` (pretty-printed `SessionMetadata`, section 1.4 / sessions.ts:137-181), `<id>.events.jsonl` (append-only JSONL `{session,type,ts,...}`; 1000 -> 500 line retention), `<id>.sock`, `<id>.pid` (decimal daemon pid + newline), `<id>.lock` (creation lock: holder pid; stale/garbage locks are reclaimed), `<id>.events.lock`, `.recovery/`, `theme`, `gc.log`, `*.tmp.<pid>.<rand>` atomic-write temporaries (readers must ignore). Root mode 0700.

### 5.8 Timeouts / constants a port must match

Daemon start wait 30 s (`DEFAULT_START_TIMEOUT_MS`, spawn.ts:109); post-socket settle 100 ms (spawn.ts:361); `pty kill` and `pty rm` daemon-exit wait 7 s; `restart` post-SIGTERM sleep 200 ms; `queryStats` 2 s; `peek --wait` poll 200 ms; fabric dial / route / remote list 10 s; pty-relay 5 s; socket liveness probe budget 500 ms per `listSessions`; daemon exit-broadcast grace 500 ms before shutdown (server.ts:1589); daemon shutdown backstop 5 s (`PTY_SHUTDOWN_DEADLINE_MS`); detach double-tap window 300 ms; events keep-alive tick 60 s; `readRecentEvents` default 50; `lastLines` cap 200; `sun_path` limit 104 bytes; random id 8 chars; displayName <= 160 scalars; name <= 255 chars; packet max 32 MiB.

### 5.9 Session id / displayName generation and reuse rules (summary)

- Random ids: 8 chars from `23456789abcdefghjkmnpqrstuvwxyz`; tests only pin `/^[a-z0-9]{6,12}$/`.
- `pty run`: auto displayName `<basename(cwd)>-<basename(cmd)>[-<firstArgBase>]` sanitized (section 1.11); `--name` overrides; `--no-display-name` omits; legacy positional forms set the displayName with a stderr hint.
- `pty up`: displayName = manifest `display_name` or `<prefix>-<key>`/`<key>`; id = manifest `id` or random; re-runs match by the `(ptyfile, ptyfile.session)` tag pair, not by name.
- `run -a` on a gone session reuses previous `displayName`, `cwd`, `tags` (only if no `--tag` given — new `--tag`s replace the whole set), `extraEnv`, `unsetEnv`.
- Display names are non-unique and may equal another session's id; an exact id always wins on lookup; ambiguity fails closed with the id list.

### 5.10 Reserved / behavioral tags

Reserved (hidden without `--tags`): `ptyfile`, `ptyfile.session`, `ptyfile.tags`, `strategy`, and any key starting with `:` (tags.ts:56-79). Behavioral: `strategy=permanent` (gc respawn, `[permanent]` marker, never self-reaped), `strategy.status=flapping` (`[flapping]` marker, gc skip), `strategy.consecutive-fast-fails`, `strategy.last-respawn-at`, `strategy.command-hash` (gc bookkeeping; cleared by `restart`/`up`), `strategy.fast-fail-window`, `strategy.fast-fail-limit`, `strategy.idle-days`, `strategy.abandon-if-cwd-gone=false`, `parent=<id>` (orphan kill), `keep` (any value other than `false/0/no/off` = keep; exempt from self-reap and gc sweep), `role=agent` (restart guard), `:l<pid>-<rand>` (layout tags pruned by gc).

---

## 6. TEST-PINNED CONTRACTS (every asserted literal, grouped by command)

Test harness facts common to all: tests run `node dist/cli.js <args>`; most isolate with `PTY_SESSION_DIR=<tmpdir>` (some with `PTY_ROOT`), usually `PTY_ROOT_LEGACY_SILENT=1`; the vitest worker deletes ambient `PTY_ROOT`, `PTY_SESSION`, `PTY_SESSION_DIR`, `PTY_REAP_ON_EXIT` first (tests/setup/isolate-env.ts:20-28) and `vitest-global.ts` sets `PTY_ROOT_LEGACY_SILENT=1`. Daemons are often started directly (`node dist/server.js` with `PTY_SERVER_CONFIG={name,command,args,displayCommand,cwd,rows:24,cols:80,tags?,ephemeral?,extraEnv?,unsetEnv?}`), and exited sessions are kept observable by tagging `keep=true`. `status !== 0` below means "non-zero, exact value not pinned"; `status === N` is exact. Escapes such as `\u0007`, `\u2028`, `\u2029`, `\x1b` denote the corresponding single characters.

### 6.1 Global / env / version (tests/pty-root.test.ts, version.test.ts, gc-flap-clear-badge-root-len.test.ts, wrapper-signal-forwarding.test.ts, process-title.test.ts, nesting-prevention.test.ts)

- `pty --version`, `pty version`, `pty -v`, `pty -V` (no env): status 0; `stdout.trim()` matches `/^\d+\.\d+\.\d+(\+[0-9a-f]{4,})?$/`; the part before `+` equals package.json `version`; stderr does not contain `Unknown command` (version.test.ts:31-45).
- `pty list --json` with `PTY_ROOT=<a>` + `PTY_SESSION_DIR=<b>` + `PTY_ROOT_LEGACY_SILENT=1` (env built from scratch: only PATH/HOME + these): status 0, stdout parses to `[]` (pty-root.test.ts:37-53). Same with only `PTY_SESSION_DIR`: status 0, stderr matches `/PTY_SESSION_DIR is deprecated/` exactly once (:55-66); only `PTY_ROOT`: stderr does not match `/deprecated/` (:68-77); `PTY_SESSION_DIR` + `PTY_ROOT_LEGACY_SILENT=1`: no `/deprecated/` (:79-89); both set, no silencer: stderr matches `/both PTY_ROOT and PTY_SESSION_DIR are set/` exactly once (:233-245); both + silencer: no match (:247-257).
- `pty --root <dir> list --json` (no PTY_ROOT): status 0, `[]` (:93-102). `pty --root <flagRoot> list --json` with `PTY_ROOT=<envRoot>` containing a planted `leak.json` (`{command:"sh",args:[],displayCommand:"sh",cwd,rows:24,cols:80,tags:{},pid:999999,createdAt}`): `[]` (:104-124). `pty --root` (no value): status != 0, stderr matches `/--root requires a path/` (:126-133); `pty --root --json list`: same (:135-142).
- `pty run -d --id rd<rand> -- cat` with `PTY_ROOT=<root>`, `PTY_SESSION_DIR=<scratch>`, silencer: status 0; `<root>/<name>.json` and `<root>/<name>.sock` exist; `<scratch>/<name>.json` does not; `~/.local/state/pty/<name>.json` does not (:201-231).
- Root length: `pty list` with `PTY_ROOT=/tmp/` + 95 x `a`: status != 0; stderr matches `/PTY_ROOT is too long/`, `/104-byte kernel limit/`, `/Shorten the root/` (gc-flap-clear-badge-root-len.test.ts:163-180). `pty definitely-not-a-real-subcommand` with a 105-byte root: stderr matches `/PTY_ROOT is too long/` and NOT `/Unknown command/` (:182-195). A root of exactly 90 bytes (`/tmp/` + 85 x `c`): `pty list --json` status 0 -> `[]` (:197-212). `pty --root <short> list --json` with an over-long `PTY_ROOT` env: status 0 -> `[]` (:217-230).
- `node bin/pty remote-serve --socket <dir>/remote.sock` (env `PTY_SESSION_DIR`): stdout includes `pty remote-serve listening on <socketPath>` within 5 s; `pgrep -P <pid>` is empty (bin/pty spawns no child); after `SIGTERM` it exits within 5 s with exit code 0 and `signal === null` (wrapper-signal-forwarding.test.ts:50-91).
- Linux: after `pty run -d --id title-test --no-display-name -- sleep 30` (status 0), `/proc/<pid from title-test.pid>/comm` is `pty-daemon` (process-title.test.ts:38-62).
- Nested TUI: bare `pty`, `pty i`, `pty interactive` with `PTY_SESSION=outer-session`: status != 0, stderr contains `already inside pty session` (bare form also matches `/interactive picker|Ctrl/i`); `pty --force` alone: stderr does NOT contain `already inside pty session` (nesting-prevention.test.ts:213-241).

### 6.2 `pty run` (spawn-options, nesting, nesting-prevention, display-name, tags, restart-launch-parity, restart-env-scrub, exit-signal, pty-root tests)

- `run -d --id <name> -- cat` (`PTY_SESSION_DIR`): status 0; `queryStats(name)` gives `name` and `process.alive === true`; `<dir>/<name>.pid` is an integer (spawn-options.test.ts:249-272). With `PTY_CREATION_LOCK_OWNER_PID=<test pid>` and the test already holding the lock: status 0, `<name>.lock` content is the test pid, `.pid` written (:274-298).
- `run -d --id <name> --isolate-env --unset-env PATH --env PATH=<PATH> -- sh -c "env > /tmp/pty-iso-env.txt; exec /bin/sleep 30"` with `PTY_SECRET_TEST=...` in env: status 0; dump lacks `PTY_SECRET_TEST`, contains `PATH=<PATH>` and `PTY_SESSION=<name>` (:352-387). Without `--isolate-env`, a custom `PTY_LEGACY_MARKER` propagates (:389-414). `--isolate-env` with `TERM` deleted from the invoker env: child has `TERM=xterm-256color` (:617-651).
- Daemon config errors: missing cwd -> daemon exit 1, stderr contains `Working directory does not exist: <dir>` and `Cannot start session "<name>"`; file as cwd -> `Working directory is not a directory: <path>`, no `posix_spawnp failed` (:513-536). `sh -lc 'cd <d> && rmdir <d> && exec node dist/cli.js list'`: status 0, stderr lacks `uv_cwd` (:538-551).
- Library `spawnDaemon({displayName:" Worker"})` rejects with `Display name must be trimmed`, nothing written (:183-195); child env always has `PTY_SESSION_GENERATION` matching `/^[0-9a-f]{32}$/` (:460); `env` + isolate/extra/unset -> `/mutually exclusive/` (:463-511).
- Nested (`PTY_SESSION=outer-session`): `run -- echo hello`: stdout contains `hello`, stderr contains `Already inside pty session` and `outer-session`, status 0, `list --json` -> `[]` (nesting.test.ts:119-135); `run -a -- echo wrapped`: same shape (:137-152); `run -d --id <n> -- cat`: status 0, stderr lacks `Already inside pty session`, `list --json` has `{name, status:"running", pid}` (:154-175); `run -- sh -c "exit 42"`: status 42 (:177-187); with `PTY_SESSION` deleted, `run -d --id <n> -- cat` creates normally (:189-214). `run -a --id <running-target> -- cat` nested: status != 0, stderr contains `already inside pty session "outer-session"` and the target name (nesting-prevention.test.ts:245-254); `run -a --id <not-running> -- true` nested: stderr contains `Already inside pty session` and `running directly` (:256-265); `run --force --id <t> -- cat` nested creates a session listed `running` (:267-296); `run -- true` nested: `Already inside pty session`, `running directly`, status 0 (:298-305).
- Session `sh -c "echo PTY_SESSION=$PTY_SESSION; exec cat"` then `peek --plain <name>`: stdout contains `PTY_SESSION=<name>` (nesting.test.ts:106-117).
- `run -d -- cat`: one session; `name` matches `/^[a-z0-9]{6,12}$/`; `displayName` is a non-empty string (display-name.test.ts:67-80). `run -d --no-display-name -- cat`: `displayName` absent (:84-95). `run -d --id mysvc -- cat`: name `mysvc`, displayName truthy (:99-109). `--id raw --no-display-name`: no displayName (:111-120). `--id svc --name "My Pretty Service"`: both pinned (:122-132). `--id` of 120 x `x`: status != 0, stderr matches `/exceeds the.*byte kernel limit|too long/i` (:134-140). `--id dup` twice: second status != 0, stderr contains `already in use` (:142-149). `--name "My Very Long Display Label With Spaces and Punctuation"`: stored verbatim, name random (:153-166). `--id same --name same`: ok (:168-174). Two sessions `--name shared` (`a1`,`a2`): both allowed, listed in order `["a1","a2"]` (:176-184). Invalid `--name` values ` Worker`, `Worker `, `Worker\u0007`, `Worker\u2028Next`, `Worker\u2029Next`, `"😀".repeat(161)`: status != 0 and `list --json` -> `[]` (no session created) (:186-201); `"😀".repeat(156) + "/a\\b"` (160 scalars) accepted and round-trips (:203-211). A 110-char displayName `org.cos.orc-payments-platform.orc-checkout-api.worker-authz-service.subworker-db-migrations.verifier-contracts` works with `peek --plain`, `send <dn> hi`, `tag <dn> role=worker`, `events --recent <dn>`, `kill <dn>` (:377-432).
- `run -d --id <n> --tag owner=forge --tag env=staging -- cat`: metadata `tags` equals `{owner:"forge",env:"staging"}` (tags.test.ts:256-280). `run -d --id bad-tag --tag no-equals-sign -- cat`: status != 0, stderr contains `key=value` (:375-390). `run -a -d --id <exited> -- cat` keeps previous tags `{owner:"ci",keep:"true"}` (:314-341); `run -a -d --id <exited> --tag owner=new -- cat` replaces them with `{owner:"new"}` (:343-373).
- restart-launch-parity: `run -d --id run-a-unset --no-display-name --tag keep=true --unset-env NO_COLOR -- true` then, after exit, `run -a -d --id run-a-unset --no-display-name -- sh -c '...'`: child sees `NO_COLOR` unset; metadata `unsetEnv` equals `["NO_COLOR"]` (:73-104). `run -d --id unset-parity --no-display-name --tag keep=true --unset-env NO_COLOR --env ASSIGNMENT_WINS=explicit --unset-env ASSIGNMENT_WINS -- sh -c ...`: child records `|explicit`; `unsetEnv` equals `["NO_COLOR","ASSIGNMENT_WINS"]`, `extraEnv` equals `{ASSIGNMENT_WINS:"explicit"}` (:106-126). `run -d -e --id launch-parity --name "Launch Parity" --tag keep=true --tag role=service --cwd <cwd> --isolate-env --env ST_AGENT=managed-first --env CATALOG=/managed/catalog --env PTY_SESSION=must-not-win --env ST_AGENT=managed-final -- sh -c ...`: child records `managed-final|/managed/catalog|<name>|<cwd>`; metadata `ephemeral === true`, `isolateEnv === true`, `extraEnv` equals `{ST_AGENT:"managed-final",CATALOG:"/managed/catalog",PTY_SESSION:"must-not-win"}`, `tags` matches `{keep:"true",role:"service"}`, `displayName === "Launch Parity"`, `cwd`, `rows > 0`, `cols > 0` (:141-175). Fresh `run -d --id fresh -- sh -c ...` inherits `ST_AGENT`/`ST_ROOT` (`creator-abc|/creator/convoy`) (restart-env-scrub.test.ts:83-90).
- exit-signal: `run -d --id sk -- sh -c "exec sleep 300"`, SIGKILL the leaf: metadata `exitCode === 137`; `session_exit` event has `exitCode 137`, `signal 9` (:49-72). `run -d --id ce -- sh -c "exit 5"`: `exitCode 5`, no `signal` field in metadata or event (:74-87).

### 6.3 `pty attach` (attach-no-restart, attach-stream, nesting-prevention tests)

- `attach --help`: status 0, stdout contains `--no-restart`, matches `/never prompt/`, contains `--attach-stream-fd-v1 <fd>`, matches `/GEOMETRY.*SCREEN.*DATA.*EXIT/s` (attach-no-restart.test.ts:134-140; attach-stream.test.ts:98-103).
- `attach --no-restart --auto-restart missing`: status != 0, stderr matches `/mutually exclusive/` (:142-147). `attach --no-restart missing`: status != 0, stderr contains `Session "missing" not found.`, stdout lacks `Restart?` (:149-155).
- Exited session (`run -d --id <n> --tag keep=true -- sh -c "...; exit 42"`), `attach --no-restart <n>` in a PTY with delayed input: code != 0; output lacks `Restart?` and `Command was:`; no re-launch; `session_start` count stays 1 (:157-179). Hand-written vanished metadata (`command:"sh"`, `args`, `displayCommand:"synthetic stored command"`, `cwd`, `createdAt`, `tags:{keep:"true"}`), `attach --no-restart vanished-target`: code != 0, output contains `is not running` and `vanished`, lacks `Restart?` and `synthetic stored command` (:181-206).
- Running session, `attach --no-restart running-target` in node-pty; after typing `finish\r` the session exits 37: attach exit code 37, output contains `running-target exited with code 37`; no second incarnation (:208-255). `attach legacy-target` (default policy) on an exited session: output contains `Restart? [Y/n]`, then the session is re-launched (invocation count 2) and `session_start` count is 1 (new log) (:257-273). `attach --auto-restart automatic-target`: no `Restart?`, re-launched (:275-286).
- Nested: `attach <t>` with `PTY_SESSION=outer-session`: status != 0, stderr contains `already inside pty session "outer-session"` and matches `/--force/i` (nesting-prevention.test.ts:110-119); dead session + `attach -r <t>`: refused before any prompt (stdout lacks `Restart?`) (:121-142); `attach --force <bogus>`: stderr lacks `already inside pty session`, matches `/not found/`; `--force` may precede or follow `-r` (:144-174).
- Stream fd mode (attach-stream.test.ts): `attach --attach-stream-fd-v1 999999 missing`: status != 0, stderr matches `/attach-stream-fd-v1.*999999.*not writable/i`, stderr lacks `Session "missing"`, stdout `""` (:105-115). `attach --attach-stream-fd-v1` (no value): status != 0, stderr matches `/attach-stream-fd-v1 requires a file descriptor/`, stdout `""` (:117-124). Real session via `bin/pty attach --attach-stream-fd-v1 3 <name>` with fd 3 a pipe: status 0, stdout and stderr are **empty buffers**, packets `[GEOMETRY, SCREEN(contains LAUNCHER_READY), ..., EXIT]` (:126-181); sending byte `0x1c` after SCREEN -> last packet `DETACH`, no `EXIT`, status 0 (:183-239); detach before any daemon data -> stream is exactly `[DETACH]` (:241-269); EXIT arriving within the 300 ms detach window wins: stream `[GEOMETRY, SCREEN, EXIT]`, status 0 not 42 (:271-325); fragmented/coalesced input is reframed in order `[GEOMETRY, SCREEN, DATA, EXIT]`, geometry `{rows:31,cols:97}`, payloads byte-exact (:327-351); ATTACH geometry is the stdout TTY size (`{rows:27,cols:91}`) and nothing is painted to the TTY (:353-402); daemon sending SCREEN first: status != 0, stream empty, stderr matches `/daemon does not support attach stream v1/i` (:404-413); GEOMETRY then DATA/EXIT before SCREEN: status 1, stderr matches `expected SCREEN before DATA|EXIT`, stream `[GEOMETRY]` (:415-432); close without EXIT: status 1, stderr `/machine stream truncated before EXIT: connection closed/i`, stream `[GEOMETRY, SCREEN]` (:434-446); TCP reset: status 1, `/machine stream truncated before EXIT/i` and NOT `/session .* not found/i` (:448-459); broken fd 3 under a 1 MiB DATA: status 1, stderr `/machine stream descriptor 3 failed.*EPIPE/i` (:461-493); reconnect keeps one stream: `[GEOMETRY, SCREEN, DATA, GEOMETRY, SCREEN, DATA, EXIT]`, second geometry `{rows:21,cols:71}`, stderr matches `/reconnecting/`, stdout empty (:495-561); reconnect that sends DATA before SCREEN: status 1, `/expected SCREEN before DATA/i`, stream `[GEOMETRY, SCREEN, DATA, GEOMETRY]` (:563-618).

### 6.4 `pty exec` (exec.test.ts; env `PTY_SESSION=<name>`, `PTY_SESSION_GENERATION=<metadata.generation>`)

- `exec -- echo hello-from-exec`: status 0, stdout contains `hello-from-exec`; metadata `displayCommand === "echo hello-from-exec"`, `args` equals `["hello-from-exec"]` (:112-131). Without `PTY_SESSION`: status != 0, stderr contains `not inside a pty session` (:133-146). Session tagged `ptyfile`: status != 0, stderr contains `pty.toml` (:148-164). Tags `{role:"dev",strategy:"permanent"}` preserved (:166-181). `exec -- sh -c "exit 42"`: status 42 (:183-195). Bare `exec` with `PTY_SESSION=test`: status != 0, stderr contains `Usage` (:197-206). `exec -- echo swapped`: a `session_exec` event with `session`, `command === "echo swapped"`, `previousCommand` defined (:208-227). `exec -- /nonexistent/cmd`: status != 0, stderr contains `not found` (:229-242). Metadata lock held by another process: status != 0, stderr contains `busy`, metadata untouched (:244-275). `PTY_SESSION_GENERATION` of an old generation: status != 0, stderr contains `replacement generation`, command not run, metadata untouched, no `session_exec` event (:277-304).

### 6.5 `pty peek` (peek-wait.test.ts, nesting.test.ts)

- `peek --plain <n>` vs `peek --plain --full <n>` on a session that printed 100 lines: full has more lines, >= 100, contains `line1` and `line100` (:90-108).
- `peek --wait READY -t 5 --plain <n>`: status 0, stdout contains `READY` (:111-121). `peek --wait NEVER -t 1 --plain <n>` on `cat`: status 1, stderr contains `Timed out` and `NEVER` (:123-133). Text already on screen: status 0 in < 2000 ms (:135-148). `peek --wait FIRST --wait SECOND -t 5 --plain <n>` where only `SECOND` printed: status 0 (:150-160). Exited session (`keep=true`) that printed `TEST_PASSED`: `peek --wait TEST_PASSED -t 5 --plain <n>` -> status 0, stdout contains it (:162-174); pattern absent: status 1, stderr contains `exited` and `MISSING` (:176-188). `peek --plain <n>` on an exited keep session: status 0, stdout contains `SAVED_OUTPUT` (:190-201).

### 6.6 `pty send` (send-paste.test.ts, seq-delay.test.ts)

- Raw bytes captured via `stty raw -echo; cat > file`: `send <n> --paste hello-paste` -> exactly `\x1b[200~hello-paste\x1b[201~`, status 0 (:121-132); `send <n> post-paste --paste` -> `\x1b[200~post-paste\x1b[201~` (:134-145); `send <n> --paste --seq "first " --seq "second " --seq third` -> `\x1b[200~first second third\x1b[201~` with exactly one start and one end marker (:147-166); `send <n> --with-delay 0.05 --paste --seq A --seq B` -> `\x1b[200~AB\x1b[201~` (:168-186); `send <n> plain-text` -> `plain-text`, no markers (:188-201); `send <n> --paste "line-one\nline-two\n"` -> markers around the literal newlines (:203-219).
- `send somename "hello world" --bogus` (no daemon): status != 0, stderr contains `Unexpected argument` and `--bogus` (:225-231). `send somename "sudo cmd" --enter`: status != 0, stderr contains `--enter`, `--seq`, `key:return` (:233-240); same for `--newline`, `--return`, `--cr` (:242-250). `send <n> still-works` -> bytes `still-works` (:252-263).
- `send <n> --with-delay 0 --seq key:ctrl+u --seq key:ctrl-u --seq key:ctrl_u --seq key:C-u` -> `\x15\x15\x15\x15`, status 0 (:267-290). `send <n> --with-delay 0 --seq PARTIAL --seq key:ctrl- --seq AFTER`: status != 0, stderr matches `/Incomplete key spec.*ctrl-u.*supported keys/is`, **nothing delivered** (:292-314).
- Timing: `DEFAULT_SEQ_DELAY_MS === 300`; `resolveSeqDelayMs(undefined) === 300`, `(0) === 0`, `(0.1) === 100`, `(0.5) === 500`, `(2) === 2000` (seq-delay.test.ts:15-28). Nine `--seq` items: default run minus `--with-delay 0` run > 1200 ms (:83-94); `--with-delay 0.2` minus `0` > 900 ms (:96-106).

### 6.7 `pty events` / `pty emit` (peek-wait.test.ts, events-emit.test.ts, metadata-events.test.ts)

- `events --wait bell -t 10 <n>` after the session prints `\x07`: status 0, stdout contains `bell` (peek-wait.test.ts:205-215). `events --wait bell -t 1 <n>` on `cat`: status 1, stderr contains `Timed out` (:217-226).
- `emit <n> user.tests-passed --json '{"count": 42}'`: status 0; last event `type user.tests-passed`, `data` equals `{count:42}` (events-emit.test.ts:138-151). `emit user.from-inside` with `PTY_SESSION=<n>`: status 0, event written (:153-163). `emit <n> bogus-type`: status != 0, stderr matches `/must start with/` (:165-173). `emit user.whatever` with no `PTY_SESSION`: status != 0, stderr matches `/not running inside a pty session|no session ref/` (:175-187). `emit <n> user.note --text "checkpoint reached"`: `text` set, `data` undefined (:189-202). `--json '{"ok":true}' --text done`: both fields (:204-217). `emit <n> user.bad --json "{not-valid-json"`: status != 0, stderr matches `/not valid JSON|--json/`, no event appended (:219-233). 1200 emits leave <= 1000 lines and the tail (`"i":1199`) intact (:237-262). Type validation messages: `/must start with/` for `build-done`, `session_start`, `state.set`; `/suffix/` for `user.`; `/non-empty/` for `""`; `/whitespace/` for `user.has space`, `user.tab\tfoo` (:92-110).
- `formatEvent` pins: `display_name_change` output contains `display_name ->`, `"new"`, `"old"` (JSON-quoted), and `null` when cleared; `tags_change` contains `tags ->`, `role=web`, `owner=forge`, `{}` for empty; `metadata_change` contains `metadata ->`, `"Worker"`, `"worker"` (metadata-events.test.ts:632-692).

### 6.8 `pty list` / `ls` (list-filters, tags, list-purity, up-down, gc-flap-clear-badge tests)

- Fabricated `<n>.json` without `.sock`/`.pid` and without `exitedAt`/`exitCode`: `list --json` element `status "vanished"`, `exitCode null`, `exitedAt null` (list-filters.test.ts:119-132); text `list` contains `Vanished sessions` and the name (:134-143). Clean exit (`true`, keep): `status "exited"`, `exitCode 0`, `exitedAt` non-null (:145-159). `.pid` = live pid, no `.sock`, 48 h-old `createdAt`: `status "running"`, file kept (:167-191); dead pid `2147483647`: `vanished`, `.pid` and `.json` kept (:193-210).
- `list --json --status running|exited|vanished` return exactly the matching names (:214-237); `list --status bogus`: status != 0, stderr contains `--status expects` (:239-244). `list --json --older-than 1h` / `--newer-than 1h` (:248-273); `list --older-than 1week`: status != 0, stderr contains `duration` (:275-280); `list --json --older-than 1h --filter-tag env=prod` composes (:282-294).
- `list --summary` with one exited (createdAt 2 h ago) and one vanished (now): stdout contains `2 sessions`, `1 exited`, `1 vanished`, `oldest: <old>`, `newest: <recent>` (:298-316). `list --json --summary`: `{total:1, byStatus:{vanished:1, exited:0, running:0}, oldest:{name,status:"vanished",ageSeconds>=295}, newest:{name}}` (:318-335); with `--status vanished` (:337-354); `list --summary --status running` on none: `No matching sessions.` (:356-362).
- Sort: `list --json` order by `displayName ?? name`: `["bbb-friendly","bbb-raw","mmm-friendly","zzz-raw"]` (:386-403); text buckets ordered `a1` < `m1` < `z1` (:405-423); displayName `zebra` on id `aaa` sorts after id `mmm` (:425-437).
- Tags: `list --json` element `tags` equals `{owner:"myapp"}` (tags.test.ts:118-128); `list --json --filter-tag layout=work` (:130-140) and two `--filter-tag` AND (:142-152); text `list` shows `#role=web` `#env=dev` (:154-163); hides `#ptyfile=`, `#ptyfile.session=`, `#ptyfile.tags=`, `#strategy=` but shows `[permanent]` (:165-184); `list --tags` shows `#ptyfile=` and `#strategy=permanent` (:186-199); `:l1234-abc` / `:layout` hidden by default, shown as `#:l1234-abc=1` / `#:layout=grid` with `--tags` (:201-218); `list --filter-tag layout=work` filters text output (:220-230). Untagged sessions have no `tags` field in metadata (:232-240).
- `[flapping]` shown and `[permanent]` hidden for `strategy.status=flapping`; `[permanent]` otherwise (gc-flap-clear-badge-root-len.test.ts:118-158).
- `ls --json` array with a `name` field (rm-kill-ephemeral.test.ts:230-250). `list --json` exposes `cwd` and `displayName` (gc-permanent.test.ts:292-294, 358-361). `listSessions()` never creates the root, never deletes stale sockets/pids or corrupt metadata (list-purity.test.ts:50-79).

### 6.9 `pty stats` (stats-cli.test.ts, display-name.test.ts)

- `stats <n>` (running `cat`): stdout contains `Session: <n>`, `Terminal:`, `Scrollback:`, `Clients:`, `Process:`, `Modes:`, `running`, `CPU:`, `Memory:`, `Daemon:` (:108-125). `stats --json <n>`: `name`, `terminal.cols 80`, `terminal.rows 24`, `terminal.scrollbackCapacity 10024`, `process.alive true`, `process.pid` number, `process.resources.rssKb/cpuPercent` numbers, `daemon.pid` number, `daemon.resources.rssKb` number, `clients`, `modes` defined (:127-151); rss > 0, cpu >= 0, pids > 0 and distinct (:189-209). `stats` (no ref) lists `Session: <n1>` and `Session: <n2>` (:153-164). `stats nonexistent`: non-zero (:166-175). Exited keep session: stdout contains `exited`, no `CPU:`/`Memory:` (:177-187, 211-222). `stats <displayName> --json` returns `name` = stable id (display-name.test.ts:315-326); exact id beats a same-text displayName (:328-338).

### 6.10 Ambiguity (display-name.test.ts:340-367)

With two sessions `--name shared` (`alpha`, `beta`), each of `attach shared`, `peek --plain shared`, `send shared hello`, `stats shared --json`, `events --recent shared`, `restart -y shared`, `kill shared`, `tag shared role=test`, `tag-multi shared role=test`, `emit shared user.test`, `rename --show shared`, `rename --clear shared`, `rename shared renamed`, `rm shared`: status != 0, stderr contains `Session reference "shared" is ambiguous.` and both `alpha` and `beta`.

### 6.11 `pty rename` / `pty metadata patch` (display-name, metadata-events tests)

- `rename webapp my-label`: status 0, stdout contains `my-label`, metadata updated (display-name.test.ts:215-226). `rename --show api` -> stdout trimmed is exactly `friendly-api` (:228-237); without a displayName stdout contains `no displayName` (:239-247). `rename --clear svc`: status 0, displayName removed (:249-259). `rename only-one` outside a session: status != 0, stderr contains `only allowed inside a pty session` (:261-266). `rename aaa bbb` (bbb is another id): allowed (:268-278); `rename same same` allowed (:280-287). Inside (`PTY_SESSION=insider`): `rename from-inside` sets it (:291-300); `rename --clear` clears it (:302-311). `rename <n> friendly` appends a `display_name_change` event with `value "friendly"` (metadata-events.test.ts:476-487).
- `metadata patch --id <n>` with stdin `{"displayName":"CLI Worker","tags":{"role":"worker"}}`: status 0; stdout JSON matches `{changed:true, metadata:{displayName:"CLI Worker", tags:{role:"worker"}}}`; exactly one `metadata_change` event appended (:371-388). `metadata patch --id missing-id` when only a displayName matches: status != 0, stderr contains `Session id "missing-id" not found` (:390-406). `metadata patch` (no --id, stdin `{}`): stderr `/missing required --id/`; `--id target` with stdin `not-json`: `/invalid JSON on stdin/`; stdin `[]`: `/Metadata patch must be a JSON object/`; all status != 0 (:408-417). Library: invalid patches (` Worker`, `Worker\u2028Next`, `Worker\u2029Next`, 161 emoji -> `/Invalid displayName/`; `{"":"value"}` -> `/tag keys must be non-empty/`; `{role:1}` -> `/tag values must be strings or null/`; `{unknown:true}` -> `/unknown field "unknown"/`) write nothing (:339-357); `metadata_change` `previous`/`value` carry only touched keys with `null` for absent (:263-308); no-op -> `changed:false`, no event (:310-325); event-lock held -> rejects `/event log is busy/i` (:146-167).

### 6.12 `pty tag` / `pty tag-multi` (tag-mutate, tag-bulk, tag-multi, exit-reap tests)

- `tag <n> role=server env=dev`: status 0, stdout contains `role=server` and `env=dev`, metadata `tags` equals `{role:"server",env:"dev"}` (tag-mutate.test.ts:95-107); `tag <n> role=new` updates (:109-118); `tag <n> --rm env` (:120-130); removing the last tag deletes the `tags` field (:132-141); `tag <n>` prints current tags (:143-151); `No tags` when empty (:153-160); works on exited keep sessions, e.g. `tag <n> strategy=permanent` -> `{keep:"true",strategy:"permanent"}` (:162-175); `tag nonexistent foo=bar`: status != 0, stderr contains `not found` (:177-183). `tag <n> keep=true` on a running session makes its exit metadata survive (exit-reap.test.ts:772-786).
- tag-bulk: multiple `k=v` in one call; `color=red color=blue` -> `blue`; merges with existing; `key=` -> `""`; `foo=bar=baz` -> `{foo:"bar=baz"}`; 30 keys at once; `--rm a --rm c`; `--rm never-was-set` status 0 no-op; `--rm dup --rm dup` -> tags field removed; `added=new --rm drop`; `k=v --rm k` -> removed; position independence (`fresh=1 --rm drop another=2` == `--rm drop another=2 fresh=1`); `x=new --rm y z=new --rm x` -> `{z:"new"}` (tag-bulk.test.ts:119-288). Errors: `tag <n> no-equals-here` -> status != 0, stderr matches `/key=value|--rm/`, nothing written; `tag <n> =value` -> `/key/i`; `tag <n> --rm` -> `/--rm/`, tags untouched; `tag <n> --rm ""` -> `/key/i`; `tag no-such-session k=v` -> `/not found/`; `tag <n> good=yes no-equals` -> nothing written (:292-356). Events: `tag <n> a=1 b=2 c=3 --rm z` -> exactly one `tags_change` with `previous {}` and `value {a:"1",b:"2",c:"3"}`; no-op writes emit nothing; `same=x new=y` -> one event with both (:360-411). `tag <n>` dump contains `role=web`; empty matches `/No tags/`; displayName refs write to the stable-id file (:415-445).
- tag-multi read: `tag-multi <a>` stdout contains `<a>` and `role=web`; `tag-multi <a> <b>` both; `--json` -> `{"<a>":{role:"web"},"<b>":{env:"dev"}}`; untagged -> `{"<a>":{}}`; displayName refs keyed by stable id; unresolvable name -> status != 0, stderr `/not found|no-such-session/` (tag-multi.test.ts:120-190). Selectors: `--filter-tag role=web --json` keys = matching ids; two filters AND; zero matches -> status 0 `{}`; `--all --json` all ids; empty dir -> `{}` (:198-253). Write: `tag-multi <a> <b> audit=2026-04-25`; `--rm role` (a keeps `{env:"prod"}`, b loses `tags`); `fresh=1 --rm drop`; one `tags_change` per session; no-op session emits none; unresolvable name -> no writes; displayName refs (:261-356). `--filter-tag role=web audit=...` writes only to matches; zero matches -> status 0 no writes; `--all role=web` -> status != 0, stderr `/--yes/`, nothing written; `--all --yes stamped=1` and `--all -y role=web` apply to all (:364-423). Mutex: `--all --filter-tag k=v`, `--all <a>`, `--filter-tag k=v <a>` -> status != 0, stderr `/mutually exclusive|pick one/i`; bare `tag-multi` and `tag-multi role=web` -> `/selector/i` (:431-470). Ops errors mirror `tag`: `=value` -> `/key/i`; trailing `--rm` -> `/--rm/`; `--rm ""` -> `/key/i`; `--filter-tag` alone -> `/--filter-tag|k=v/i`; `--filter-tag no-equals` -> `/filter|k=v/i`; `foo=bar=baz`; `k=v --rm k` (:478-540). Mixed read `{a:{role:"web"}, b:{}, c:{env:"prod"}}`; empty-write appends no events; `--all --json` equals per-name read (:548-587).

### 6.13 `pty kill` / `pty rm` / exit reaping (kill-wait, rm-kill-ephemeral, rm-immediate-reuse, exit-reap tests)

- `kill kw` on a running session: status 0, stdout contains `killed`, daemon pid dead when the command returns (kill-wait.test.ts:40-52); `kill kw2` then `rm kw2`: both status 0, no files starting with `kw2` remain (:54-65); after out-of-band metadata unlink + SIGTERM, no `kw3.json` or `kw3.json.tmp.*` reappears (:67-90).
- `kill <n>`: stdout contains `killed`; `.json` kept, `.sock` removed (rm-kill-ephemeral.test.ts:123-138). Exited session: status != 0, output contains `not running` (:140-151). `kill nope`: status != 0, output contains `not found` (:153-158). SIGSTOPped daemon: status != 0, output contains `daemon PID <pid> is still running after 7s` and `<n>.sock may still be owned` (:160-171).
- `rm <n>` exited: stdout contains `removed`, all files gone (:178-194). Running: status != 0, `still running` (:196-204). `rm nope`: `not found` (:206-211). Ephemeral (`ephemeral:true`) sessions leave no files after exit (:217-228) and disappear from `ls --json` (:241-251).
- rm-immediate-reuse (env `PTY_ROOT=<dir>`, `PTY_SESSION_DIR=""`): loop 5x: `run -d --id reuse --tag keep=true -- sh -c "sleep 0.05; exit 0"` status 0; wait for `exitedAt`; metadata `daemonPid` > 0 and `generation` non-empty; `rm reuse` status 0, stdout contains `removed`, old daemon dead; `run -d --id reuse -- cat` status 0 with a new `generation`; 650 ms later `.sock` exists, `.pid` is the replacement pid, generation unchanged; `kill reuse`, `rm reuse` status 0 (:112-162).
- exit-reap (shipped default, `PTY_REAP_ON_EXIT` unset): non-permanent sessions exiting 0 or 3 leave no `<n>*` files, including events; `gc` afterwards prints `Nothing to clean up.` and no `<n>` (:670-715). With daemon env `PTY_REAP_ON_EXIT=false`: `.json` survives; `gc` prints `Removed: <n>` and sweeps it; `ephemeral:true` still reaps (:718-755). Exemptions: `keep=true` retains; `keep=false` does not; `strategy=permanent` retains; `kill` retains `.json` without `.sock`; permanent + ephemeral reaps; keep beats ephemeral; `rm` removes a kept session (stdout `removed`) (:758-872). SIGKILLed daemon leaves `.json`; `gc` prints `Removed: <n>`; a killed session is swept by the next `gc` (`Removed: <n>`); kept sessions print `Kept (keep tag): <n>` and stay (:875-932).

### 6.14 `pty evidence` (exit-reap.test.ts:203-666; env `PTY_ROOT`, `PTY_ROOT_LEGACY_SILENT=1`)

- Session `sh -c "printf '<out>\n'; printf '<err>\n' >&2; exit 23"` with `keep=true`: `evidence snapshot --id <n>` -> status 0, stderr `""`, JSON `{_tag:"snapshot", snapshot:{name, status:"exited", exitCode:23, stream:"combined", tail:{_tag:"present", lastLines:[...]}}}` where `lastLines` equals the metadata `lastLines` and contains both stdout and stderr sentinels; `evidence remove --id <n> --expected-generation <gen>` -> `{"_tag":"removed"}`; then snapshot -> `{"_tag":"unavailable","reason":"missing"}`; against a live replacement: old generation -> `{"_tag":"generation-mismatch"}`, current generation -> `{"_tag":"not-terminal"}`, files intact (:204-286).
- Invalid argv (each: status != 0, stderr non-empty, stdout `""`): `evidence`; `evidence snapshot`; `evidence snapshot --id`; `evidence snapshot --id one --id two`; `evidence snapshot --id one unexpected`; `evidence snapshot --id one --expected-generation gen`; `evidence remove --id one`; `evidence remove --id one --expected-generation`; `evidence remove --id one --expected-generation gen --expected-generation gen2`; `evidence unknown --id one` (:288-304). Unsafe ids `/absolute`, `../traversal`, `nested/path`, `.`, `..` likewise; `normal.dotted-task-id` is accepted and yields `{"_tag":"unavailable","reason":"missing"}` (:642-666).
- Vanished metadata (`generation`, `daemonPid:2147483647`, no exit fields) -> `{_tag:"snapshot", snapshot:{name, generation:"vanished-generation", status:"vanished", exitCode:null, stream:"combined", tail:{_tag:"unavailable"}}}` (:369-394); `lastLines: []` -> `tail:{_tag:"present", lastLines:[]}` (:397-423); missing `generation` -> `{_tag:"unavailable", reason:"generation-unavailable"}` and remove -> `generation-mismatch` (:426-448); a directory at `<n>.events.jsonl` makes `remove` fail (status != 0, stdout `""`) while `.sock`/`.pid` are gone and `.json` retained (:451-502); `invalid-metadata` for numeric/empty `generation`, string `exitCode`, missing `exitedAt`, non-string `lastLines` entries, 201 `lastLines`, malformed JSON, > 2 MiB padding, directory, symlink (:505-609).

### 6.15 `pty gc` (gc, gc-abandoned, gc-flapping, gc-parent-child, gc-permanent, gc-flap-clear-badge, pty-root tests)

- Vanished session: `gc` status 0, stdout contains `Removed: <n>`, `.json` gone (gc.test.ts:121-135; :268-286 with synthetic metadata). Dead `:l<pid>-abc` tag: `Pruned orphan tags on <n>: #:l<pid>-abc`, `role` kept (:137-160); live `:l<pid>-xyz` kept, not mentioned (:162-177); `:layout`/`:other` untouched (:179-196). Nothing to do: `Nothing to clean up.` (:198-206). `gc --dry-run`: `Would remove: <n>` and `Dry run`, file kept; then `gc` -> `Removed: <n>` (:208-229); `Would prune orphan tags on <n>: #<key>` then `Pruned ...` (:231-253); `gc -n` alias (:255-266).
- `gc --print-launchd-plist`: status 0; contains `<!DOCTYPE plist`, matches `/<string>com\.compoundingtech\.pty\.gc(?:\.[A-Za-z0-9._-]+)?<\/string>/`, contains `<key>StartInterval</key>`, `<integer>30</integer>`, `<key>PTY_ROOT</key>` (:288-301); via `bin/pty` contains `<string><binPath></string>` (:303-312); `--interval=15` -> `<integer>15</integer>` (:314-319); `--interval=0` and `--interval=abc` -> status != 0 (:321-327). Label/logPath per root: default root -> `<string>com.compoundingtech.pty.gc</string>` and no `com.compoundingtech.pty.gc.`; root basename `my-network` -> `...gc.my-network`; `<string><root>/gc.log</string>`; `<key>PTY_ROOT</key>` present and `<key>PTY_SESSION_DIR</key>` absent; basename `weird name with spaces` -> `...gc.weird-name-with-spaces` (pty-root.test.ts:146-194).
- Abandoned: permanent session with deleted cwd -> `Abandoned: <n> (cwd-gone)`, files gone, `session_abandoned` event with `reason "cwd-gone"` if the log survives (gc-abandoned.test.ts:153-183); non-permanent not reaped (:185-199); `strategy.abandon-if-cwd-gone=false` opt-out (:201-218); `gc --dry-run` -> `Would abandon: <n> (cwd-gone)` + `Dry run`, nothing touched (:220-236); `gc --idle-days 14` with `lastAttachAt` 30 d old -> `Abandoned: <n> (idle <N>d)` (regex `Abandoned: <n> \(idle \d+d\)`) (:238-257); 3 d old -> not reaped (:259-275); no `lastAttachAt` -> not reaped (:277-289); tag `strategy.idle-days=10` without a flag reaps (:291-310); cwd-gone wins over idle (:312-329); `gc --idle-days 0` and `gc --idle-days=-5`: status != 0, stderr contains `--idle-days expects a positive integer` (:331-340); abandoned + respawned in one pass: `Abandoned: <a> (cwd-gone)` and `Respawned: <b>` (:344-370). Library: lock held -> `reapSkipped` `{name, operation:"abandoned", reason:"busy", signalled:false}` (:123-146); orphan variant `operation:"orphan"` (gc-parent-child.test.ts:92-114).
- Flapping (synthetic metadata; counter/last-respawn-at tags): `gc --dry-run` below limit -> `Would respawn: <n>`, no `Would flap`, tags unchanged (gc-flapping.test.ts:118-138); at limit -> `Would flap: <n> (3 fast-fails in 60s, limit 3)`, no respawn, no mutation (:140-160); real `gc` at limit -> `Flapping: <n> (3 fast-fails in 60s, limit 3)`, tags `strategy.status=flapping`, `strategy.consecutive-fast-fails=3`, `session_flapping` event `{counter:3, limit:3, window:60}` (:162-186); already flapping -> `Skipped (flapping): <n>`, no `Respawned:`, no new event (:188-207); slow fail resets (:209-226); command-hash change resets and clears the mark, no `Skipped (flapping)` (:230-251); tag `strategy.fast-fail-limit=2` -> `Would flap: <n> (2 fast-fails in 60s, limit 2)` (:253-269); `gc --dry-run --fast-fail-limit=10` lifts the limit (:271-286); tag `strategy.fast-fail-window=10` (:288-305); no prior `last-respawn-at` -> respawn (:307-319). `restart -y <n>` on a flapping-exited session drops `strategy.status`, `strategy.consecutive-fast-fails`, `strategy.last-respawn-at`, `strategy.command-hash` and keeps `strategy=permanent` and `role` (gc-flap-clear-badge-root-len.test.ts:84-114).
- Parent/child: dead (vanished) parent -> `Killed orphan child: <child> (parent <parent>` ... and `.json` gone (gc-parent-child.test.ts:121-138); missing parent -> `Killed orphan child: <child> (parent nonexistent-parent missing)` (:140-151); live parent preserved (:153-167); cycle A<->B both killed (:169-190); `parent` + `strategy=permanent` -> killed, no `Respawned: <child>` (:192-211); `gc --dry-run` -> `Would kill orphan child: <child> (parent nonexistent-parent missing)` + `Dry run` (:213-224).
- Permanent respawn: `gc` -> `Respawned: <n>`; respawned child keeps `unsetEnv`/`extraEnv` (`|explicit\n|explicit\n`); metadata `unsetEnv` equals `["NO_COLOR","ASSIGNMENT_WINS"]` (gc-permanent.test.ts:124-170); `session_respawn` event written; new pid (:172-208); vanished non-permanent -> `Removed: <n>`, no `Respawned:` (:210-226); `gc --dry-run` -> `Would respawn: <n>` + `Dry run`, no `.pid` (:228-244); exactly one `^Respawned: ` line per invocation (:246-267); pty.toml-managed: `up <dir>`, then the toml edited, `gc` -> `Respawned: <id> (pty.toml re-read)` and the new command runs; `list --json` element with `displayName "perm"`, metadata `tags.ptyfile === <dir>/pty.toml` (:269-337); per-session `cwd` honored on respawn (`found.cwd === runDir`) (:339-390).

### 6.16 `pty restart` (nesting-prevention, restart-guardrail, restart-env-scrub, restart-launch-parity, display-name, tags tests)

- `restart -y <t>` with `PTY_SESSION=outer-session`: status 0, stdout contains `Session "<t>" restarted.` and matches `/not attached.*outer-session/` (nesting-prevention.test.ts:178-187); `restart -y --force <t>`: no `/not attached/` (:189-199); without `PTY_SESSION`: `restarted.` and no `/not attached/` (:201-209).
- `restart -y ag` on `--tag role=agent`: status != 0, stderr matches `/stateful agent/`, `/role=agent/`, `/--force/`, `/convoy/` (restart-guardrail.test.ts:36-46); `claude --resume ABC-123` command: `/stateful agent/`, `/claude --resume/` (:48-57); plain session: status 0, stdout contains `restarted`, no `/stateful agent/` (:59-67); `restart -y --force ag2`: no `/stateful agent/`, stdout `restarted` (:69-79).
- `restart -y s` with `ST_AGENT=smalltalk-claude`, `ST_ROOT=/leaked/convoy`: status 0, stdout `restarted`; child sees `UNSET|UNSET` (restart-env-scrub.test.ts:62-81).
- `restart -y unset-parity` / `launch-parity` / `toml-parity`: status 0, stdout contains `restarted`; the child re-records identical env/cwd output; `restartShape` (`command,args,displayCommand,cwd,rows,cols,ephemeral,tags,displayName,isolateEnv,extraEnv,unsetEnv`) is deep-equal before/after (restart-launch-parity.test.ts:106-242). `up <project>` with the toml `[sessions.worker]` (`id`, `display_name = "TOML Worker"`, `cwd = "work"`, tags, env): child sees `from-toml|<name>|<project>/work`; metadata `displayName "TOML Worker"`, `displayCommand` contains `TASK_VALUE`, `cwd`, tags match `{keep:"true",role:"worker","ptyfile.session":"worker"}`, `extraEnv` equals `{TASK_VALUE:"from-toml",PTY_SESSION:"must-not-win"}` (:191-242).
- `restart -y svc` (with `PTY_SESSION=outer`) keeps `displayName "My Service"` and `tags.role "web"`; stdout contains `restarted` (display-name.test.ts:440-466). `restart -y <n>` keeps `{owner,env,keep}` tags and rewrites `createdAt` (tags.test.ts:282-312).

### 6.17 `pty up` / `pty down` (up-down.test.ts, up-name-decouple.test.ts)

- `up <dir>` with sessions `one`,`two`: status 0, stdout contains `one (started)`, `two (started)`, `Started 2 sessions`; two running (:81-101). `up <dir> web db`: `web (started)`, `db (started)`, no `worker`; running displayNames `["db","web"]` (:103-128). `[sessions.envprobe.env]` -> child sees `hello|world`; stdout `envprobe (started)` (:130-154). Second `up`: `mycat (already running)`, `All sessions already running` (:156-171). Tag sync: `updated tags: strategy=permanent, role=server`; `tags.ptyfile` defined (:173-200); manual `tag manual custom=yes` survives `up` (:202-223); removed toml tag -> stdout contains `-env`, `tags["ptyfile.session"] === "remover"` (:225-257); deleted tags table -> `-env`, `-role` (:259-284); manual tags survive that (:286-311); value change -> `env=prod` and no `-env` (:313-336); unchanged -> `unchanged (already running)` and no `updated tags` (:338-353); toml tags appear in `list --json` with `ptyfile` (:355-372); `cwd === projDir` by default (:374-388); absolute `cwd` honored (:390-406); relative `cwd = ".."` from `<proj>/.convoy/pty.toml` resolves to `projDir` (:408-427); `prefix = "myapp"` -> `myapp-web (started)`, `myapp-worker (started)` (:429-451); `up <dir> web` filters by short name -> `myapp-web (started)`, no `worker` (:453-476); `tags.ptyfile === <proj>/pty.toml`, `tags["ptyfile.session"] === "tracked"` (:478-493); `up <dir> fake`: status != 0, stderr contains `Unknown session: fake` and `Available: real` (:495-507); no manifest: stderr `No pty.toml found` (:509-516); `# empty config`: `No sessions defined` (:518-526); session without command: `missing a "command" field` (:528-539).
- `down <dir>`: status 0, `alpha (stopped)`, `beta (stopped)`, `Stopped 2 sessions` (:543-565); `down <dir> stop`: `stop (stopped)`, no `keep`, one running left (:567-589); nothing started: `No sessions to stop` (:591-601).
- up-name-decouple: `up <dir>` with `prefix = "myapp"`: `name` matches `/^[a-z0-9]{6,12}$/`, `!== "myapp-web"`, `displayName === "myapp-web"`, `tags["ptyfile.session"] === "web"` (:77-97); a 90-char prefix works, `name.length < 20` (:99-118); re-running `up` matches by the tag pair (`svc (already running)`, same id) (:120-137); `id = "pinned"` -> `name "pinned"`, `displayName "svc"` (:139-153); `display_name = "My Web Server"` overrides the prefix (:155-171); `kill svc` by displayName works (:173-188).

### 6.18 `pty recover` (recovery.test.ts; all via `pty --root <root> ...`, env `PTY_SESSION=""`, `PTY_ROOT_LEGACY_SILENT=1`)

- `list --json` after unlinking `.sock`/`.pid` of a live daemon: element matches `{name, pid, status:"running"}`; `gc` keeps `.json`; `kill <n>` status 0 (:129-151). With `recovery.processStartToken` tampered: `{name, pid:null, status:"vanished"}` (:153-170). Non-private root (0755): metadata has no `recovery` (:189-194).
- `recover <n> --snapshot <file>`: status 0 after a registry unlink; `queryStats(n).daemon.pid` unchanged; `generation`, `processStartToken`, `launchIdentity` unchanged; `recovery.secret` and `metadataRevision` rotated; the provider was not relaunched (marker has 1 line); events file has exactly one `"session_start"` line; attached clients keep streaming (:335-370). Stale snapshot after `tag <n> role=current strategy=permanent` + `rename <n> "Current Display"`: status != 0 and nothing recreated; current snapshot: status 0 with tags/displayName/lastAttachAt preserved (:372-399). A stale recovery lock from a killed recoverer is resumed (status 0, lock removed) (:401-449). Root or `.recovery` chmod 0755: status != 0, no request/result/lock/sock/pid/json written (:452-475). Tampered/unsupported/wrong-pid/wrong-generation/wrong-start/wrong-launch/wrong-secret snapshots, a foreign `<n>.lock`, and the wrong `--root`: status != 0, daemon untouched; malformed/oversized/symlink request files are deleted without following the symlink (:478-532). A foreign socket at `<n>.sock` is never replaced (status != 0; still a listening socket) (:534-552). Replaying a rotated snapshot fails; the current one succeeds (:557-570). **No stderr string for `pty recover` is pinned by tests.**

### 6.19 `pty completions` / help — see sections 3 and 4 (tests/help.test.ts, tests/completions.test.ts).
