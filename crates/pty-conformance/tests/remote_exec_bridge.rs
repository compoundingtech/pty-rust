//! Port of tests/remote-exec-bridge.test.ts: the on-demand `pty remote-serve
//! --stdio` end to end. A local socket spawns the handler per connection and
//! pipes socket <-> stdin/stdout (fabric's `expose --exec` deploy); the real
//! `--remote` client (list/peek/send) runs through it, and the handler must
//! exit when the interaction ends (the bridge in `remote_support` half-closes
//! the socket only once the handler's stdout hits EOF, so a handler that
//! lingered would hang every `list`).

mod remote_support;

use pty_conformance::*;
use remote_support::*;
use std::time::Duration;

/// The Node `beforeAll`: a `cat` session named "Demo" that printed EXEC_MARK.
fn remote_with_demo(rig: &Rig) -> Bridge {
    let srv = rig.make_root();
    let r = remote_run(
        rig,
        &srv,
        &["--id", "demo", "--name", "Demo", "--", "sh", "-c", "printf 'EXEC_MARK\\r\\n'; cat"],
    );
    expect_status(&r, 0);
    wait_remote_socket(&srv, "demo");
    std::thread::sleep(Duration::from_millis(500));
    Bridge::start(rig, &srv)
}

/// node: tests/remote-exec-bridge.test.ts:97
#[test]
fn list_remote_works_through_the_exec_bridge() {
    let rig = Rig::new();
    let _bridge = remote_with_demo(&rig);
    let out = rig.pty(&["ls", "--remote", "testpeer", "--json"]);
    let v = expect_json(&out);
    let names: Vec<&str> = v["remote"][0]["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(names.contains(&"demo"), "{}", out.summary());
}

/// node: tests/remote-exec-bridge.test.ts:104
#[test]
fn peek_remote_streams_the_screen_back_over_stdio() {
    let rig = Rig::new();
    let _bridge = remote_with_demo(&rig);
    let out = rig.pty(&["peek", "--remote", "testpeer", "demo", "--plain"]);
    expect_status(&out, 0);
    expect_contains(&out.stdout(), "EXEC_MARK");
}

/// node: tests/remote-exec-bridge.test.ts:110
#[test]
fn send_remote_delivers_input_over_stdio() {
    let rig = Rig::new();
    let _bridge = remote_with_demo(&rig);
    let s = rig.pty(&[
        "send",
        "--remote",
        "testpeer",
        "demo",
        "--seq",
        "STDIO_SEND_OK",
        "--seq",
        "key:return",
    ]);
    expect_status(&s, 0);
    std::thread::sleep(Duration::from_millis(400));
    let p = rig.pty(&["peek", "--remote", "testpeer", "demo", "--plain"]);
    expect_contains(&p.stdout(), "STDIO_SEND_OK");
}

/// node: tests/remote-exec-bridge.test.ts:118
#[test]
fn peek_remote_of_a_missing_session_is_a_clean_not_found() {
    let rig = Rig::new();
    let _bridge = remote_with_demo(&rig);
    let out = rig.pty(&["peek", "--remote", "testpeer", "does-not-exist", "--plain"]);
    expect_failure(&out);
    expect_regex(&out.stderr(), "not found");
}
