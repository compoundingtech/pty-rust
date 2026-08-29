# pty-rust (main @ e4d6cda) — inventory, deviations from Node pty, and st2 call sites

Paths:
- Rust port: `<this-repository>` (branch main, e4d6cda)
- st2 supervisor: `<st2-checkout>` (branch pty-rust, 68ece9b)
- Node reference: `<node-pty-checkout>` (500eab2, package `@compoundingtech/pty` 0.12.0)

All line numbers are `file:line` in the repo named by the path prefix. Nothing was modified.

---

## Part A — the Rust port

Crate layout (`src/lib.rs:15-30`): package `pty-testkit` exposes modules `client, daemon, duration, input, keys, paste, protocol, ptyfile, queries, registry, screenshot, session, stats`, re-exports `Screenshot`, `Session`, `SpawnOptions`, `build_spawn_env`. The `pty` binary is `src/bin/pty.rs` (auto-discovered; no `[[bin]]` in `Cargo.toml`). 7,200 lines total (src + tests + example).

### A1. CLI (`src/bin/pty.rs`)

Dispatch (`src/bin/pty.rs:25-53`). Hand-rolled argv parser, no clap. Subcommand words and aliases:

| word(s) | handler | lines |
|---|---|---|
| `__daemon` | `cmd_daemon` | 263-337 |
| `run`, `spawn` | `cmd_run` | 58-188 |
| `ls`, `list` | `cmd_ls` | 340-404 |
| `peek` | `cmd_peek` | 407-496 |
| `send` | `cmd_send` | 499-592 |
| `attach`, `a` | `cmd_attach` | 595-619 |
| `up` | `cmd_up` | 644-709 |
| `down` | `cmd_down` | 712-738 |
| `restart` | `cmd_restart` | 771-826 |
| `rm`, `remove` | `cmd_rm` | 829-850 |
| `rename` | `cmd_rename` | 853-880 |
| `kill` | `cmd_kill` | 883-901 |
| `status`, `stats` | `cmd_status` | 922-968 |
| `version`, `--version`, `-v`, `-V` | prints `CARGO_PKG_VERSION` (bare `0.1.0`) | 39-44 |
| `help`, `--help`, `-h`, no args | `print_help` | 45-48, 19-22, 970-990 |
| anything else | `pty: unknown command '<x>'. Try \`pty help\`.` → exit 1 | 49-52 |

That is 15 words (13 user-facing commands + `__daemon` + help/version). The module doc comment (`src/bin/pty.rs:2`) still describes the original "v0 surface: run / ls / peek / send / attach / kill / status" (7 + `__daemon` = the "eight" of the original port); `up/down/restart/rm/rename/status` were added afterwards (see `git log`: 5d5c5cb, beee24f, etc.).

**Global flags: none.** There is no `--root`, no `--help` per subcommand (`pty send --help` treats `--help` as the session ref and fails "no such session"), no `-h` after a subcommand. Node has a global `--root <path>` scanned anywhere in argv (`pty/src/cli.ts:677-686`) and per-command `--help` (`cli.ts:53-57`).

**Env vars read**
- `PTY_ROOT`, then deprecated `PTY_SESSION_DIR`, then `$HOME/.local/state/pty` (`src/registry.rs:40-51`). No deprecation warning (Node warns unless `PTY_ROOT_LEGACY_SILENT`, `pty/src/sessions.ts:82-110`).
- `PTY_SESSION` — nesting guard in `run` (`src/bin/pty.rs:140-155`); also scrubbed by the testkit's `build_spawn_env` (`src/session.rs:51-70`).
- `PTY_REAP_ON_EXIT` — daemon reap default (`src/daemon.rs:98-103`).
- `HOME` — default root.
- Not read (Node reads them): `PTY_SERVER_CONFIG`, `PTY_SPAWNER_PID`, `PTY_SHUTDOWN_DEADLINE_MS`, `PTY_ROOT_LEGACY_SILENT`, `PTY_SESSION_GENERATION`.

**Exit codes**: 0 ok; 1 runtime/not-found/daemon failure; 2 usage errors (`pty run` with no command 130-133, invalid name 158-161, `send` with no args 500-503, `--with-delay` non-numeric 547-548, bad `--seq` key 562-565, `attach/restart/rm/rename/kill` missing ref); 127 when the nesting path's `exec()` fails (153-154). Node uses exit 1 for usage errors too (e.g. `cli.ts:1128`, `1387`, `1607`), so `2` is a deviation.

#### `pty run` (`src/bin/pty.rs:57-188`)
Flags parsed: `--id X`, `--name X`, `--cwd D`, `--rows R` (default 24), `--cols C` (default 80), `-d/--detach`, `--force`, `-e/--ephemeral`, `--tag k=v` (repeatable; `k=v` split on first `=`), and **accepted-but-ignored**: `-a/--attach`, `--isolate-env`, `--no-display-name` (114-117). `--` starts the command (118-121); any other token, **including any unrecognised flag**, starts the command (122-127).

Consequences / deviations vs Node run (`pty/src/cli.ts:789-832`):
- **`--env K=V` and `--unset-env K` are not parsed.** They fall into the bare-command branch, so `pty run … --env A=B -- sh -c x` makes the daemon try to exec a program literally named `--env`. The daemon's `spawn_command` fails (`src/daemon.rs:157`) before metadata is written (`:208`), the daemon exits 1, and `spawn_session_daemon` still returns Ok because "daemon exited ⇒ session ran" (`src/bin/pty.rs:254-256`), so `pty run` prints the id and exits 0 while nothing exists. See Part B — st2 always passes `--env`.
- No `-a` attach-or-create semantics (Node `cli.ts:928-941`), no `--isolate-env` semantics (Node `server.ts:136-206`), no `rows/cols/ephemeral/isolateEnv/extraEnv/unsetEnv/env` persisted to metadata (Node persists them, `sessions.ts:155-163`).
- `--rows/--cols` exist in Rust but not in Node's `run` flag list (Rust extension used by the parity loader, `tests/parity_fixtures.rs:229-235`).
- Display name: Rust sets `displayName` only when `--name` is given (`src/bin/pty.rs:175`, `:213-215`); Node auto-derives a cwd+cmd label unless `--no-display-name` (`cli.ts:960-978`) and validates it (`validateDisplayName`, `sessions.ts:67`). Rust does not validate display names.
- Foreground `pty run` (no `-d`) never attaches in Rust; it always daemonises and prints the id (`:179-182`). Node attaches unless `-d` (`cli.ts` cmdRun: prints `Session "<name>" created.` then attaches; detach path at cmdRun+99..+101).
- Success output: Rust prints the bare id on stdout; Node prints `Session "<name>" created.`.
- Duplicate id: Rust `pty run: session '<name>' already exists` exit 1 (`:162-165`, only when the socket is connectable); Node `Session id "<id>" is already in use.` exit 1 (`cli.ts:939`, whenever the name exists at all).
- Id generation: 8 chars of base36(nanos ^ pid·const) (`src/registry.rs:262-275`); Node `randomSessionName()` with 8 retries (`cli.ts:943-952`).
- Name validation: non-empty, ≤200 chars, no `/`, `\`, control chars, socket path ≤103 bytes (`src/registry.rs:239-259`). Node: `validateName` (`sessions.ts:35-65`) plus a startup root-length check (`cli.ts:688-716`).
- Nesting (`:140-155`): if `PTY_SESSION` is non-empty and neither `-d` nor `--force`, Rust `exec()`s the command in-process (cwd honoured; `PTY_SESSION` not scrubbed). `--force`/`-d` create a real nested session (the source comment at `:97-99` calls this the canonical ruling).

**Daemon launch (`spawn_session_daemon`, `:192-260`)**: re-execs `current_exe()` with `__daemon --name N --rows R --cols C --cwd D [--display-name X] [--ephemeral] [--tag k=v]* -- <cmd…>` (`:202-222`), stdio all `/dev/null` (`:223-225`), `setsid()` in `pre_exec` (`:227-233`) — no double fork, no cgroup move. Readiness poll every 30 ms up to 15 s: socket connectable, or metadata already records `exitCode` (preserve mode fast exit), or the daemon process exited (reap mode fast exit) (`:238-259`). Node instead spawns `process.execPath server.js` detached with the config JSON in `PTY_SERVER_CONFIG` (`pty/src/spawn.ts:186-203`, `server.ts:1468`).

**`pty __daemon`** (`:263-337`): parses `--name --rows --cols --cwd --display-name --ephemeral --tag`, builds `DaemonConfig` with `env: Vec::new()` (`:325`, so no extra env is ever plumbed) and `display_command = command.join(" ")` (`:321`), calls `daemon::run`.

#### `pty ls | list [--json]` (`:340-404`)
Only flag: `--json`. JSON is an array of `{name, status, pid, command, cwd, createdAt, exitCode, exitedAt[, displayName]}` (`:344-386`): `status` ∈ `running|exited|vanished`; `pid` = daemon pid from `<name>.pid` or `null`; `command` = `displayCommand`; `exitCode`/`exitedAt` explicit `null` when unset; `displayName` omitted when unset. **Missing vs Node: `tags`** — Node emits `...(tags ? {tags} : {})` (`pty/src/cli.ts:2303`). Text mode prints `NAME STATUS COMMAND` with `exited:<code>` / `dead` (`:388-403`). Missing Node flags: `--tags`, `--summary`, `--status`, `--older-than`, `--newer-than`, `--remote [peer]`, tag filters (`cli.ts:1250-1316`). Node's `status` uses `pid` from a live socket/pid; Rust's is derived in `registry::list_sessions` (see A3).

#### `pty peek` (`:406-496`)
Flags: `--plain|-p`, `--full`, `-f|--follow`, `--wait TEXT` (single pattern), `-t|--timeout SECS` (integer, default 5), `<ref>` positional anywhere. Node: multiple `--wait`, float `-t`, `--remote` (`cli.ts:1085-1091`). Behaviour:
- Plain peek requests `PEEK{plain,full}`, prints the SCREEN payload, adds a trailing newline in plain mode if missing (`:482-494`). **The daemon ignores `full`** (`src/daemon.rs:516-517`) and `capture()` already includes scrollback (see A4), so `peek`, `peek --full` are identical and both include scrollback.
- Gone session: falls back to the `<name>.screen` sidecar (`src/client.rs:23-36`); Node falls back to `metadata.lastLines` (`cli.ts` cmdPeek+53..+60). Missing sidecar → the connect error, exit 1.
- `--wait`: polls `peek(plain=true)` every 100 ms (`src/client.rs:66-78`); on timeout prints `pty peek: timed out waiting for "<needle>"` exit 1 (`:472-475`). Node also consults `lastLines` on exit (`cli.ts` cmdPeekWait+27..+43).
- `-f`: geometry-neutral ATTACH (flag byte 0x01), streams DATA/SCREEN to stdout, never forwards stdin, returns child exit code (`src/client.rs:84-115`).

#### `pty send` (`:498-592`)
Forms: literal `pty send <ref> <text…>` (extra args joined with spaces, no newline, `:584-591`; Node rejects extra args, `cli.ts:1191-1196`); `--seq <value>` repeatable with `key:<name>` resolution via `keys::parse_seq_value` (`:539-582`); `--with-delay <sec>` anywhere (`:541-552`; Node requires it first, `cli.ts:1158-1168`); `--paste <value>` which takes the **next argument** as payload and wraps it in bracketed-paste (`:514-535`) — in Node `--paste` is a boolean modifier of the positional/`--seq` data (`cli.ts:1152-1157`). Pacing: default 300 ms between items, `Math.round(n*1000)` (`src/client.rs:130-159`). Missing: `--remote`, Node's typo detection (`--enter` etc., `cli.ts:1180-1186`), "cannot mix positional with --seq" error.

#### `pty attach` (`:594-619`)
Only a positional `<ref>`; every `-`-prefixed arg is silently skipped (`:596`). Prints `[attached to <name> — press Ctrl+\ to detach]` to stderr. Client loop in `src/client.rs:310-436`: raw mode, ATTACH with current tty size (non-neutral), reader thread prints DATA/SCREEN, on EXIT restores termios, prints `\r\n`, `exit(code<0?0:code)`; Ctrl+\ (0x1c, also the Kitty encoding `ESC[92;5u` normalised) arms a 300 ms double-tap window: second tap sends a literal 0x1c, timeout detaches (`:369-421`), then DETACH packet, `TERMINAL_SANITIZE` + cursor-to-bottom + `[detached]` (`:423-435`). Missing Node flags: `-r/--auto-restart`, `--no-restart`, `--force`, `--remote`, `--attach-stream-fd-v1`, the attach nesting guard (`cli.ts:994-1042`).

#### `pty up|down [dir] [names…]` (`:644-738`)
First arg is a dir if it exists; remaining args filter by short name or display name. `up`: skips running (`already running`), spawns `sh -c "<exports>; <command>"` (`ptyfile::command_with_env_exports`) at 24×80 with `--name <displayName>` and manifest tags; exits 1 if any failed. `down`: `kill_session` for each present. Node adds a `ptyfile` tag and other behaviours (not compared further).

#### `pty restart <ref>` (`:771-826`)
Kills if alive, `registry::cleanup` (removes `.json`!), respawns `meta.command + meta.args` at **24×80** with same cwd/displayName/tags, prints `restarted <name>`. Node: `-y/--yes`, `--force`, prompts if running, persists rows/cols and env (`cli.ts:1358-1382`).

#### `pty rm <ref>` (`:829-850`)
Kills if alive, then removes all files, prints `removed <name>`. Node refuses on running (`Session "x" is still running…` exit 1), waits ≤7 s for the recorded daemon pid to exit, generation-checks, prints `Session "x" removed.` (`cli.ts:3036-3087`). Not-found text: Rust `pty rm: no such session '<ref>'`; Node `Session "<x>" not found.` (`cli.ts:3040`).

#### `pty rename <ref> <label…>` (`:853-880`) — rewrites `displayName` in the JSON; no lock, no event.

#### `pty kill <ref>` (`:883-901`, `kill_session` `:747-768`)
SIGTERM the daemon pid, wait ≤3 s for it to exit, SIGKILL as last resort; prints `killed <name>` exit 0 **even when no pid file exists**. Node: exit 1 if not running, strips `strategy` tag, waits ≤7 s, prints `Session "x" killed.` (`cli.ts:2618-2665`).

#### `pty status|stats [--all] [<ref>]` (`:919-968`)
`--json` is accepted but meaningless: output is **always JSON** (Node prints a human screen without `--json`). With ref: live → the daemon's `StatsResult` (A4); gone → `GoneStats {name,status,exitCode,exitedAt[,tags]}` (`:904-917`, matches Node's gone shape `cli.ts` cmdStats+12..+20). No ref: array of live results (`{"name":…,"error":"query failed"}` on failure) plus gone entries only with `--all` (`:952-967`).

#### Missing subcommands entirely
`interactive`/`i` (Node default with no args), `exec`, `events`, `emit`, `remote-serve`, `recover`, `gc`, `tag`, `tag-multi`, `metadata` (`patch`), `evidence`, `test`, `completions` (`pty/src/cli.ts:738-1640`).

#### Version / help
Rust `version` prints `0.1.0`; Node prints `<semver>+<short-sha>` (`0.12.0+…`, `pty/src/version.ts:42-49`). Rust help is one fixed block (`:970-990`) that omits `--force/-e/--tag/-d/--paste/-f` from `run`/`send` lines and lists `stats` only (not `status`).

#### Signal handling (client side)
No handlers in the CLI; `attach` relies on `RawMode` Drop (`src/client.rs:187-218`) and the reader thread's explicit termios restore on EXIT (`:345-350`).

### A2. Protocol (`src/protocol.rs`) and attach handshake

Frame `[type u8][len u32 BE][payload]`, `HEADER_SIZE=5`, `MAX_PACKET_LENGTH = 32 MiB` (`:73-76`), identical to Node (`pty/src/protocol.ts:23-30`).

Message types (`:12-31`, `:36-63`): DATA 0, ATTACH 1, DETACH 2, RESIZE 3, EXIT 4, SCREEN 5, PEEK 6, STATUS 7, `Unknown(u8)` for everything else. **Node additionally defines GEOMETRY 10** (`protocol.ts:13`, `encodeGeometry` `:74-79`) and reserves 8/9. Rust never sends GEOMETRY; Rust clients ignore unknown types (`src/client.rs:100-104`, `:337-355`) so a Node daemon's GEOMETRY is dropped silently.

Encoders/decoders: `encode_data/attach/detach/resize/exit/peek/screen/status/status_response`, `decode_size` (default 24×80), `decode_exit` (default -1), `decode_peek` (bit0 plain, bit1 full) — all byte-compatible with Node except **ATTACH**: Rust `encode_attach(rows, cols, geometry_neutral)` appends a 5th flag byte `ATTACH_FLAG_GEOMETRY_NEUTRAL = 0x01` when neutral (`:78-115`); Node's `encodeAttach` is 4 bytes only and the Node server only checks `length < 4` (`server.ts:932`), so the flag is a Rust extension the Node daemon ignores.

`PacketReader::feed` (`:183-231`) returns `io::Error(InvalidData)` and clears its buffer on an oversize length (Node throws `PacketTooLargeError` and poisons, `protocol.ts:141-146`); unknown type bytes are preserved. `read_packet` (`:234-255`) is a blocking one-shot reader used by `client::status`.

Handshake (daemon `src/daemon.rs:508-533`, `:320-343`): client sends ATTACH → daemon marks it streaming, immediately sends one SCREEN containing `screenshot::serialize_for_replay` (VT with modes+cursor+kitty flags, `src/screenshot.rs:39-48`), resizes PTY+terminal if non-neutral and size differs (last attach wins), stamps `lastAttachAt`. No GEOMETRY, no 80 ms redraw settle, no SIGWINCH nudge, no "initial screen cut" (Node `server.ts:931-994`). PEEK → one SCREEN (plain text or VT) without marking the client streaming (`:344-350`). DATA from any connection is written to the PTY (`:351-354`) — Node drops DATA from readonly clients (`server.ts:1020-1025`). RESIZE from any connection is applied verbatim (`:355-360`) — Node only from attached non-readonly clients with min-wins negotiation (`server.ts:1029`). STATUS → one STATUS JSON reply. DETACH or EOF removes the client.

### A3. Registry (`src/registry.rs`)

Directory: `$PTY_ROOT` → `$PTY_SESSION_DIR` → `~/.local/state/pty` (`:40-51`), created 0700 by `ensure_session_dir` (`:54-63`, called by the daemon and by `write_metadata`; **`pty ls` does not create it**).

Files written per session:
| file | writer | notes |
|---|---|---|
| `<name>.json` | `write_metadata` (`:108-117`): pretty JSON via tmp `<name>.json.tmp` + rename | Node tmp is `<target>.tmp.<pid>.<rand>` (`sessions.ts:251-275`) |
| `<name>.pid` | `write_pid` (`:154-160`): daemon pid, tmp `<name>.pid.tmp` + rename | Node writes plainly |
| `<name>.sock` | daemon `UnixListener::bind` (`src/daemon.rs:255-257`) | removed at teardown |
| `<name>.screen` | `write_final_screen` (`:95-100`): JSON `{plain, ansi}` of the last screen, preserve mode only | **Rust-only file**; Node has no equivalent (uses `lastLines`) |
| `<name>.events.jsonl` | never written; removed by `cleanup` if present (`:229`) | Node tier-1 event log |

Not written: `<name>.lock`, `<name>.events.lock`, `.recovery/`, `theme`, `gc.log` (`pty/docs/disk-layout.md` table).

`SessionMetadata` (`:15-36`, serde camelCase, optionals skipped when None):
`command, args, displayCommand, cwd, createdAt` (ISO-8601 seconds, `Z`, `src/daemon.rs:557-569` — Node has milliseconds), `exitCode?, exitedAt?, lastLines?` (last 50 lines, `src/daemon.rs:448-449`), `tags?` (BTreeMap), `displayName?`, `lastAttachAt?`.

Field-by-field vs Node `SessionMetadata` (`pty/src/sessions.ts:137-181`, `docs/disk-layout.md`):
- Present in both: `command, args, displayCommand, cwd, createdAt, exitCode, exitedAt, lastLines, tags, displayName, lastAttachAt`.
- **Missing in Rust**: `generation`, `daemonPid`, `recovery{…}`, `rows`, `cols`, `ephemeral`, `isolateEnv`, `extraEnv`, `unsetEnv`, `env`.
- Extra in Rust: none inside the JSON (unknown fields on read are dropped by serde, so a Node-written file round-trips through `rename`/`ClientAttach` with `generation/daemonPid/recovery/rows/cols/env…` **silently deleted** — `src/bin/pty.rs:866-877`, `src/daemon.rs:339-342`).
- `command` in Rust is the argv[0] as typed, not a resolved binary path (Node: "resolved binary path").

Liveness (`list_sessions` `:172-216`): enumerates `*.json` (skipping `*.json.tmp`), then `dead = (pid file readable && !pid_alive(pid)) && !socket_reachable(name)`; `alive = exit_code.is_none() && !dead` (`:208-210`). `pid_alive` = `kill(pid,0)==0 || EPERM` (`:135-141`). Status mapping in `cmd_ls`: alive→`running`, `exitCode` present→`exited`, else→`vanished`. So a session with metadata but no pid file and no socket is reported `running` (indeterminate ⇒ alive; test `tests/registry_liveness.rs:53-116`). Node additionally scans `.sock` files without `.json`, uses a socket-probe budget, and accepts `daemonPid` only with a matching recovery token (`sessions.ts:895-1000`).

`cleanup` (`:219-230`) removes `.sock .pid .json .screen .events.jsonl`. `resolve_ref` (`:288-297`): exact name, else first `displayName` match (Node requires a unique match).

### A4. Daemon (`src/daemon.rs`)

Threading: libghostty `Terminal` is `!Send` (`Terminal<'static,'static>` behind `Rc<RefCell>` callbacks), so the **actor thread is `daemon::run` itself** (`:133-474`): it owns the terminal, the PTY writer, the client map and all metadata writes. Helper threads only send `DaemonMsg` (`:48-58`) over one `mpsc` channel: the PTY reader thread (`:233-252`, also `child.wait()`s and sends `PtyExited`), the accept thread (`:261-270`), and per client a reader thread (packets → msgs, `:494-540`) and a writer thread (`Sender<Vec<u8>>` → socket, `:482-491`). Query replies libghostty generates (`on_pty_write`, `:222-227`) accumulate in an `Rc<RefCell<Vec<u8>>>` and are flushed to the PTY after every `vt_write` (`:279-285`, `:298-299`).

Behaviours:
- PTY via `portable-pty` `openpty` with `cfg.rows/cols` (defaults 24×80 from the CLI), pixel 0 (`:137-145`).
- Child env: inherits the daemon's env (which inherited `pty run`'s), plus `cfg.env` (always empty), plus **`PTY_SESSION=<name>` and `TERM=xterm-256color` unconditionally** (`:150-155`). Node sets TERM only when absent (`server.ts:150-155`) and also injects `PTY_SESSION_GENERATION` (`server.ts:174-206`). Nothing is scrubbed.
- Terminal: `Terminal::new(Options{cols, rows, max_scrollback: 10_000})` (`:216-221`); `scrollbackCapacity` reported as `rows + 10000` (`:385`, matches Node `server.ts:1129`).
- Signals: `SIGTERM`/`SIGINT` handler (`:163-167`, `on_external_stop` `:77-89`) sets `EXTERNAL_STOP` and sends **SIGHUP** to the child; a watchdog thread SIGKILLs the child 500 ms later (`:171-182`). No SIGHUP handler on the daemon itself, no `PTY_SHUTDOWN_DEADLINE_MS`, no spawner-liveness watchdog.
- Metadata + pid written after a successful spawn (`:189-212`); `pid` = daemon pid (Node semantics).
- Attach: SCREEN replay (modes+cursor+kitty), geometry negotiation "last non-neutral attach wins" (`:320-343`), `lastAttachAt` stamped on every attach (Node: non-readonly only).
- Peek: `capture()` → `text` (plain) or `ansi` (VT). `capture` (`src/screenshot.rs:54-78`) uses libghostty `Format::Plain` / `Format::Vt` with default options; the Plain output **includes scrollback** (asserted by `tests/terminal_fidelity.rs:49-70`), only trailing blank rows are popped, lines are not right-trimmed (`tests/parity.rs:41-73`). Node's non-`--full` peek is viewport-only (`server.ts:1015-1017`).
- Resize: any RESIZE sets `cur_rows/cols`, resizes PTY and terminal (`:355-360`).
- Multiple clients: unlimited; DATA broadcast only to `streaming` (attached) clients; dead senders dropped (`:300-308`).
- Status reply (`:361-420`) → `stats::StatsResult` (`src/stats.rs:74-83`): `name; terminal{cols,rows,cursorX,cursorY,scrollbackUsed,scrollbackCapacity}; process{alive:true,exitCode:null,pid:<child>,resources{rssKb,cpuPercent}|null}; daemon{pid,resources}; clients{total,attached,readOnly[,geometryNeutral]}; modes{sgrMouse,cursorHidden,kittyKeyboard,kittyKeyboardFlags:[bits]}; uptimeSeconds; createdAt`. Resources from `/proc/<pid>/statm` + `/proc/<pid>/stat` (`src/stats.rs:99-148`). vs Node `StatsResult` (`pty/src/client.ts:295-341`): Node has an optional `clients.connections[]` (Rust none), Rust has `clients.geometryNeutral` (Node none); `kittyKeyboardFlags` is `[bits]` in Rust vs a list of numbers in Node. Attached vs read-only: streaming non-neutral vs streaming neutral; transient peek/status connections are not counted (`:361-372`).
- Child exit (`:424-466`): `should_reap(external_stop, ephemeral, keep, permanent, config_reap)` (`:108-129`): external stop → reap only if ephemeral; else `keep=true` → preserve; `ephemeral` → reap; `strategy=permanent` → preserve; else `PTY_REAP_ON_EXIT` (unset ⇒ **reap**; `false/0/no/off` ⇒ preserve). Same precedence as Node `shouldReapAtExit` (`sessions.ts:1069-1098`), except Node's `keep` accepts any non-falsey value (`isKeepRequested`, `sessions.ts:1040-1044`) while Rust requires exactly `keep=true` (`:427`). EXIT packet is sent to all clients before cleanup. Preserve path writes `<name>.screen`, `exitCode`, `exitedAt`, `lastLines` (last 50). Reap path `registry::cleanup`. Then teardown unconditionally unlinks `.sock` and `.pid` (`:471-473`) — hence `ls --json pid: null` after exit. Exit code = `ExitStatus::exit_code()` from portable-pty (`:543-545`); Node reports `128+signal` for signal deaths (`server.ts:573-577`) — not verified equivalent here.
- No events log, no generation token, no recovery capability, no `PTY_SESSION_GENERATION`, no `nudgeRedraw` on attach, no readonly-client DATA filtering.

`default_cwd()` = process cwd (`:586-588`). `now_iso8601` is hand-rolled (`:557-583`), second precision.

### A5. Testkit public API (`pty-testkit` crate)

- `Session::spawn(command: &str, args: &[&str], opts: SpawnOptions) -> io::Result<Session>` (`src/session.rs:89-183`); `SpawnOptions { rows: Option<u16>, cols: Option<u16>, cwd: Option<PathBuf>, env: Vec<(String,String)> }` (`:34-43`). Env is `env_clear()` + `build_spawn_env(process env, opts.env)` (scrubs `PTY_SERVER_CONFIG`, `PTY_SESSION`, and `PTY_ROOT`/`PTY_SESSION_DIR` unless the caller set them, `:51-70`) + `TERM=xterm-256color` default.
- Properties: `rows()`, `cols()` (`:212-219`).
- Input: `send_keys(&str)`, `press(&str) -> Result<(), KeyError>`, `type_str(&str)` (`:225-240`).
- Screen: `screenshot() -> Screenshot` (`:245-248`), `title() -> String` (OSC title, `:252-258`).
- Waits: `wait_for_text(text, timeout_ms) -> Result<Screenshot,String>`, `wait_for_absent`, `wait_for(pred, timeout_ms, description)` (`:263-306`, 20 ms poll, error message includes the screen).
- `resize(rows, cols)` (`:311-321`, PTY + terminal), `has_exited()`, `close()`, `Drop` kills (`:326-340`).
- `Screenshot { lines: Vec<String>, text: String, ansi: String }` + `contains()` (`src/screenshot.rs:9-23`); `capture(&Terminal)`, `serialize_for_replay(&Terminal)`.
- `keys::resolve_key`, `keys::parse_seq_value`, `KeyError` (`src/keys.rs`); `duration::{parse_duration, format_duration}`; `input::{parse_key, parse_input, KeyEvent, MouseEvent, InputEvent, …}` (`src/input.rs:10-59, 158, 184`); `queries::strip_terminal_queries`; `paste::{wrap_bracketed_paste, BRACKETED_PASTE_START/END}`; `ptyfile::{read_pty_file, command_with_env_exports, PtyFile, PtySessionDef}`; `registry::*`, `client::*`, `daemon::{run, DaemonConfig, should_reap, default_cwd, now_iso8601, now_epoch_f64}`, `stats::*`.

Lacks vs Node's testing library (`pty/docs/testing.md`):
- `Session.server()` mode entirely: `attach()`, `reconnect()`, `connectToExisting()`, `name`, `server`, server-mode `resize` with effective min-wins geometry, `hasExited` semantics. (The Rust daemon exists but there is no in-process client `Session` over it; `client::*` is CLI-oriented.)
- Default 5000 ms timeout (Rust requires `timeout_ms`), async API (Rust is blocking/polling).
- Key-spec separators: Node accepts `ctrl+c`, `ctrl-c`, `ctrl_c`, `C-c` (`pty/src/keys.ts:24-64`); Rust splits on `+` only (`src/keys.rs:69`).
- `press` returns `Result` instead of throwing; `sendKeys`/`type` are `send_keys`/`type_str`.
- `pty test` (vitest wrapper), the TUI framework/`PtyHandle` embedding API (README "Still on the TypeScript side", `README.md:218-223`).
- `screenshot().lines` semantics: Node right-trims each line; Rust deliberately keeps written trailing spaces (`src/screenshot.rs:57-64`, parity-pinned in `tests/parity.rs:41-73`).

### A6. Tests

172 `#[test]` functions + 1 doctest = 173 (README table `README.md:168-189`). Heavy daemon tests hold a process-wide mutex (`tests/cli_e2e.rs:26-29`) and use a unique `PTY_ROOT` per test with `PTY_SESSION`/`PTY_REAP_ON_EXIT` scrubbed (`:41-70`).

| file | count | covers |
|---|---|---|
| `tests/cli_e2e.rs` | 14 | run/ls/peek/send/kill lifecycle; up/down; restart/rename/rm; `peek -f`; `stats --json` contract (exact/type/omit, `:290-372`); bare-semver version; `ls --json` node shape (`:393-447`); default reap; external kill preserves; post-exit peek; `run --force` nested; attach double-tap; nesting prevention; interactive attach driven through the testkit |
| `tests/duration.rs` | 15 | `parse_duration`/`format_duration` |
| `tests/env_isolation.rs` | 5 | `build_spawn_env` scrubbing |
| `tests/input_parse.rs` | 21 | key parsing incl. Kitty CSI-u |
| `tests/interactive_tui.rs` | 3 | bash readline editing via libghostty |
| `tests/keys.rs` | 21 | `resolve_key`, `parse_seq_value` |
| `tests/mouse_parse.rs` | 9 | SGR mouse |
| `tests/parity.rs` | 7 | seq delay rounding; plain capture trailing-space; replay restores modes |
| `tests/parity_fixtures.rs` | 2 | shared JSON fixtures (below) |
| `tests/paste.rs` | 4 | bracketed paste |
| `tests/protocol.rs` | 20 | framing round-trips, unknown types, oversize |
| `tests/ptyfile.rs` | 16 | manifest parsing |
| `tests/registry_liveness.rs` | 1 | node #117 liveness rule |
| `tests/terminal_fidelity.rs` | 4 | alt-screen, scrollback, bold/underline, CR overwrite |
| `tests/terminal_queries.rs` | 19 | query stripping; DA1/DSR/DA2 replies |
| `tests/terminal_spawn.rs` | 11 | echo/ls/colors/CUP/CJK/clear/ctrl-c/resize/title |

Fixtures (`tests/fixtures/parity/`): byte-identical mirrors of the Node repo's `tests/fixtures/parity/screens.json` and `shapes.json` (verified with `cmp`; Node loaders are `tests/parity-fixtures.test.ts`, `parity-shapes.test.ts`).
- `screens.json` (v2, 3 fixtures): `idle-prompt-plain` (`printf 'READY> '; exec cat` → plain screen exactly `READY> `, length 7); `post-exit-final-screen` (`PTY_REAP_ON_EXIT=false`, exit 7 → screen `LINE_A\nLINE_B\nDONE`, `status=exited`, `exitCode=7`, idempotent peek); `post-exit-reaped` (default reap → peek non-zero, absent from `ls`). Loader `shared_parity_fixtures_pass` (`tests/parity_fixtures.rs:191-286`) runs `pty run --id <id> --rows --cols -- <cmd…>` with the fixture env, sleeps `settleMs`, then `peek --plain` / `ls --json`.
- `shapes.json` (v1, 2 fixtures): `ls-json-shape` — two sessions created with the exact Node CLI argv (`run -d --id runsess --no-display-name -- cat`, `run -d --id exsess --name my-label -- sh -c "exit 3"`) under `PTY_REAP_ON_EXIT=false`, each `ls --json` field asserted by policy `{exact}|{type}|{omitWhenUnset}`; `client-count-during-peek` — after a transient `peek`, `stats --json` has `clients.attached == 0`. Loader `shared_json_shape_fixtures_pass` (`:92-189`).

### A7. Dependencies and build

`Cargo.toml` (17 lines): package `pty-testkit` 0.1.0, **`edition = "2024"`** (README `:149` says 2021 — stale); deps `libc 0.2.186`, `libghostty-vt 0.2.1`, `portable-pty 0.9.0`, `regex 1.13.1`, `serde 1.0.229 (derive)`, `serde_json 1.0.151`, `toml 1.1.3`; dev-deps repeat `libghostty-vt` and `serde_json`. `Cargo.lock` is committed and pins `libghostty-vt-sys 0.2.1` from crates.io. Source uses let-chains (`src/bin/pty.rs:246-248`, `src/registry.rs:40-41`) → needs Rust ≥ 1.88; README says built with 1.97.

Build requirement (README `:147-160`): **Zig 0.15.2 on PATH**; `libghostty-vt-sys`'s build script fetches the Ghostty source and runs `zig build` on first build (~20 s, network needed, then cached). No `build.rs`, no Cargo features, no `flake.nix`/`nix.md`, no CI config in pty-rust. `.gitignore` = `/target`. `.convoy/` and `.claude/rules/convoy.md` are the working agent's own convoy setup (a `pty.toml` declaring that agent's sessions), not part of the port.

Binary: `target/{debug,release}/pty`. The daemon re-execs `current_exe()` (`src/bin/pty.rs:202`), so the binary path must stay valid for the life of every session it hosts.

How st2 was pointed at it: not via nix or PATH. st2 commit b63abfb ("Let one task choose the pty build that hosts it") added `ST2_PTY_BIN` — a per-task env key; only `pty run` follows it (`st2 src/run.rs:387-400`). st2's `flake.nix:9` still pins the **Node** pty (`github:compoundingtech/pty/504ac73…`) and uses `pty.packages.${system}.default` for its checks (`flake.nix:193`). An internal note says one agent opted in "with four lines in its declaration" (env `ST2_PTY_BIN=<path>`), and that Node `list --json`/`kill`/`rm`/`send --seq`/`peek` were exercised against a Rust-hosted daemon.

---

## Part B — st2 call sites

Runner: `PtyCli { bin: "pty", catalog_root }` (`src/run.rs:268-279`); root resolution `effective_pty_root` = exported `$PTY_ROOT` if non-empty, else the catalog's declared `pty_root`, else `<catalog>/pty` (`src/run.rs:313-323`, `src/catalog.rs:95-100`). Timeouts: `PTY_LIST_TIMEOUT = 2s`, `PTY_DAEMON_SHUTDOWN_WAIT = 6s` (`src/run.rs:44-45`); ding `PTY_COMMAND_TIMEOUT = 2s` (`src/ding/mod.rs:45`).

### B1. Every invocation, with argv

1. **`pty run`** — `build_run_command` (`src/run.rs:402-486`), program = `run_bin()` = task env `ST2_PTY_BIN` (expanded) if non-empty else `pty` (`:395-400`). Argv:
   `run -d --force --id <pty_id> ( --name <display> | --no-display-name ) --cwd <dir> [--tag k=v]* [--tag keep=true (agent/ding tasks)] [--unset-env NO_COLOR (agent tasks without NO_COLOR)] (--env K=V)+ -- ( sh -c <command> | <argv…> )`.
   `--env` is emitted for every key of `managed_task_env` (`:359-369`): always `CATALOG`, `ST_ROOT`, `PTY_ROOT`, `TERM=xterm-256color`, plus `ST_HOOKS` when available, plus the task's declared env — so **at least four `--env` flags on every spawn**. The same map is also set in the process env (`:461-462`). Wrapped in `systemd-run --scope --unit=<unit>` when available (`src/run.rs:720-733`, `src/isolate.rs`). Retries up to 4× when stderr contains **`already in use`**, issuing `pty rm <id>` between attempts (`:741-757`). Failure text checked: `spawning pty '<id>' failed: <stderr>`.
2. **`pty list --json`** — `list_entries_at` (`:667-681`), env `PTY_ROOT=<root>`, 2 s timeout; parsed into `PtyListEntry { name, status, exitCode?, pid?, createdAt?, displayName?, tags (default {}) }` (`:282-301`). `task_observations_at_root` (`:498-636`) maps `status`: `running` needs `pid` **and** `createdAt` (else Indeterminate + error), `exited`, `vanished`, other → Indeterminate. Guards the root's inode across the call and treats a missing root as "known empty" without calling `pty` (`:508-519`). `list_sessions` (`:704-718`) derives `alive = status=="running"`, `exit_code`, and `presentation{display_name, tags}` which `reconcile::presentation_matches` (`src/reconcile.rs:620-630`) compares to the declared tags/display name; a mismatch triggers call 5.
3. **`pty kill <id>`** — `:765-777`, env `PTY_ROOT`; non-zero → error with stderr.
4. **`pty rm <id>`** — three sites: corpse cleanup during spawn retry (`:752-756`); `reap_for_restart` (`:779-813`) which first reads **`<root>/<id>.pid`** directly and waits ≤6 s for that pid to die, then `rm`, treating stderr containing **`not found`** as success; `remove` (`:815-828`).
5. **`pty metadata patch --id <id>`** with one JSON object on stdin `{"displayName"?: string|null, "tags": {k: string|null}}` (`PtyMetadataPatch`, `:303-308`; `patch_presentation` `:639-661`, 2 s timeout; unit test `:3209-3240`).
6. **`pty peek <session>`** — ding's composer inspection, ANSI output, 2 s timeout (`src/ding/mod.rs:332-345`).
7. **`pty send <session> --seq <ESC[200~text ESC[201~>`** — stage a bracketed-paste notice (`pty_stage_args`, `:245-252`).
8. **`pty send <session> --seq key:return`** — submit (`pty_submit_args`, `:255-262`).
9. **`pty send <session> --with-delay 0.5 --seq <paste> --seq key:return`** — recovery transport (`pty_delivery_args`, `:268-279`).
10. **`pty --help`** — startup probe that `pty` is runnable (`probe_pty_on_path`, `:769-775`, called `:1185`).
11. **`pty peek --full --plain <task_id>`** — eval log dump to `<catalog>/logs/<id>.log` (`src/eval_run.rs:1022-1041`), env `PTY_ROOT`.
12. **`st2 pty <args…>`** — exec pass-through of arbitrary `pty` argv with `CATALOG`/`ST_ROOT`/`PTY_ROOT` set to the catalog's declared root (`src/main.rs:1633-1651`); `st2 env` prints the same exports (`:1667-1677`); `st2 doctor` only checks `pty` is on PATH (`:1700-1705`).
13. **`pty --root <root> stats --json <id>`** — test-only, reads `process.pid` (`tests/eval_run_e2e.rs:1160-1187`).

Counting distinct invocation shapes gives 13 (items 1-13); distinct subcommand words are 9: `run, list, kill, rm, metadata, peek, send, stats, --help` (+ the open-ended pass-through). The Rust port's original surface was the 8 words in its doc comment (`run/ls/peek/send/attach/kill/status/__daemon`, `pty-rust/src/bin/pty.rs:2`); today it has 15 words (A1).

Direct `$PTY_ROOT` file reads (no `pty` fork): `<root>/<id>.pid` in `reap_for_restart` (`src/run.rs:787`), `ding::session_alive` (`src/ding/mod.rs:698-709`, ambient `PTY_ROOT`/`PTY_SESSION_DIR`/`~/.local/state/pty`), `ding::session_liveness_in` (`:724-747`; missing/unparseable → Indeterminate, ESRCH → Dead, EPERM → Alive) used by the harness-state reader (`src/harness_state.rs:636-641`, `src/agents.rs:95-108`, `src/main.rs:1858-1859`). Nothing reads `<id>.json`, `.sock` or `.events.jsonl` directly. Fake `pty` shims in tests only emit `list --json` arrays (`src/run.rs:3997-4001`, `:4060`, `tests/atomic_pty_snapshot.rs:31-34`) or echo env (`tests/pty.rs:12-20`).

Env vars st2 sets for pty: `PTY_ROOT` on every pty op (all sites above) and in every task's env (`src/run.rs:359-369`, `src/exec_backend.rs:154-171`, `src/materialize.rs:451-452`, systemd unit `Environment=PTY_ROOT=` `src/service.rs:284-288`, eval `src/eval_run.rs:810`); `CATALOG`, `ST_ROOT`, `TERM`, `ST_HOOKS` via `--env`; `NO_COLOR` removed for agents. `ST2_PTY_BIN` is read (task env) not set. `PTY_SESSION` is not set by st2 (the daemon sets it); st2 stores the runtime id as `pty_session` in harness-state records (`src/harness_state.rs:165`, `src/claude_session.rs:126`) and uses it only as the `<id>.pid` key.

### B2. Which calls the Rust port satisfies at e4d6cda

| st2 call | Rust status | detail |
|---|---|---|
| 1 `run …` | **No** | `--unset-env`/`--env` are unparsed and become the command (`pty-rust/src/bin/pty.rs:122-127`); daemon exec fails, `pty run` still exits 0 and prints the id, no `<id>.json` is written → st2 sees no session and respawns every pass. Every other flag st2 passes (`-d --force --id --name --no-display-name --cwd --tag`) is accepted. The internal note's working experiment therefore must have used a build beyond main@e4d6cda (or the node daemon's registry with a patched binary) — not verifiable from the repos. |
| 2 `list --json` | Partial | Shape and `status` enum match; `pid`+`createdAt` present for running (so `RuntimeGeneration` builds). **`tags` is never emitted**, so `presentation_matches` fails for any task with declared presentation tags and st2 issues `metadata patch` (call 5) on the default binary every pass. |
| 3 `kill <id>` | Yes (semantics differ) | SIGTERM to the Rust daemon → SIGHUP child → preserve (external stop) — good for st2's evidence retention. Exit 0 even when nothing was killed (Node exits 1). |
| 4 `rm <id>` | Partial | Works; but the not-found text is `no such session '<id>'`, not `not found`, so `reap_for_restart`'s benign-miss path becomes a hard error; the `already in use` retry key never matches (`already exists`). Also removes `.screen`; Node's `rm` against a Rust-preserved session leaves `<id>.screen` behind. |
| 5 `metadata patch --id` | **No** | Subcommand absent → `unknown command 'metadata'` exit 1. Only matters if the default `bin` is the Rust binary. |
| 6 `peek <s>` | Yes (caveat) | Works; output includes up to 10k scrollback lines plus VT mode prefix (Node returns the viewport), inflating ding's 2 s inspection and making "notice present in composer" checks see older text. |
| 7-9 `send --seq …`, `--with-delay` | Yes | Same pacing rules (300 ms default, round). `key:return` resolves. |
| 10 `--help` | Yes | exit 0. |
| 11 `peek --full --plain` | Yes (nominally) | `--full` ignored by the daemon but plain capture already includes scrollback. |
| 12 `st2 pty …` | Depends on argv | Any Node-only word/flag (`exec`, `gc`, `tag`, `events`, `--root`, `ls --tags`, …) fails. |
| 13 `--root … stats --json` | **No** | No global `--root` → `unknown command '--root'`. `stats --json <id>` itself works and includes `process.pid`. |

Registry-level compatibility for a mixed fleet (Node `bin` + Rust `ST2_PTY_BIN` daemon): Node `list` sees Rust sessions (json+pid+sock present; liveness via socket/pid); Node `kill`/`rm` work on the pid file; Node `metadata patch` rewrites the Rust-written `<id>.json` (adds `generation`-less entries; unverified whether Node's lock/generation checks accept a file with no `generation`); Rust `rename`/attach rewrite drops any Node-only fields. Node clients' GEOMETRY expectation on attach/peek is not sent by the Rust daemon (the internal note reports `peek`/`send`/vim over a Node client working anyway).
