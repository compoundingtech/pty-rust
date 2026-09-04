//! Port of tests/remote-fabric.test.ts: `pty ls/peek/send/attach --remote
//! <peer>` over a stub `fabric` whose `dial` prints a local Unix socket.
//!
//! Node runs a persistent `pty remote-serve --socket <path>` behind the fake
//! fabric. That form is dropped (docs/parity.md §12: `--stdio` stays), so the
//! socket here is served by the exec-bridge in `remote_support` — one
//! `pty remote-serve --stdio` of the binary under test per connection, which
//! is what fabric's `expose --exec` does. The two cases about the
//! `--socket` daemon itself (lines 126, 157: survives a detached stdin=/dev/null
//! launch, ignores SIGHUP) have no counterpart without that form and are left
//! out.
//!
//! The local registry is the rig root (empty, so `local` is `[]`); the remote
//! registry is a second root under the rig.

mod remote_support;

use pty_conformance::*;
use remote_support::*;
use std::time::Duration;

/// The demo session of the Node `beforeAll`: `sleep 300` named "Demo Session".
fn remote_with_demo(rig: &Rig) -> Bridge {
    let srv = rig.make_root();
    let r = remote_run(rig, &srv, &["--id", "demo", "--name", "Demo Session", "--", "sleep", "300"]);
    expect_status(&r, 0);
    wait_remote_socket(&srv, "demo");
    Bridge::start(rig, &srv)
}

/// node: tests/remote-fabric.test.ts:86
#[test]
fn ls_remote_json_lists_the_peer_sessions_and_an_empty_local_root() {
    let rig = Rig::new();
    let _bridge = remote_with_demo(&rig);
    let out = rig.pty(&["ls", "--remote", "testpeer", "--json"]);
    let v = expect_json(&out);
    assert_eq!(v["local"], serde_json::json!([]), "{}", out.summary());
    let remote = v["remote"].as_array().expect("remote array");
    assert_eq!(remote.len(), 1);
    assert_eq!(remote[0]["label"], "testpeer");
    assert!(remote[0]["error"].is_null(), "{}", out.summary());
    let sessions = remote[0]["sessions"].as_array().expect("sessions");
    let demo = sessions.iter().find(|s| s["name"] == "demo").expect("demo listed");
    assert_eq!(demo["status"], "running");
    assert_eq!(demo["command"], "sleep 300");
    assert_eq!(demo["displayName"], "Demo Session");
}

/// node: tests/remote-fabric.test.ts:102
#[test]
fn ls_remote_renders_the_host_group_in_human_output() {
    let rig = Rig::new();
    let _bridge = remote_with_demo(&rig);
    let out = rig.pty(&["ls", "--remote", "testpeer"]);
    expect_status(&out, 0);
    let s = out.stdout();
    expect_contains(&s, "testpeer");
    expect_contains(&s, "Demo Session");
    expect_contains(&s, "sleep 300");
}

/// node: tests/remote-fabric.test.ts:110
#[test]
fn ls_remote_reports_a_dial_failure_as_a_host_group_error() {
    let rig = Rig::new();
    let _bridge = remote_with_demo(&rig);
    // A fabric that exits non-zero: the dial throws and the error is captured.
    let bad = rig.tmp().join("badfabric.sh");
    std::fs::write(&bad, "#!/bin/sh\nexit 3\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = rig.pty_env(
        &[("PTY_FABRIC_BIN", &bad.to_string_lossy())],
        &["ls", "--remote", "downpeer", "--json"],
    );
    let v = expect_json(&out);
    assert_eq!(v["remote"][0]["label"], "downpeer");
    let err = v["remote"][0]["error"].as_str().unwrap_or_default();
    assert!(!err.is_empty(), "expected an error string: {}", out.summary());
    assert_eq!(v["remote"][0]["sessions"], serde_json::json!([]));
}

/// node: tests/remote-fabric.test.ts:186
#[test]
fn peek_remote_shows_the_remote_screen() {
    let rig = Rig::new();
    let bridge = remote_with_demo(&rig);
    let r = remote_run(
        &rig,
        &bridge.srv_root,
        &["--id", "pk", "--", "sh", "-c", "printf 'PEEK_MARKER_9x\\r\\n'; sleep 300"],
    );
    expect_status(&r, 0);
    wait_remote_socket(&bridge.srv_root, "pk");
    std::thread::sleep(Duration::from_millis(500));
    let out = rig.pty(&["peek", "--remote", "testpeer", "pk", "--plain"]);
    expect_status(&out, 0);
    expect_contains(&out.stdout(), "PEEK_MARKER_9x");
}

/// node: tests/remote-fabric.test.ts:199
#[test]
fn peek_remote_of_a_missing_session_fails_with_not_found() {
    let rig = Rig::new();
    let _bridge = remote_with_demo(&rig);
    let out = rig.pty(&["peek", "--remote", "testpeer", "does-not-exist", "--plain"]);
    expect_failure(&out);
    expect_regex(&out.stderr(), "not found");
}

/// node: tests/remote-fabric.test.ts:205
#[test]
fn peek_remote_of_an_ambiguous_display_name_lists_the_stable_ids() {
    let rig = Rig::new();
    let bridge = remote_with_demo(&rig);
    for id in ["remote-a", "remote-b"] {
        let r = remote_run(
            &rig,
            &bridge.srv_root,
            &["--id", id, "--name", "Remote Shared", "--", "sleep", "300"],
        );
        expect_status(&r, 0);
        wait_remote_socket(&bridge.srv_root, id);
    }
    let out = rig.pty(&["peek", "--remote", "testpeer", "Remote Shared", "--plain"]);
    expect_failure(&out);
    let e = out.stderr();
    expect_contains(&e, "Session reference \"Remote Shared\" is ambiguous.");
    expect_contains(&e, "remote-a");
    expect_contains(&e, "remote-b");
}

/// node: tests/remote-fabric.test.ts:221
#[test]
fn send_remote_delivers_input_through_the_route_splice() {
    let rig = Rig::new();
    let bridge = remote_with_demo(&rig);
    let r = remote_run(&rig, &bridge.srv_root, &["--id", "sink", "--", "sh", "-c", "cat"]);
    expect_status(&r, 0);
    wait_remote_socket(&bridge.srv_root, "sink");
    std::thread::sleep(Duration::from_millis(300));
    let s = rig.pty(&[
        "send",
        "--remote",
        "testpeer",
        "sink",
        "--seq",
        "SEND_REMOTE_OK",
        "--seq",
        "key:return",
    ]);
    expect_status(&s, 0);
    std::thread::sleep(Duration::from_millis(400));
    let p = rig.pty(&["peek", "--remote", "testpeer", "sink", "--plain"]);
    expect_contains(&p.stdout(), "SEND_REMOTE_OK");
}

/// node: tests/remote-fabric.test.ts:237
#[test]
fn send_remote_to_a_missing_session_fails_with_not_found() {
    let rig = Rig::new();
    let _bridge = remote_with_demo(&rig);
    let out = rig.pty(&["send", "--remote", "testpeer", "does-not-exist", "--seq", "x"]);
    expect_failure(&out);
    expect_regex(&out.stderr(), "not found");
}

/// node: tests/remote-fabric.test.ts:245
#[test]
fn attach_remote_replays_the_screen_and_forwards_input() {
    let rig = Rig::new();
    let bridge = remote_with_demo(&rig);
    let sid = unique_id("shell-");
    let r = remote_run(
        &rig,
        &bridge.srv_root,
        &["--id", &sid, "--", "sh", "-c", "echo ATTACH_READY_MARK; cat"],
    );
    expect_status(&r, 0);
    wait_remote_socket(&bridge.srv_root, &sid);
    std::thread::sleep(Duration::from_millis(400));
    let mut t = rig.pty_tty_raw(&[], &[], &["attach", "--remote", "testpeer", &sid], 24, 80);
    assert!(
        t.wait_for_text("ATTACH_READY_MARK", Duration::from_secs(8)),
        "screen not replayed over the fabric hop: {:?}",
        t.output_str()
    );
    t.write(b"PING_OVER_ATTACH\r");
    assert!(
        t.wait_for_text("PING_OVER_ATTACH", Duration::from_secs(8)),
        "input not forwarded through the splice: {:?}",
        t.output_str()
    );
    t.kill();
}
