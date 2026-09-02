//! # pty-core
//!
//! The terminal-free half of the Rust [pty](https://github.com/compoundingtech/pty)
//! port: the wire protocol, the on-disk session registry, the client
//! operations (peek / send / status / attach), and the pure-logic modules
//! ported from the Node project ([`keys`], [`paste`], [`duration`], [`input`],
//! [`queries`], [`ptyfile`]).
//!
//! This crate deliberately does not depend on libghostty, so it builds without
//! a Zig toolchain. Terminal emulation lives in `pty-terminal`; the daemon and
//! CLI live in the `pty` binary crate.
//!
//! # One thing to know before you build on this
//!
//! **The registry's file locks are not exclusive across a crash.** They keep
//! two live, healthy processes apart, which is what they are for. They do not
//! settle a race between two processes tidying up after a daemon that died
//! holding one: both can end up believing they hold it. The Node tool has the
//! same defect, so a shared `$PTY_ROOT` is no worse than either alone.
//! [`registry::lock`] states the measurement and what a correct fix would
//! need.

pub mod client;
pub mod duration;
pub mod events;
pub mod input;
pub mod keys;
pub mod paste;
pub mod protocol;
pub mod ptyfile;
pub mod queries;
pub mod registry;
pub mod spawn;
pub mod stats;
