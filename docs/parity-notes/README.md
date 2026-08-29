# Parity build notes

These files are the working notes behind the Rust pty parity build. They were written on
2026-08-29 while the build ran, and they are kept here so that the branches `parity`,
`parity-wp5-daemon`, `parity-conformance-pass-2` and `parity-wptui-ratatui` can be picked up
by somebody who did not write them. The plan they follow is `docs/parity-plan.md` and the
behavior inventory is `docs/parity.md`, both on the `parity` branch.

## What is here

| File | What it is |
|---|---|
| `lane-common.md` | The rules every lane brief starts with: repository layout, toolchain, file ownership, the behavior bar, tests, commit style. |
| `lane-A.md` … `lane-E.md` | The briefs for lanes A (registry and events), B (terminal actor), C (client ops), D (help, completions, flake, README) and E (conformance harness). All five are merged into `parity`. |
| `lane-WP5.md`, `lane-WP7a.md`, `lane-WP7b.md`, `lane-WP8.md` | The briefs for the daemon, the CLI resolve and ask helpers, the socket verbs, and the remaining verbs. WP7a is merged. WP5 is on `parity-wp5-daemon`. WP7b and WP8 were not started. |
| `lane-WPKIT.md`, `lane-WPTS.md`, `lane-WPTUI.md` | The briefs for the Rust testkit, the TypeScript testing package, and the TUI on ratatui. WP-TUI's library and widgets are on `parity-wptui-ratatui`; its session manager is not started. WP-KIT and WP-TS were not started. |
| `plan-core.md` | Design detail for WP1 to WP9: crate layout, the actor model, the registry, the protocol. |
| `plan-verify-libs.md` | Design detail for the conformance suite, the testkit, the TypeScript package, the TUI, and the cutover. |
| `node-cli-surface.md` | An inventory of the Node pty's CLI: every verb, flag, text, exit code and JSON shape, with file and line citations into the Node source and its tests. |
| `node-daemon-protocol-disk.md` | The same for the daemon, the socket protocol and the on-disk layout under `PTY_ROOT`. |
| `node-testing-tui.md` | The same for the Node testing package and the TUI. |
| `rust-port-and-st2.md` | What the earlier Rust port already did, how st2 calls pty, and how the memory and CPU comparison was made. |
| `laneE-rust-red.txt`, `laneE2-rust-red.txt` | The conformance tests that were red against the Rust binary after the first and second passes. `scripts/conformance-both.sh` regenerates them. |
| `open-items.md` | Decisions the build raised for the maintainer. None of them blocks the build. |

## How to read the paths

The briefs were written for one machine. Its paths are replaced with placeholders:

- `<this-repository>` is a checkout of this repository.
- `<worktrees>/<lane>` is a git worktree of this repository for one lane.
- `<node-pty-checkout>` is a read-only checkout of the Node pty at `500eab2`. The Node `pty`
  binary from that version (`pty --version` prints `0.12.0+500eab2`) must be on `PATH`; the
  conformance suite uses it as the oracle.
- `<st2-checkout>` is a checkout of st2, the supervisor that spawns pty sessions.
- `<pty-layout-checkout>` is a checkout of the pty-layout program that the TUI notes cite.

The briefs also assume Zig 0.15.2 on `PATH` and a Rust toolchain of 1.88 or later.

## What is not here

- The raw conformance run log. It is large and the red lists above summarise it.
- The briefs mention a staged rollout across several hosts. The host names are removed; the
  order of the steps is what matters.
