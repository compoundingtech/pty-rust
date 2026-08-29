//! `pty kill` — interim port kept from the v0 binary. The Node-exact
//! rewrite (cli.ts:1384-1392, `cmdKill` 2618-2671) replaces this module.

use std::time::Duration;

use pty_core::registry;

use super::{CliResult, require_ref};

/// `pty kill <ref>`
pub fn run(args: &[String]) -> CliResult {
    let name = require_ref(args, "Usage: pty kill <name>")?;
    kill_session(&name);
    println!("Session \"{name}\" killed.");
    Ok(0)
}

/// SIGTERM a session's daemon (an EXTERNAL stop). The daemon forwards SIGHUP
/// to the child, escalates to SIGKILL via its watchdog if needed, then
/// PRESERVES the session (status=exited) unless it is ephemeral. Waits for
/// the daemon to finish its clean shutdown so the exit metadata is written.
/// Does not remove the session from the registry; that is `rm`.
pub(crate) fn kill_session(name: &str) {
    if let Some(pid) = registry::read_pid(name) {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if !registry::pid_alive(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if registry::pid_alive(pid) {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}
