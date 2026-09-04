//! Black-box conformance suite that runs against any pty binary.
//!
//! The binary under test is chosen by `PTY_TEST_BIN` (absolute path); unset,
//! the workspace's own `target/<profile>/pty` is used. Run the suite against
//! the Node pty with `PTY_TEST_BIN=$(which pty) cargo test -p pty-conformance`
//! and against the Rust pty with no environment at all after `cargo build -p pty`.
//!
//! Every test carries a `/// node: tests/<file>.test.ts:<line>` doc comment
//! pointing at the Node test it ports; `docs/conformance.md` is generated from
//! those comments by the `conformance-map` bin.
//!
//! The only library code is the harness in [`harness`].

pub mod harness;

pub use harness::*;
