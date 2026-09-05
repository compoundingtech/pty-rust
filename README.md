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

## Where it stands

**It is the daily driver on two machines** — an arm64 Mac and an x86_64 Linux
box — and has been since 2026-09-02. That is the honest measure of "does it
work": it is what those machines run, not a demo.

**The documented command surface matches the Node tool exactly.** Both
`pty help` outputs are 122 lines and every command appears in both. Three are
deferred rather than missing — `pty recover`, `pty evidence` and `pty test`
keep their help text and print
`pty <cmd>: not available in this build. See docs/parity.md.`

**1357 tests pass**, including a conformance suite that runs against *either*
binary. That is the part worth knowing: the two implementations are held to the
same assertions rather than compared by hand, so a behavioural difference fails
a test instead of surfacing later on somebody's machine.

**What is not finished** is written down rather than implied: the surface-by-
surface state is in [docs/parity.md](docs/parity.md), the decisions where the
two tools deliberately differ are in [docs/decisions/](docs/decisions/), and the
limits worth knowing before you rely on it are
[below](#one-known-defect-documented-rather-than-fixed).

**There are no prebuilt binaries yet.** You build it — see
[Install](#install) — and [Which systems it runs
on](#which-systems-it-runs-on) says what a built binary needs.

## Direction: compatibility and embedding

The long-term target is a behavior-compatible Rust implementation of the Node
`pty`, plus a first-class Rust API for embedding a live terminal in clients such
as Fractal. The Node implementation is the behavioral reference while the port
converges. **This README does not claim full parity**, and
[docs/parity.md](docs/parity.md) is where the remaining gaps are named surface by
surface.

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

## Which systems it runs on

**There are no prebuilt binaries yet.** Today you build it, with Nix or Cargo.
This section says what a built binary needs, because that is the question a
release has to answer.

### Linux: glibc 2.34 or newer, and the floor depends on where you build

A binary links against the glibc it was built with, and refuses to start on
anything older:

```
$ ./pty --version
./pty: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.39' not found (required by ./pty)
```

**The floor is not set by this codebase.** It is set by two symbols the Rust
standard library uses for process spawning, `pidfd_getpid` and `pidfd_spawnp`,
which appear in any Rust program that calls `Command::status()`. Nothing here
references them.

That means the build host decides the floor, and the difference is large:

| built on | floor | runs on |
| --- | --- | --- |
| glibc 2.43 (Ubuntu 25.10) | `GLIBC_2.39` | Ubuntu 24.04+, Debian 13+, Fedora 40+ |
| glibc 2.36 (Debian 12) | `GLIBC_2.34` | the above, plus Ubuntu 22.04, Debian 12, RHEL 9, Rocky 9, Amazon Linux 2023 |

Measured on 2026-09-05 by running the same binary in each distribution's
container. **Build releases on the oldest glibc you intend to support**; no
source change is involved.

### macOS: it runs, and here is exactly what was tested

**Gatekeeper does not refuse the binary.** Tested on **one** machine: arm64,
macOS 26.6, build 25G72, Darwin 25.6.0, with Gatekeeper assessments enabled.

The `com.apple.quarantine` attribute was written onto the real binary, confirmed
present, and the binary ran and exited 0. The attribute was then removed and the
run repeated with an identical result — **without that control the first run
would prove nothing about the attribute.**

Three conditions travel with that result:

- **It is one macOS version.** A bare "Gatekeeper does not apply" would outlive
  the release it was true of.
- **The binary carried an ad-hoc linker signature**, which macOS applies
  automatically on Apple silicon. That may be why it passed. A release built
  somewhere that strips or omits it is a different case, and an untested one, so
  **the release process must preserve it.**
- **This result does not transfer to a published asset.** The same test, with
  its removal control, has to run against the first one we ship.

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

### If you are already inside a session

**`run` and `attach` refuse to nest, and `--force` is how you say you meant
it.** A session inside a session is usually a mistake — a detach key press then
reaches the wrong one — so both commands stop and explain instead.

```sh
pty attach --force API      # attach from inside another session
pty run --force -- <cmd>    # create one from inside another session
```

**The refusal goes to standard error and exits 1**, in this tool and in the
Node one. A script that captures only standard output sees an empty result
and can mistake that for success, which is what happens if you forget
`--force`; the exit code tells you the truth.

`run` and `restart` are different and deliberately so: from inside a session
`run` runs the command directly and `restart` restarts without attaching.
Both did what was asked, so both exit 0.

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

### Reading a single failure from a full run

**One test failing in a whole-workspace run is not yet a defect. Re-run it
alone before treating it as one.**

```sh
cargo test -p <crate> --test <binary> -- --exact --test-threads=1 <name>
```

The suite runs 139 binaries in parallel, and each one drives real processes
through real terminals. On a machine slow enough, one or two of them lose a
race per run — **and which ones varies across the whole suite**, so a name you
have never seen before is the normal case rather than a new regression.

**Measured on 2026-09-02, and the two machines differ sharply.** Seventeen
whole-workspace runs on one Linux host: fifteen completely clean, and the two
that were not each named a real defect that was then fixed. No run there lost
a race. Four runs on an Apple silicon laptop: one or two lost races every
time, never the same ones, all green when run alone.

**So do not chase these by name.** A failure worth fixing has a cause you can
state — the two in this repository's history that looked like this both did: a
sleep standing in for a handshake, in a test that then failed reliably once
the timing was turned up. **A name that passes alone and has no such cause is
a scheduling accident**, and hunting them one at a time is unbounded work.

Whether a slower machine is the whole explanation is not established; there is
one laptop and nothing to compare it against.

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
