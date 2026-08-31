//! `pty kill <name>`: stop a session's daemon and keep its exit evidence.
//!
//! node: src/cli.ts:1384-1392 (dispatch), 2618-2671 (`cmdKill`)

use std::time::{Duration, Instant};

use pty_core::registry::{self, SessionStatus};

use super::{CliResult, require_ref};

/// How long the daemon gets to finish its shutdown. It re-flushes the exit
/// record on the way out, so returning early would let a following `pty rm`
/// race that write.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(7);

/// `cmdKill`.
pub fn run(args: &[String]) -> CliResult {
    let name = require_ref(args, "Usage: pty kill <name>")?;

    let Some(session) = registry::get_session_by_name(&name) else {
        eprintln!("Session \"{name}\" not found.");
        return Ok(1);
    };
    let (SessionStatus::Running, Some(pid)) = (session.status, session.pid) else {
        eprintln!("Session \"{name}\" is not running. Use \"pty rm {name}\" to remove it.");
        return Ok(1);
    };

    // Drop the `strategy` tag so `pty gc` does not start the session again
    // on its next pass.
    let tags = session.metadata.as_ref().and_then(|m| m.tags.as_ref());
    let was_permanent = tags.and_then(|t| t.get("strategy")).map(String::as_str) == Some("permanent");
    if was_permanent {
        let _ = registry::update_tags(&name, &Default::default(), &["strategy".to_string()]);
    }

    // SAFETY: kill(2) with a pid from the registry.
    if unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
        eprintln!("Failed to kill session \"{name}\".");
        return Ok(1);
    }

    if !wait_for_process_exit(pid, SHUTDOWN_WAIT) {
        // Leave the socket in place: it is the evidence of what is still
        // holding the session, and reporting success here would make the
        // next start look broken instead.
        eprintln!(
            "Failed to kill session \"{name}\": daemon PID {pid} is still running after 7s. \
             Socket {} may still be owned.",
            registry::socket_path(&name).display()
        );
        return Ok(1);
    }
    registry::cleanup_socket(&name);
    println!("Session \"{name}\" killed.");

    if was_permanent
        && let Some(path) = tags.and_then(|t| t.get("ptyfile"))
    {
        eprintln!("Note: this session is managed by {path}");
        eprintln!("The strategy tag will be restored on the next 'pty up'.");
    }
    Ok(0)
}

fn wait_for_process_exit(pid: i32, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if !registry::pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !registry::pid_alive(pid)
}

/// Stop a session's daemon without the reporting: an external SIGTERM, then
/// SIGKILL if it will not go. The daemon preserves the session either way.
pub(crate) fn kill_session(name: &str) {
    if let Some(pid) = registry::read_pid(name) {
        // SAFETY: kill(2) with a pid from the registry.
        unsafe { libc::kill(pid, libc::SIGTERM) };
        if !wait_for_process_exit(pid, Duration::from_secs(3)) {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}
