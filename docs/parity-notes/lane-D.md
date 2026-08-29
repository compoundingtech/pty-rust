# Lane D — help and completion vendoring, flake.nix, README (WP7 fixtures + WP-PKG)

Read lane-common.md first. Worktree name: `laneD`. Branch: `laneD`.

You own: `crates/pty/src/cli/help.rs`, `crates/pty/src/cli/completions.rs`, `crates/pty/tests/help.rs`,
`crates/pty/tests/completions.rs`, `crates/pty/tests/fixtures/help/**`, `completions/**` (repo root),
`flake.nix`, `flake.lock`, `README.md`, `.gitignore`. In `crates/pty/src/cli/mod.rs` add only the two lines
`pub mod help;` and `pub mod completions;` and wire `pty help`/`--help`/`-h` to `help::print_usage()` and
`pty completions <shell>` to `completions::run(args)` (the dispatcher is rewritten later; keep your edit to the
match arms). Do not implement per-command `--help` interception here (WP7 does).

1. Help vendoring. Generate the fixtures from the Node pty at 500eab2 by RUNNING it (the source of truth is
   what it prints): `pty help` → `crates/pty/tests/fixtures/help/usage.txt`; for every command with a
   COMMAND_HELP entry (see docs/parity-notes/node-cli-surface.md section 3.2 for
   the list: run attach exec peek send events list stats restart kill recover rm gc tag tag-multi emit rename
   metadata evidence up down test remote-serve completions) `pty <cmd> --help` → `fixtures/help/<cmd>.txt`;
   `pty tag-multi --help` (its own parser) → `tag-multi.txt`; `pty completions --help` → `completions.txt`.
   Capture stdout bytes exactly (trailing newline included). Then `help.rs`: `pub fn usage() -> &'static str`
   and `pub fn command_help(cmd: &str) -> Option<&'static str>` (aliases a→attach, ls→list, remove→rm) backed
   by `include_str!` of the fixtures, and `print_usage()`. For the three deferred commands (`recover`,
   `evidence`, `test`) keep the Node help text verbatim (drop-in), and note in README that the commands print
   "not available in this build" — do not invent new help.
   Test `crates/pty/tests/help.rs`: `pty help`, `pty --help`, `pty -h` stdout == usage fixture, exit 0;
   `pty version` matches `^0\.13\.\d+-rust\+[0-9a-f]{4,}$`. (Per-command `pty <cmd> --help` tests are added
   by WP7 when the dispatcher intercepts them; you may add them now marked `#[ignore]` with a note.)
2. Completions vendoring. Copy `completions/pty.fish`, `pty.bash`, `pty.zsh` from the Node checkout byte for
   byte into `completions/` at the repo root. `completions.rs`: `run(args)`: `--help`/`-h` → usage text
   (fixture `completions.txt`) to stdout, exit 0; no shell → usage to stderr, exit 2; unknown shell → stderr
   `pty completions: unknown shell: <shell>\n` + usage, exit 2; `fish|bash|zsh` → the vendored script to stdout
   (`include_str!`), exit 0. Test: byte equality with the files, the exit codes, and (tests/completions.test.ts)
   that every script mentions `--env` for run.
3. flake.nix (plan-core.md "WP9", plan-verify-libs.md "B6 Packaging first"). Pattern: st2's flake at
   <st2-checkout>/flake.nix lines 40-120 (`rustPlatform.buildRustPackage`,
   `cargoLock.lockFile = ./Cargo.lock`). Read `~/.cargo/registry/src/*/libghostty-vt-sys-0.2.1/build.rs` to learn
   how it fetches Ghostty (git clone of commit a887df42c56f6de86c0fe6da9c4eeca37931e083 unless
   `GHOSTTY_SOURCE_DIR` is set) and how zig fetches its own packages (`GHOSTTY_ZIG_SYSTEM_DIR` or the zig cache).
   Provide: a fixed-output `pkgs.fetchgit` of ghostty at that commit; a fixed-output derivation running
   `zig build --fetch` to populate the zig package dir (record both hashes); `nativeBuildInputs = [ zig_0_15 ]`
   (find the right attribute in the nixpkgs pinned by st2's flake); env `GHOSTTY_SOURCE_DIR`,
   `GHOSTTY_ZIG_SYSTEM_DIR`, `PTY_BUILD_SHA = self.shortRev or "dirty"`; `installShellCompletion` for the three
   files; `meta.mainProgram = "pty"`; `packages.default = pty`; `checks.completions` (generator output == files —
   since we vendor, this check runs `pty completions <shell>` and diffs); a `devShell` with rust + zig.
   Verify with `nix build .#default` on this host (nix is installed; if the sandbox blocks network, that is the
   point of the fixed-output derivations — iterate until it builds). Report the two hashes and the build time.
4. README.md: rewrite for the workspace: what pty is (one paragraph, stranger-readable), install (nix, cargo),
   the six crates with one line each, build requirements (Rust ≥ 1.88 edition 2024, Zig 0.15.2, first build
   fetches Ghostty), running tests (`cargo test --workspace`, `PTY_TEST_BIN=... cargo test -p pty-conformance`),
   the deferred commands (`recover`, `evidence`, `test` print a not-available message; link docs/parity.md §12),
   and links to docs/parity.md and docs/parity-plan.md. Keep the direction paragraph from the current README.
