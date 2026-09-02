# pty-rust

`pty` keeps terminal sessions alive after you walk away. `pty run -- <command>`
starts the command inside a real pseudo-terminal that a small per-session daemon
owns; you can detach, come back with `pty attach`, read the screen from a script
with `pty peek --plain`, type into it with `pty send`, and list, tag, restart,
or kill sessions from any shell. Programs and agents drive sessions through the
same commands and JSON output. This repository is a Rust port of the Node
[`pty`](https://github.com/compoundingtech/pty), with
[libghostty](https://libghostty.tip.ghostty.org/) (the terminal core extracted
from [Ghostty](https://ghostty.org)) in place of `@xterm/headless`. The port is
meant as a drop-in: same commands, flags, texts, JSON shapes, exit codes, files
under `$PTY_ROOT`, and socket protocol, so the two implementations can share a
registry while a fleet migrates.

## Direction: compatibility and embedding

The long-term target is a behavior-compatible Rust implementation of the Node
`pty`, plus a first-class Rust API for embedding a live terminal in clients such
as Fractal. The Node implementation is the behavioral reference while the port
converges. This README does not claim that the current experiment has reached
full parity.

Compatibility means that the same user-visible operations and wire messages have
the same result. It does not require identical source code or internal design.
Rust, `portable-pty`, and `libghostty` can require a different implementation.
When that difference changes behavior, record a decision that states the Node
behavior, the Rust behavior, the reason, the client effect, and the conformance
test.

The embedding API should serve the Rust CLI and other Rust clients through one
implementation. Its terminal handle should eventually provide the capabilities
that Node's `@myobie/pty/tui` `PtyHandle` provides: attach lifecycle, input,
resize, typed cell-grid and wrapped-line reads, cursor and terminal-mode state,
scrollback access, and activity or exit events. `libghostty::Terminal` is not
`Send`, so one clear actor must own it and publish typed events or snapshots to
consumers.

Keep the current protocol as the baseline. Add a protocol feature only after a
real failing use case shows that the current byte-framed messages cannot express
the required behavior. Track the compatibility matrix, crate boundaries, and
acceptance tests in [issue #1](https://github.com/compoundingtech/pty-rust/issues/1).

Where the port stands against the Node `pty`, surface by surface, is in
[docs/parity.md](docs/parity.md); the work packages that close the gap are in
[docs/parity-plan.md](docs/parity-plan.md).

## Install

With Nix (flakes), from a checkout or straight from GitHub:

```sh
nix build                                   # ./result/bin/pty, completions under ./result/share
nix run . -- help
nix profile install github:compoundingtech/pty-rust
```

The flake builds hermetically: Ghostty's source and its Zig packages are
fixed-output fetches, so `nix build` needs no network beyond the Nix cache.
`nix flake check` also runs the test suite and verifies that the installed
completion files are what the binary prints. `nix develop` opens a shell with
the Rust toolchain and Zig, pointed at the same pre-fetched Ghostty.

With Cargo (see the build requirements below):

```sh
cargo install --path crates/pty
```

Shell completions ship in [`completions/`](completions/) and are also printed
by `pty completions <fish|bash|zsh>`.

## Usage

```sh
pty run -d --name "API" -- node server.js    # start a session in the background
pty list                                     # sessions (--json for programs)
pty peek --plain API                         # the screen as plain text
pty send API --seq "npm test" --seq key:return
pty attach API                               # interactive; Ctrl+\ detaches
pty kill API && pty rm API
pty help                                     # every command; pty <cmd> --help for one
```

Sessions live under `$PTY_ROOT` (default `~/.local/state/pty`): one unix socket,
pid file, and metadata file per session. Set `PTY_ROOT` to isolate a registry,
for example in tests.

`pty version` prints `0.13.<n>-rust+<short-sha>`: one minor above the Node line,
a `rust` pre-release tag, and the commit it was built from.

### Commands not in this build

Three Node commands are deferred (see [docs/parity.md §12](docs/parity.md#12-candidates-to-leave-off)
for the reasoning): `pty recover`, `pty evidence`, and `pty test`. Their help
texts are kept verbatim so `--help` still describes them, but running them
prints `pty <cmd>: not available in this build. See docs/parity.md.` and exits 1.

### One known defect, documented rather than fixed

**A session's lock files are not exclusive across a crash.** When a lock's
holder has died, any process may steal it, and two processes stealing the same
stale lock can both end up holding it. Measured on 2026-09-02: eight threads
released together against one stale lock produced more than one winner in 386
races out of 400.

The **Node tool has the identical sequence and the identical defect**, so a
shared `$PTY_ROOT` is no worse than either implementation alone, and neither
one can be relied on here.

**In ordinary use this does not arise.** Taking a lock still keeps two live,
healthy processes apart. It needs a daemon that died holding a lock and two
processes arriving together to clean up after it — typically two creators for
the same session name.

**Do not rely on these locks for correctness after a crash.** A correct steal
needs an exclusive create that only one process can win, which means a second
file in a directory both implementations read. That is a change to a protocol
they share and has to be agreed between them, which is why it is written down
here instead of fixed on one side. `crates/pty-core/src/registry/lock.rs` and
`docs/hardening.md` carry the interleaving in full.

## The crates

A Cargo workspace of six crates under `crates/`:

- **`pty-core`** — the wire protocol, session registry and locks, events,
  metadata, names and tags, key/paste/duration/input parsing, `pty.toml`
  manifests, and the client operations (attach loop, peek, send, status). No
  terminal emulator, no Zig.
- **`pty-terminal`** — the libghostty actor: owns the terminal, produces typed
  snapshots and the VT/plain serializations, answers terminal queries.
- **`pty-testkit`** — Playwright-style test sessions: spawn a process in a real
  PTY, feed it to libghostty, take screenshots, wait for text, send named keys.
- **`pty-tui`** — the TUI library (ratatui + crossterm): pane, theme, focus,
  widgets, and the app runner behind the interactive session manager.
- **`pty-conformance`** — the black-box suite that runs against any `pty`
  binary, Node or Rust, chosen with `PTY_TEST_BIN`.
- **`pty`** — the `pty` binary: the command-line interface, the per-session
  daemon, and the remote bridge.

## Building from source

- Rust 1.88 or newer (edition 2024; `rust-version` is pinned in `Cargo.toml`).
- Zig 0.15.2 on `PATH`, and `git`: the `libghostty-vt-sys` crate builds
  Ghostty's terminal core from source with Zig.
- The first build clones Ghostty at the commit `libghostty-vt-sys` pins and
  lets Zig fetch Ghostty's own packages; both are cached under `target/` after
  that. To build without network, point `GHOSTTY_SOURCE_DIR` at a Ghostty
  checkout and `GHOSTTY_ZIG_SYSTEM_DIR` at a populated Zig package directory
  (`flake.nix` shows how both are produced).

```sh
cargo build --release                        # target/release/pty
```

## Running the tests

```sh
cargo test --workspace                       # every crate's suite
PTY_TEST_BIN=target/release/pty cargo test -p pty-conformance   # black-box, any binary
```

The workspace tests drive real programs through real PTYs and real daemons,
with each test on its own `PTY_ROOT` under the temp dir. The conformance suite
runs the same way against whichever binary `PTY_TEST_BIN` names, so it can be
pointed at the Node `pty` to check the reference itself. Help texts and
completion scripts are vendored byte for byte from the Node repository
(`crates/pty/tests/fixtures/help/`, `completions/`) and the tests hold the
binary to them.

### Checking the macOS build without a Mac

`pty-core` deliberately has no Zig dependency, so it can be type-checked for
Apple silicon from any machine:

```sh
rustup target add aarch64-apple-darwin
cargo check -p pty-core --target aarch64-apple-darwin
cargo check -p pty      --target aarch64-apple-darwin
```

**This is worth running before you touch anything platform-specific.** It
caught a call to `pipe2`, which Linux has and macOS does not, and it produced
the same error a Mac did.

Both crates that hold platform-specific code are covered, and the check really
does compile the macOS branches — a deliberate error inside one is reported,
and the host build is unaffected by it. Running the whole workspace's TESTS
still needs a Mac.
