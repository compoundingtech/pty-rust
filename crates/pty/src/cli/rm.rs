//! `pty rm <ref>` / `pty remove <ref>`: remove a session that is not
//! running from the registry — socket, pid, metadata, events, recovery
//! revision — under the creation lock with a generation check.
//!
//! node: src/cli.ts:1604-1613, 3036-3087 (`cmdRm`)

use std::time::Duration;

use pty_core::registry::{
    self, SessionGenerationOwner, SessionStatus, cleanup_owned_all, wait_for_process_exit,
};

use super::{CliError, CliResult, require_ref};

/// `cmdRm`.
pub fn run(args: &[String]) -> CliResult {
    let name = require_ref(args, "Usage: pty rm <name>")?;
    let Some(session) = registry::get_session(&name).map_err(CliError)? else {
        return Err(CliError(format!("Session \"{name}\" not found.")));
    };
    if session.status == SessionStatus::Running {
        return Err(CliError(format!(
            "Session \"{name}\" is still running. Use \"pty kill {name}\" first."
        )));
    }

    // `exited` means the child is gone, not necessarily the daemon: it keeps
    // its socket alive briefly so attached clients receive the exit packet,
    // then cleans up. Wait on the old generation's daemon so an immediate
    // same-name `pty run` cannot publish a socket the old daemon unlinks.
    let generation = session
        .metadata
        .as_ref()
        .and_then(|m| m.generation.clone());
    let daemon_pid = session
        .pid
        .or_else(|| session.metadata.as_ref().and_then(|m| m.daemon_pid))
        .or_else(|| registry::read_session_pid(&name));
    if let Some(pid) = daemon_pid
        && !wait_for_process_exit(pid, Duration::from_secs(7))
    {
        return Err(CliError(format!(
            "Session \"{name}\" daemon did not exit within 7s; not removed. Try again."
        )));
    }

    // Re-check generation ownership in the same critical section as the
    // unlink so a replacement that published meanwhile is left alone.
    let owner = SessionGenerationOwner {
        generation: generation.unwrap_or_default(),
        pid: daemon_pid.unwrap_or(-1),
    };
    if !cleanup_owned_all(&name, &owner) {
        return Err(CliError(format!(
            "Session \"{name}\" was replaced while waiting; new generation was not removed."
        )));
    }
    println!("Session \"{name}\" removed.");
    Ok(0)
}
