//! Port of tests/process-title.test.ts: the daemon names itself
//! `pty-daemon` so it is identifiable in ps/top. The only OS-visible proof
//! is `/proc/<pid>/comm`, so the test is Linux-only (as in Node).

use pty_conformance::*;

/// node: tests/process-title.test.ts:38
#[test]
fn daemon_process_comm_is_pty_daemon() {
    if !cfg!(target_os = "linux") {
        return;
    }
    let rig = Rig::new();
    let d = rig.daemon("title-test", &["sleep", "30"], DaemonOpts::no_display_name());
    let comm = std::fs::read_to_string(format!("/proc/{}/comm", d.pid())).unwrap();
    assert_eq!(comm.trim(), "pty-daemon");
}
