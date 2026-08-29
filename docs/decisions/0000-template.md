# NNNN <title>

Status: accepted | superseded by NNNN

Node behavior: what the Node pty (0.12.0+500eab2) does, with the source line.

Rust behavior: what the Rust pty does instead.

Why: the reason the difference is kept rather than hidden.

Client effect: what a client, a script, or a fleet consumer can observe.

Test: conformance/tests/<file>.rs::<fn> (gated `_node` / `_rust` pair) — and
the fixture under crates/pty-conformance/fixtures if any.

Migration / negotiation: none | what a consumer has to change, or how the
two sides negotiate.

---

Rules:

- One record per observable difference. A difference the CLI can hide
  (query answers, trimming) is fixed in Rust, not recorded.
- The record exists only once a gated test proves the difference on both
  binaries: `PTY_TEST_BIN=$(which pty) cargo test -p pty-conformance` green
  and `cargo test -p pty-conformance` green.
- Number records in order; never reuse a number. Superseding a record keeps
  the old file with `Status: superseded by NNNN`.
