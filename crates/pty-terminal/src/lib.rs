//! # pty-terminal
//!
//! The libghostty-backed terminal side of the Rust
//! [pty](https://github.com/compoundingtech/pty) port: one owner of the
//! `!Send` `libghostty_vt::terminal::Terminal`, typed reads, Node-equivalent
//! serialization, terminal query answers, and terminal events.
//!
//! - [`actor`]: [`TerminalActor`], the synchronous owner of the terminal. Feed
//!   it the child's output with [`TerminalActor::write`]; read the screen with
//!   [`TerminalActor::plain`], [`TerminalActor::serialize`], and
//!   [`TerminalActor::snapshot`]; drain query answers with
//!   [`TerminalActor::take_pty_replies`] and events with
//!   [`TerminalActor::take_events`].
//! - [`strip`]: the streaming scanner that keeps terminal queries out of the
//!   DATA broadcast and tracks the mode flags Node tracks.
//! - [`queries`]: the exact answers Node gives to DA1/DA2/DSR/XTVERSION and the
//!   OSC 10/11/4 colour queries.
//! - [`serialize`]: the ATTACH/PEEK replay (Node mode prefix + VT) and the
//!   plain-text screen (viewport or full scrollback).
//! - [`snapshot`]: [`CellGrid`], the typed cell grid for renderers.
//! - [`handle`]: [`TerminalHandle`], a `Send + Sync` handle over an actor
//!   thread, either spawning a child or attaching to a session daemon.
//! - [`screenshot`]: the testkit's [`Screenshot`] capture.

pub mod actor;
pub mod handle;
pub mod queries;
pub mod screenshot;
pub mod serialize;
pub mod snapshot;
pub mod strip;

pub use actor::{Modes, Notification, Range, TerminalActor, TerminalEvent};
pub use handle::{
    AttachOptions, AttemptId, HandleEvent, SessionRef, SpawnOptions, TerminalHandle,
};
pub use screenshot::{Screenshot, capture, serialize_for_replay};
pub use serialize::SerializeOpts;
pub use snapshot::{CellGrid, CellSnap, ColorSnap, Wide};
