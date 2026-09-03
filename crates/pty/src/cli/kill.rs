//! `pty kill <name>`: stop a session's daemon and keep its exit evidence.
//!
//! node: src/cli.ts:1384-1392 (dispatch), 2618-2671 (`cmdKill`)

use std::time::{Duration, Instant};

use pty_core::registry::{self, SessionStatus};

use crate::daemon::tree::{ProcessIdentity, snapshot_descendant_processes};

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

    // Take the tree BEFORE the signal. After the daemon exits its children are
    // reparented to init, so the parent links that identify them as this
    // session's processes are gone. This snapshot is the only chance to learn
    // which processes the word "killed" would be a claim about.
    let before = snapshot_descendant_processes(pid);

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
    let after = aftermath(&before);
    report(&name, &after);

    if was_permanent
        && let Some(path) = tags.and_then(|t| t.get("ptyfile"))
    {
        eprintln!("Note: this session is managed by {path}");
        eprintln!("The strategy tag will be restored on the next 'pty up'.");
    }
    // Anything left is a failure, and the status says so. `unknown` counts:
    // "I could not confirm the tree is empty" is not success, and a caller
    // that reads 0 as done would be wrong.
    Ok(if after.all_gone() { 0 } else { 1 })
}

/// What the pre-kill snapshot looks like once the daemon has gone.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Aftermath {
    /// The start token still matches, so this is the same process and it is
    /// still running.
    pub survived: Vec<i32>,
    /// The pid has not exited but its start token could not be read. We cannot
    /// tell whether it is the same process or a pid the kernel has reused.
    ///
    /// This case gets its own list rather than joining either side. Folding it
    /// into `survived` would invent a survivor; dropping it would repeat the
    /// defect this whole change exists to remove, which is a failure to
    /// measure reported as an answer.
    pub unknown: Vec<i32>,
}

impl Aftermath {
    pub fn all_gone(&self) -> bool {
        self.survived.is_empty() && self.unknown.is_empty()
    }
}

/// Re-check a snapshot against the live process table.
///
/// `exited` must be `has_process_exited_for_reap` rather than `!pid_alive`.
/// A zombie answers `kill(pid, 0)` and keeps a readable start token, so the
/// two cheaper predicates both call it a survivor. It is a dead process
/// waiting to be reaped, and reporting it as still running would be this
/// command over-claiming again, only in the other direction. Measured on
/// Linux 2026-09-03: state `Z`, `/proc/<pid>/stat` readable, token unchanged,
/// `kill(pid, 0)` succeeds.
pub(crate) fn aftermath_with(
    before: &[ProcessIdentity],
    read_token: impl Fn(i32) -> Option<String>,
    exited: impl Fn(i32) -> bool,
) -> Aftermath {
    let mut out = Aftermath::default();
    for id in before {
        if exited(id.pid) {
            continue;
        }
        match read_token(id.pid) {
            Some(token) if token == id.process_start_token => out.survived.push(id.pid),
            // A different token is a pid the kernel handed to somebody else.
            Some(_) => {}
            None => out.unknown.push(id.pid),
        }
    }
    out
}

fn aftermath(before: &[ProcessIdentity]) -> Aftermath {
    aftermath_with(
        before,
        registry::read_process_start_token,
        registry::has_process_exited_for_reap,
    )
}

/// Say what was verified, and nothing more.
///
/// `killed` is now a claim about the whole tree, so it is printed only when
/// every process in the snapshot is gone. Otherwise stdout gets the part that
/// was verified — the daemon stopped — and stderr gets what survived it.
fn report(name: &str, after: &Aftermath) {
    if after.all_gone() {
        println!("Session \"{name}\" killed.");
        return;
    }
    println!("Session \"{name}\" daemon stopped.");
    if !after.survived.is_empty() {
        eprintln!(
            "Session \"{name}\": {} process(es) survived the kill and are still running: {}",
            after.survived.len(),
            join_pids(&after.survived)
        );
    }
    if !after.unknown.is_empty() {
        eprintln!(
            "Session \"{name}\": {} process(es) may still be running: {}. \
             Their start tokens could not be read, so this is not a conclusion.",
            after.unknown.len(),
            join_pids(&after.unknown)
        );
    }
}

fn join_pids(pids: &[i32]) -> String {
    pids.iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn id(pid: i32, token: &str) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            process_start_token: token.to_string(),
            depth: 1,
        }
    }

    #[test]
    fn a_matching_token_is_a_survivor() {
        let before = vec![id(10, "tok:10")];
        let after = aftermath_with(&before, |_| Some("tok:10".into()), |_| false);
        assert_eq!(after.survived, vec![10]);
        assert!(after.unknown.is_empty());
        assert!(!after.all_gone());
    }

    #[test]
    fn a_reused_pid_is_not_a_survivor() {
        let before = vec![id(10, "tok:10")];
        let after = aftermath_with(&before, |_| Some("tok:different".into()), |_| false);
        assert!(after.all_gone());
    }

    #[test]
    fn a_gone_process_is_gone() {
        let before = vec![id(10, "tok:10")];
        let after = aftermath_with(&before, |_| None, |_| true);
        assert!(after.all_gone());
    }

    /// The whole point of the `unknown` list: a pid we can see but cannot
    /// identify is reported as undecided, never silently as dead.
    #[test]
    fn an_unreadable_token_on_a_live_pid_is_undecided() {
        let before = vec![id(10, "tok:10")];
        let after = aftermath_with(&before, |_| None, |_| false);
        assert!(after.survived.is_empty());
        assert_eq!(after.unknown, vec![10]);
        assert!(!after.all_gone());
    }

    #[test]
    fn an_empty_snapshot_is_all_gone() {
        assert!(aftermath_with(&[], |_| None, |_| false).all_gone());
    }

    /// The mocked cases above prove the branching. This one proves the
    /// branching is about real processes: it spawns one, classifies it with
    /// the real token reader and the real liveness check, kills it, and
    /// classifies it again.
    #[test]
    fn a_real_process_is_classified_from_the_real_process_table() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id() as i32;
        let token = registry::read_process_start_token(pid).expect("live pid has a start token");
        let before = vec![id(pid, &token)];

        let alive = aftermath_with(
            &before,
            registry::read_process_start_token,
            registry::has_process_exited_for_reap,
        );
        assert_eq!(alive.survived, vec![pid], "a running process reads as a survivor");
        assert!(alive.unknown.is_empty());

        let _ = child.kill();
        let _ = child.wait();

        let dead = aftermath_with(
            &before,
            registry::read_process_start_token,
            registry::has_process_exited_for_reap,
        );
        assert!(
            dead.all_gone(),
            "a reaped process must not read as a survivor, got {dead:?}"
        );
    }

    /// A zombie is a dead process that still answers `kill(pid, 0)` and still
    /// has a matching start token. Reporting it as a surviving process would
    /// be a false alarm, so this test uses a real one: Rust does not reap a
    /// `Child` until something waits on it.
    #[test]
    fn a_real_zombie_is_not_a_survivor() {
        let mut child = std::process::Command::new("true").spawn().expect("spawn true");
        let pid = child.id() as i32;
        let token = registry::read_process_start_token(pid).expect("live pid has a start token");
        let before = vec![id(pid, &token)];

        // Let it exit. It stays a zombie because nothing has waited on it.
        for _ in 0..200 {
            if registry::has_process_exited_for_reap(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            registry::pid_alive(pid),
            "precondition: an unreaped zombie still answers kill(pid, 0)"
        );
        assert_eq!(
            registry::read_process_start_token(pid).as_deref(),
            Some(token.as_str()),
            "precondition: a zombie keeps its start token, so the token alone cannot decide"
        );

        let after = aftermath(&before);
        assert!(after.all_gone(), "a zombie must not be reported, got {after:?}");

        let _ = child.wait();
    }

    #[test]
    fn pids_render_in_snapshot_order() {
        assert_eq!(join_pids(&[300, 200, 100]), "300, 200, 100");
    }
}
