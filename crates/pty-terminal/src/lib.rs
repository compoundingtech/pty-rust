//! # pty-terminal
//!
//! The libghostty-backed terminal side of the Rust
//! [pty](https://github.com/compoundingtech/pty) port. Today this is
//! [`screenshot`]: capturing a [`Screenshot`] (plain lines, joined text, VT
//! serialization) from a `libghostty_vt::terminal::Terminal`, and the replay
//! serialization the daemon sends on attach.
//!
//! The terminal actor (single-threaded owner of the `!Send` `Terminal`) and
//! the `TerminalHandle` embedding API arrive in a later work package (see
//! docs/parity-plan.md, WP4). Until then callers own the `Terminal` themselves
//! and call the free functions here.

pub mod screenshot;

pub use screenshot::{Screenshot, capture, serialize_for_replay};
