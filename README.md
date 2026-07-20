# pty-rust

A Rust port of the [`pty`](https://github.com/compoundingtech/pty) project —
persistent terminal sessions with detach/attach **plus** a Playwright-style TUI
testing library — using **[libghostty]** (the terminal library extracted from
the [Ghostty] terminal emulator) as the terminal-emulation backend in place of
`@xterm/headless`.

Two things live here:

1. **The `pty` CLI (v0)** — a runnable `pty` binary with a per-session daemon
   that owns the PTY and a libghostty terminal: `run` / `ls` / `peek` / `send` /
   `attach` / `kill` / `status`. See [The `pty` CLI](#the-pty-cli-v0) below.
2. **`pty-testkit`** — the Rust port of pty's terminal testing harness (the
   `Session` type), which is also the foundation the CLI is built on and the
   correctness net for the port.

[libghostty]: https://libghostty.tip.ghostty.org/
[Ghostty]: https://ghostty.org

## The `pty` CLI (v0)

```sh
cargo build                                   # builds the `pty` binary
PTY=target/debug/pty

$PTY run -- bash --norc --noprofile           # spawn a persistent session -> prints its id
$PTY ls                                        # list sessions (--json for JSON)
$PTY peek <id>                                 # print the current screen (ANSI)
$PTY peek --plain <id>                         # ... as plain text
$PTY peek --wait "Ready" -t 10 <id>            # wait until text appears
$PTY send <id> --seq "echo hi" --seq key:return   # send an ordered key sequence
$PTY send <id> "literal text"                  # or literal text (no newline)
$PTY attach <id>                               # attach interactively (Ctrl-] to detach)
$PTY status <id>                               # session stats as JSON
$PTY kill <id>                                 # terminate the session
```

Each session runs in a detached daemon that hosts the PTY and a libghostty
terminal; clients connect to a per-session unix socket under `$PTY_ROOT`
(default `~/.local/state/pty`) speaking the wire protocol (`protocol.rs`). The
daemon replays the screen on attach and answers device queries (DA1/DSR) through
libghostty, so the session behaves like a real terminal. Set `PTY_ROOT` to
isolate a registry (e.g. for tests).

## The testing harness (`pty-testkit`)

The original `pty` project ships a `Session` testing harness: it spawns a
process in a PTY, feeds the output into a headless `xterm.js`, and lets tests
take "screenshots" (plain text + ANSI) and wait for on-screen content. This
crate reimplements that harness in Rust, swapping the terminal emulator for
libghostty:

```
   real process  ──stdout──▶  PTY  ──bytes──▶  libghostty Terminal  ──▶  Screenshot
   (bash, ls, …)              (portable-pty)   (VT parse + grid)         { lines, text, ansi }
        ▲                                            │
        └──────────────  input / query replies  ◀────┘
```

- **`Session::spawn`** — spawn a command in a real PTY.
- **`screenshot()`** → `{ lines, text, ansi }`, matching the TS `Screenshot`
  (plain text via libghostty `Format::Plain`; ANSI via `Format::Vt`).
- **`wait_for_text` / `wait_for_absent` / `wait_for`** — poll the screen.
- **`send_keys` / `type_str` / `press("ctrl+c")`** — send input; named keys use
  the same encoding table as `pty`'s `keys.ts`.
- **`resize(rows, cols)`** — resize the PTY (SIGWINCH) and the emulator together.
- **`title()`** — the OSC-set window title libghostty tracks.
- Terminal **query replies** (DA1, DSR, …) that libghostty generates are
  captured via `on_pty_write` and flushed back to the PTY, so programs that
  block on a device-attributes response (e.g. fish) start promptly.

Because `Terminal` from libghostty is `!Send`, it lives on the test thread; a
reader thread only ferries raw PTY bytes over a channel, which the main thread
drains into the terminal on demand.

## Build requirements

- **Rust** (edition 2021; built with 1.97).
- **Zig 0.15.2** on `PATH`. The `libghostty-vt-sys` build script fetches the
  Ghostty source and compiles the VT core with `zig build`, so a matching Zig
  toolchain must be installed. Install it with:

  ```sh
  curl -fsSL https://ziglang.org/download/0.15.2/zig-x86_64-linux-0.15.2.tar.xz | tar -xJ -C ~/.local/opt
  ln -sf ~/.local/opt/zig-x86_64-linux-0.15.2/zig ~/.cargo/bin/zig   # ~/.cargo/bin is already on PATH for cargo
  ```

The first build clones + compiles Ghostty's VT core (~20s); it is cached
thereafter.

## Running the tests

```sh
cargo test
```

111 tests pass:

| Test file | Ported from | Count | Backend |
| --- | --- | --- | --- |
| `tests/keys.rs` | `tests/keys.test.ts` | 21 | pure |
| `tests/duration.rs` | `tests/duration.test.ts` | 15 | pure |
| `tests/env_isolation.rs` | `tests/env-isolation.test.ts` | 5 | pure |
| `tests/input_parse.rs` | `tests/input-parse.test.ts` | 21 | pure |
| `tests/mouse_parse.rs` | `tests/mouse-parse.test.ts` | 9 | pure |
| `tests/terminal_queries.rs` (strip) | `tests/terminal-queries.test.ts` | 16 | pure |
| `tests/terminal_queries.rs` (responses) | `tests/terminal-queries.test.ts` | 3 | **libghostty** |
| `tests/terminal_spawn.rs` | `screenshot.test.ts` / `shells.test.ts` | 11 | **libghostty** |
| `tests/terminal_fidelity.rs` | `screen-replay-altscreen` / `scrollback-fidelity` | 4 | **libghostty** |
| `tests/interactive_tui.rs` | interactive-editing (Playwright-style) | 3 | **libghostty** |
| `tests/cli_e2e.rs` | `pty` CLI lifecycle + attach | 2 | **libghostty** |
| doctest | — | 1 | — |

`interactive_tui.rs` drives `bash`'s raw-mode readline through the harness —
arrow-key cursor editing, `Ctrl-A` line-start jump, `Ctrl-C` line-discard — and
asserts on how libghostty renders the in-place redraws. This is the marquee use
case: send keystrokes, watch the screen update, assert the result.

The query-response tests prove libghostty answers device queries end-to-end:
a program emits `ESC[c` / `ESC[6n` / `ESC[>c`, libghostty generates the reply
(`ESC[?62;22c` / `ESC[1;1R` / `ESC[>1;0;0c`), and the harness flushes it back to
the PTY. (libghostty does not answer the OSC 10/11 color queries without default
colors configured, so those two TS response cases are intentionally not ported.)

The libghostty-backed tests drive real programs and assert on the emulated
screen: `echo`/`ls`/`ls -la` capture, ANSI color preservation, cursor
positioning (CUP), CJK wide characters, clear-screen, bash input + echo, `ctrl+c`
interrupt, `ctrl+d`, resize → SIGWINCH (`stty size`), OSC window-title tracking,
alternate-screen enter/restore (`?1049h`/`?1049l`), scrollback retention, text
styling (bold/underline) in the ANSI capture, and carriage-return overwrite.

## Scope

The `pty` project is ~24.6k lines of tests across 108 files. Most of that suite
tests the TypeScript **CLI, session daemon, wire protocol, and TUI
framework/widgets** — porting those means porting the whole application, which is
outside this experiment. This crate ports the slice where **libghostty is the
subject**: the PTY → terminal-emulation → screenshot testing path, plus the pure
utility modules its harness depends on (`keys`, `duration`) and the spawn-env
isolation logic.

## Layout

```
src/
  bin/pty.rs      the `pty` CLI: run/ls/peek/send/attach/kill/status + __daemon
  daemon.rs       per-session daemon: PTY + libghostty terminal, serves protocol
  client.rs       client ops: peek / send / status / interactive attach
  protocol.rs     wire packet framing (port of protocol.ts)
  registry.rs     session dir + <name>.sock/.pid/.json (port of sessions.ts)
  session.rs      Session test harness: PTY (portable-pty) + libghostty Terminal
  screenshot.rs   Screenshot { lines, text, ansi } capture
  keys.rs         named-key → bytes (port of keys.ts)
  duration.rs     parse/format durations (port of duration.ts)
  input.rs        stdin key + SGR-mouse + Kitty CSI-u parsing (port of tui/input.ts)
  queries.rs      terminal-query stripping (port of stripTerminalQueries)
examples/demo.rs  live libghostty screenshot loop (cargo run --example demo)
tests/            ported test suites + CLI e2e (see table above)
```
