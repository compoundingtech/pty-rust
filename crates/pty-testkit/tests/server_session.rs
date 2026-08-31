//! Server mode: the testkit driving a session a `pty` daemon owns.
//!
//! These need the `pty` binary. `PTY_BIN` names it; the workspace build sets
//! that below so the tests use the binary from this tree rather than
//! whatever is on PATH.
//!
//! node: the server-mode half of tests/screenshot.test.ts

use std::time::Duration;

use pty_testkit::{ServerOptions, Session};

/// Point the testkit at the `pty` built from this workspace.
fn use_local_pty() {
    // CARGO_BIN_EXE_ is only set for the crate that owns the binary, so find
    // it beside this test binary instead.
    let mut dir = std::env::current_exe().expect("test binary path");
    dir.pop(); // deps/
    dir.pop(); // debug/
    let bin = dir.join("pty");
    if bin.exists() {
        // SAFETY: set before any thread that reads it; the tests run this
        // first and never change it afterwards.
        unsafe { std::env::set_var("PTY_BIN", &bin) };
    }
}

fn server(command: &str, args: &[&str]) -> Session {
    use_local_pty();
    Session::server(
        command,
        args,
        ServerOptions {
            rows: Some(24),
            cols: Some(80),
            ..Default::default()
        },
    )
    .expect("start a server-mode session")
}

#[test]
fn it_shows_what_the_session_printed_before_the_client_arrived() {
    let mut s = server("sh", &["-c", "printf 'BEFORE-ATTACH\\n'; exec cat"]);
    // The daemon replays the screen on attach, so output from before this
    // client existed is still there.
    s.wait_for_text("BEFORE-ATTACH", 8000)
        .expect("the replay carried the earlier output");
    s.close();
}

#[test]
fn it_sends_input_and_sees_the_answer() {
    let mut s = server("sh", &["-c", "stty -echo; while read line; do echo \"got:$line\"; done"]);
    std::thread::sleep(Duration::from_millis(200));
    s.type_str("hello\r");
    s.wait_for_text("got:hello", 8000).expect("the session answered");
    s.close();
}

#[test]
fn reconnecting_rebuilds_the_screen_from_the_daemon() {
    let mut s = server("sh", &["-c", "printf 'STAYS-ON-SCREEN\\n'; exec cat"]);
    s.wait_for_text("STAYS-ON-SCREEN", 8000).expect("first client");

    s.reconnect().expect("reconnect");
    // A fresh terminal, filled only by the daemon's replay.
    s.wait_for_text("STAYS-ON-SCREEN", 8000)
        .expect("the replay rebuilt the screen");
    s.close();
}

#[test]
fn a_second_client_sees_the_same_screen_and_the_smaller_size_wins() {
    let mut first = server("sh", &["-c", "printf 'SHARED-LINE\\n'; exec cat"]);
    first.wait_for_text("SHARED-LINE", 8000).expect("first client");

    let mut second = Session::connect_to_existing(&first, 10, 40).expect("second client");
    second
        .wait_for_text("SHARED-LINE", 8000)
        .expect("the second client sees the same screen");

    // The daemon gives every client the smallest requested geometry.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while second.cols() > 40 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(second.cols(), 40, "the smaller width should win");
    assert_eq!(second.rows(), 10, "the smaller height should win");

    // Closing a client that does not own the session leaves it running.
    second.close();
    first.wait_for_text("SHARED-LINE", 8000).expect("still alive");
    first.close();
}

#[test]
fn it_reports_the_exit_status() {
    let mut s = server("sh", &["-c", "exit 7"]);
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while !s.has_exited() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(s.has_exited(), "the session should have ended");
    assert_eq!(s.exit_code(), Some(7));
    s.close();
}

#[test]
fn a_session_has_a_name_and_a_registry() {
    let s = server("cat", &[]);
    assert!(!s.name().is_empty(), "a server session has an id");
    let root = s.root().expect("a server session has a root").to_path_buf();
    assert!(
        root.join(format!("{}.json", s.name())).exists(),
        "the session is in its registry"
    );
    drop(s);
}
