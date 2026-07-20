//! # pty-testkit
//!
//! A Rust port of the [pty](https://github.com/compoundingtech/pty) project's
//! Playwright-style TUI testing library, using **libghostty** (the Ghostty
//! terminal library, via [`libghostty-vt`](https://docs.rs/libghostty-vt)) as
//! the terminal-emulation backend in place of `@xterm/headless`.
//!
//! The core type is [`Session`]: spawn a process in a real PTY, feed its output
//! into a libghostty terminal, take text/ANSI "screenshots", wait for content,
//! and send named keys.
//!
//! It also ports several of the pty project's pure-logic modules
//! ([`keys`], [`duration`]) so their test suites can run unchanged against Rust.

pub mod client;
pub mod daemon;
pub mod duration;
pub mod input;
pub mod keys;
pub mod paste;
pub mod protocol;
pub mod ptyfile;
pub mod queries;
pub mod registry;
pub mod screenshot;
pub mod session;

pub use screenshot::Screenshot;
pub use session::{build_spawn_env, Session, SpawnOptions};
