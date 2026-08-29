//! Port of tests/list-live-session-race.test.ts: `pty list` observes; it
//! never destroys a live session's files when the pid file is transiently
//! missing, and never reaps a dead one either (that is gc/rm's job).

use pty_conformance::*;
use std::time::Duration;

/// node: tests/list-live-session-race.test.ts:92
#[test]
fn live_session_survives_a_missing_pidfile() {
    let rig = Rig::new();
    let id = "live-norace";
    rig.daemon(id, &["cat"], DaemonOpts::no_display_name());
    std::fs::remove_file(rig.pid_path(id)).unwrap();
    let found = rig.list_entry(id).expect("live session listed");
    assert_eq!(found["status"], "running");
    assert!(rig.socket_path(id).exists(), "list reaped the live socket");
}

/// node: tests/list-live-session-race.test.ts:111
#[test]
fn dead_session_is_reported_vanished_without_reaping() {
    let rig = Rig::new();
    let id = "dead-reap";
    let d = rig.daemon(id, &["cat"], DaemonOpts::no_display_name());
    let pid = d.pid();
    kill_pid(pid, libc::SIGKILL);
    assert!(poll_for(Duration::from_secs(5), || !pid_alive(pid)));
    let found = rig.list_entry(id).expect("dead session listed");
    assert!(rig.socket_path(id).exists(), "list unlinked the stale socket");
    assert!(rig.pid_path(id).exists(), "list unlinked the stale pid file");
    assert_eq!(found["status"], "vanished");
}
