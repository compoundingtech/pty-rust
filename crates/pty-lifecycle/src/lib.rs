//! Daemon launch and registry lifecycle operations for `pty` sessions.
//!
//! This crate contains the importable, terminal-free lifecycle surface shared
//! by the `pty` binary and library callers. Callers choose the executable that
//! implements the private `__daemon` entrypoint when spawning.

mod config;
mod gc;
mod launch;

pub use config::DaemonConfig;
pub use gc::{
    Abandoned, Flapped, GcOptions, GcResult, OrphanKill, PrunedTags, ReapSkip, RespawnFailed,
    Respawned, gc, prune_orphan_layout_tags,
};
pub use launch::{
    DEFAULT_START_TIMEOUT, READY_FD_ENV, ReadyNotifier, SpawnError, SpawnParams, SpawnedDaemon,
    apply_persisted_launch_options, set_process_title, spawn_daemon,
};
pub use pty_core::registry::{
    LockBusy, SessionGenerationOwner, cleanup_all, cleanup_owned_all, cleanup_owned_socket,
    cleanup_socket, wait_for_process_exit,
};
