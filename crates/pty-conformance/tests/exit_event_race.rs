//! Port of tests/exit-event-race.test.ts: the daemon flushes exactly one
//! `session_exit` (with a numeric `exitCode`) to `<id>.events.jsonl` before
//! it exits — whether the child ended on its own or the daemon was
//! SIGTERMed like `pty kill` does — and every daemon generation writes
//! exactly one `session_start`. Node launches `dist/server.js` directly;
//! here the daemon comes from `pty run -d`, and `keep=true` keeps the
//! events file past the exit-time self-reap where Node used it too.

use pty_conformance::*;
use serde_json::Value;
use std::time::Duration;

fn read_events(rig: &Rig, id: &str) -> Vec<Value> {
    let path = rig.root().join(format!("{id}.events.jsonl"));
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad event line {l:?}: {e}")))
        .collect()
}

fn daemon_pid(d: &Daemon) -> i32 {
    d.meta()["daemonPid"].as_i64().expect("daemonPid") as i32
}

fn wait_dead(pid: i32) {
    wait_until(&format!("daemon {pid} to exit"), || !pid_alive(pid));
}

/// node: tests/exit-event-race.test.ts:82
#[test]
fn captures_session_exit_when_child_exits_naturally() {
    let rig = Rig::new();
    let d = rig.daemon("xnat", &["true"], DaemonOpts::keep());
    wait_dead(daemon_pid(&d));
    let events = read_events(&rig, "xnat");
    let exits: Vec<&Value> = events.iter().filter(|e| e["type"] == "session_exit").collect();
    assert_eq!(exits.len(), 1, "{events:?}");
    assert!(exits[0]["exitCode"].is_number(), "{:?}", exits[0]);
}

/// node: tests/exit-event-race.test.ts:105
#[test]
fn captures_session_exit_when_daemon_is_killed_via_sigterm() {
    let rig = Rig::new();
    let d = rig.daemon("ksig", &["/bin/sh", "-c", "sleep 30"], DaemonOpts::no_display_name());
    std::thread::sleep(Duration::from_millis(200));
    let pid = d.pid();
    kill_pid(pid, libc::SIGTERM);
    wait_dead(pid);
    let events = read_events(&rig, "ksig");
    let exits = events.iter().filter(|e| e["type"] == "session_exit").count();
    assert_eq!(exits, 1, "{events:?}");
}

/// node: tests/exit-event-race.test.ts:124
#[test]
fn session_start_is_always_present() {
    let rig = Rig::new();
    let d = rig.daemon("sstart", &["true"], DaemonOpts::keep());
    wait_dead(daemon_pid(&d));
    let events = read_events(&rig, "sstart");
    let starts = events.iter().filter(|e| e["type"] == "session_start").count();
    assert_eq!(starts, 1, "{events:?}");
}
