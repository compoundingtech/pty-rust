//! Interactive, Playwright-style TUI tests: drive a real program's raw-mode
//! line editor through the harness and assert on how libghostty renders the
//! in-place redraws. This is the marquee use case of the pty testing library —
//! send keystrokes, watch the screen update, assert the result.
//!
//! We use `bash`'s readline (which puts the tty in raw mode and handles arrow
//! keys / control chords itself) so no custom TUI fixture is needed.

use pty_testkit::{Session, SpawnOptions};

fn opts() -> SpawnOptions {
    SpawnOptions {
        rows: Some(24),
        cols: Some(80),
        ..Default::default()
    }
}

/// Wait until some line, on its own, equals `s` — i.e. command *output*, not the
/// echoed command line (which contains extra text like the prompt / `echo`).
fn wait_standalone_line(sess: &mut Session, line: &str, timeout_ms: u64) -> bool {
    sess.wait_for(
        |ss| ss.lines.iter().any(|l| l == line),
        timeout_ms,
        &format!("standalone line {line:?}"),
    )
    .is_ok()
}

#[test]
fn arrow_left_then_insert_edits_the_line() {
    // Type "echo hello", move the cursor two cells left (before "lo"), insert
    // "XX" → the command becomes "echo helXXlo"; running it prints "helXXlo".
    let mut s = Session::spawn("bash", &["--norc", "--noprofile"], opts()).expect("spawn");
    s.wait_for_text("$", 8000).expect("prompt");

    s.type_str("echo hello");
    // Let readline paint the line before moving the cursor.
    s.wait_for_text("echo hello", 5000).expect("line painted");
    s.press("left").unwrap();
    s.press("left").unwrap();
    s.type_str("XX");
    // The edited command line renders in place.
    s.wait_for_text("echo helXXlo", 5000).expect("edited line");

    s.type_str("\r");
    // The executed output "helXXlo" appears on its own line.
    assert!(
        wait_standalone_line(&mut s, "helXXlo", 5000),
        "edited command did not run:\n{}",
        s.screenshot().text
    );
    let _ = s.press("ctrl+d");
    s.close();
}

#[test]
fn ctrl_a_jumps_to_line_start_for_prefix_insert() {
    // Type "world", Ctrl-A to jump to the start, then type "echo " → "echo
    // world". If Ctrl-A didn't work the buffer would be "worldecho " and bash
    // would error, so a standalone "world" output line proves the jump worked.
    let mut s = Session::spawn("bash", &["--norc", "--noprofile"], opts()).expect("spawn");
    s.wait_for_text("$", 8000).expect("prompt");

    s.type_str("world");
    s.wait_for_text("world", 5000).expect("typed");
    s.press("ctrl+a").unwrap();
    s.type_str("echo ");
    s.wait_for_text("echo world", 5000).expect("prefixed");

    s.type_str("\r");
    assert!(
        wait_standalone_line(&mut s, "world", 5000),
        "ctrl+a prefix insert did not run:\n{}",
        s.screenshot().text
    );
    let _ = s.press("ctrl+d");
    s.close();
}

#[test]
fn ctrl_c_discards_the_current_line() {
    // Type a partial command, Ctrl-C to abort it (readline discards the line and
    // gives a fresh prompt), then run a different command. The aborted text must
    // never execute.
    let mut s = Session::spawn("bash", &["--norc", "--noprofile"], opts()).expect("spawn");
    s.wait_for_text("$", 8000).expect("prompt");

    s.type_str("echo SHOULD-NOT-RUN");
    s.wait_for_text("SHOULD-NOT-RUN", 5000).expect("typed");
    s.press("ctrl+c").unwrap();
    // Let the SIGINT reach bash and reset readline before typing the next line.
    std::thread::sleep(std::time::Duration::from_millis(400));
    s.type_str("echo clean-line\r");

    assert!(
        wait_standalone_line(&mut s, "clean-line", 5000),
        "follow-up command did not run:\n{}",
        s.screenshot().text
    );
    // The aborted command must not have produced a standalone output line.
    let ss = s.screenshot();
    assert!(
        !ss.lines.iter().any(|l| l == "SHOULD-NOT-RUN"),
        "aborted command should not have executed:\n{}",
        ss.text
    );
    let _ = s.press("ctrl+d");
    s.close();
}
