//! The per-session daemon: owns the PTY and a libghostty terminal actor,
//! serves the wire protocol over `<root>/<name>.sock`, publishes the
//! session's files, and records its exit — Node's `server.ts` with the
//! same ordering guarantees.
//!
//! - [`launch`]: how a CLI spawns one (`spawn_daemon`) and how the process
//!   starts (`daemon_main`, the `__daemon` argv).
//! - [`config`], [`env`]: the start-up config and the child's environment.
//! - [`lifecycle`]: the actor loop, publication, exit, shutdown.
//! - [`clients`], [`geometry`], [`status`], [`events`]: the packet handlers,
//!   effective geometry, STATUS, and the events log.
//! - [`tree`]: descendant termination on an external kill.

pub mod clients;
pub mod config;
pub mod env;
pub mod events;
pub mod geometry;
pub mod launch;
pub mod lifecycle;
pub mod status;
pub mod tree;

use std::path::PathBuf;

pub use config::DaemonConfig;
#[allow(unused_imports)]
pub use launch::{SpawnError, SpawnParams, SpawnedDaemon, set_process_title, spawn_daemon};

/// The `pty __daemon` entry: title, config from fd 3 (or
/// `PTY_SERVER_CONFIG`), then the daemon. Returns the process exit status.
///
/// node: src/server.ts:1458-1478
pub fn daemon_main() -> i32 {
    set_process_title("pty-daemon");
    let cfg = match DaemonConfig::from_process() {
        Ok(cfg) => cfg,
        Err(msg) => {
            eprintln!("{msg}");
            return 1;
        }
    };
    match lifecycle::run(cfg) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("{msg}");
            1
        }
    }
}

/// The default session working directory when none is given.
pub fn default_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}
