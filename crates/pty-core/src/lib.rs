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

pub mod client;
pub mod duration;
pub mod input;
pub mod keys;
pub mod paste;
pub mod protocol;
pub mod ptyfile;
pub mod queries;
pub mod registry;
pub mod stats;
