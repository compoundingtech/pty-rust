# Common rules for every parity lane

Repository: <this-repository>. Integration branch: `parity`.
Plan: docs/parity-plan.md (read your work package section and "Shape decisions" first).
Map: docs/parity.md (the behavior inventory and the decisions; section 12 = dropped/deferred).
Node reference checkout (read-only): <node-pty-checkout> at 500eab2.
Inventories with file:line citations into the Node source and its tests:
  docs/parity-notes/node-cli-surface.md
  docs/parity-notes/node-daemon-protocol-disk.md
  docs/parity-notes/node-testing-tui.md
  docs/parity-notes/rust-port-and-st2.md
  docs/parity-notes/plan-core.md   (design detail for WP1-WP9)
  docs/parity-notes/plan-verify-libs.md (design detail for WP-CONF/KIT/TS/TUI/CUT)

Worktree: `cd <this-repository> && git worktree add <worktrees>/<lane> -b <lane> parity`.
Work ONLY in your worktree. Never touch the main checkout or another lane's worktree. Never `git stash`. Do not push.
Toolchain: Rust edition 2024 (>= 1.88), Zig 0.15.2 on PATH (libghostty-vt-sys builds Ghostty's VT core, cached after the first build).
The Node `pty` binary is on PATH (`pty --version` prints `0.12.0+500eab2`); use it as the oracle whenever a text or shape is in doubt: run it under a temp `PTY_ROOT` and compare.

File ownership: touch only the paths your brief names. If you must change a shared file (root Cargo.toml, a crate's lib.rs to add a module), keep the change to the single line that registers your module, so merges stay clean.

Behavior bar: byte-for-byte texts, exit codes, JSON key order and shapes as the inventories pin them. When the Node behavior and the inventory disagree, the Node source wins; note it in your report.
Where a difference cannot be hidden by the CLI (a libghostty vs xterm rendering difference), write a decision record docs/decisions/NNNN-<slug>.md (template in plan WP-CONF) and gate the test; do not silently accept it.

Tests: every behavior you implement gets a test. Cite the Node test in a doc comment: `/// node: tests/<file>.test.ts:<line>`.
Run `cargo test --workspace` before you finish; paste the `test result:` lines.

Commits: on your lane branch, messages written for a stranger (plain words, what changed and why, no agent names, no plan references), each ending with the line
`Claude-Session: <the session link the harness adds>`.
Report: final commit SHA, test summary, decisions you made, anything left undone and why.
