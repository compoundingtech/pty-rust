//! Port of tests/wrapper-signal-forwarding.test.ts. Node's case runs
//! `remote-serve --socket <path>` through `bin/pty` and checks that the
//! launcher spawns no child and exits 0 on SIGTERM; `--socket` is dropped
//! (docs/parity.md §12), so the still-existing shape is pinned instead: a
//! long-lived CLI command (`peek --wait`) is a single process with no child
//! of its own that dies on SIGTERM (Node has no handler: killed by the
//! signal), and `remote-serve --stdio` serves one `{"op":"list"}` request
//! and exits 0.

use pty_conformance::*;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

fn children_of(pid: u32) -> String {
    Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// node: tests/wrapper-signal-forwarding.test.ts:50
#[test]
fn long_lived_command_is_one_process_that_dies_on_sigterm() {
    let rig = Rig::new();
    rig.daemon("ws", &["cat"], DaemonOpts::no_display_name());
    let mut cmd = rig.command(&["peek", "--wait", "NEVER", "-t", "30", "ws"]);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    std::thread::sleep(Duration::from_millis(700));
    assert!(child.try_wait().unwrap().is_none(), "peek --wait exited early");
    let kids = children_of(child.id());
    assert_eq!(kids, "", "the CLI spawned a child: {kids}");
    kill_pid(child.id() as i32, libc::SIGTERM);
    assert!(poll_for(Duration::from_secs(5), || child.try_wait().map(|s| s.is_some()).unwrap_or(true)));
    let status = child.wait().unwrap();
    // Node installs no SIGTERM handler for a waiting peek: the process is
    // killed by the signal, so there is no exit code.
    assert!(!status.success(), "{status:?}");
    let mut err = String::new();
    let _ = child.stderr.take().unwrap().read_to_string(&mut err);
    expect_not_contains(&err, "Timed out");
}

/// node: tests/wrapper-signal-forwarding.test.ts:50
#[test]
fn remote_serve_stdio_answers_one_list_request_and_exits() {
    let rig = Rig::new();
    rig.daemon("rs", &["sleep", "300"], DaemonOpts::no_display_name());
    let out = rig.pty_stdin(b"{\"op\":\"list\"}\n", &["remote-serve", "--stdio"]);
    expect_status(&out, 0);
    let line = out.stdout();
    let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap_or_else(|e| panic!("{e}: {line}"));
    let sessions = v["sessions"].as_array().expect("sessions array");
    let rs = sessions.iter().find(|s| s["name"] == "rs").expect("rs listed");
    assert_eq!(rs["status"], "running");
    assert_eq!(rs["command"], "sleep 300");
    assert_eq!(line.matches('\n').count(), 1, "{line:?}");
}
