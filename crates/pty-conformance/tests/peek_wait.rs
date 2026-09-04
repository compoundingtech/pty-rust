//! Port of tests/peek-wait.test.ts: `peek --full`, `peek --wait` (live,
//! already-on-screen, multiple patterns, exited sessions), and
//! `events --wait`.

use pty_conformance::*;
use std::time::{Duration, Instant};

/// node: tests/peek-wait.test.ts:81
#[test]
fn peek_full_shows_the_scrollback() {
    let rig = Rig::new();
    let name = unique_id("pw");
    rig.daemon(
        &name,
        &["sh", "-c", "for i in $(seq 1 100); do echo line$i; done; exec cat"],
        DaemonOpts::no_display_name(),
    );
    std::thread::sleep(Duration::from_millis(500));
    let normal = rig.pty(&["peek", "--plain", &name]);
    let full = rig.pty(&["peek", "--plain", "--full", &name]);
    expect_status(&normal, 0);
    expect_status(&full, 0);
    let normal_lines = normal.stdout().trim().lines().count();
    let full_lines = full.stdout().trim().lines().count();
    assert!(full_lines > normal_lines, "full={full_lines} normal={normal_lines}");
    assert!(full_lines >= 100, "full={full_lines}");
    let s = full.stdout();
    expect_contains(&s, "line1");
    expect_contains(&s, "line100");
}

/// node: tests/peek-wait.test.ts:101
#[test]
fn wait_until_text_appears() {
    let rig = Rig::new();
    let name = unique_id("pw");
    rig.daemon(&name, &["sh", "-c", "sleep 0.5; echo READY; exec cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["peek", "--wait", "READY", "-t", "5", "--plain", &name]);
    expect_status(&r, 0);
    expect_contains(&r.stdout(), "READY");
}

/// node: tests/peek-wait.test.ts:112
#[test]
fn wait_times_out_when_text_never_appears() {
    let rig = Rig::new();
    let name = unique_id("pw");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["peek", "--wait", "NEVER", "-t", "1", "--plain", &name]);
    expect_status(&r, 1);
    let err = r.stderr();
    expect_contains(&err, "Timed out");
    expect_contains(&err, "NEVER");
}

/// node: tests/peek-wait.test.ts:123
#[test]
fn wait_returns_immediately_when_text_is_already_on_screen() {
    let rig = Rig::new();
    let name = unique_id("pw");
    rig.daemon(&name, &["sh", "-c", "echo ALREADY; exec cat"], DaemonOpts::no_display_name());
    std::thread::sleep(Duration::from_millis(300));
    let start = Instant::now();
    let r = rig.pty(&["peek", "--wait", "ALREADY", "-t", "5", "--plain", &name]);
    let elapsed = start.elapsed();
    expect_status(&r, 0);
    expect_contains(&r.stdout(), "ALREADY");
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
}

/// node: tests/peek-wait.test.ts:138
#[test]
fn multiple_wait_patterns_match_any() {
    let rig = Rig::new();
    let name = unique_id("pw");
    rig.daemon(&name, &["sh", "-c", "echo SECOND; exec cat"], DaemonOpts::no_display_name());
    std::thread::sleep(Duration::from_millis(300));
    let r = rig.pty(&["peek", "--wait", "FIRST", "--wait", "SECOND", "-t", "5", "--plain", &name]);
    expect_status(&r, 0);
    expect_contains(&r.stdout(), "SECOND");
}

/// node: tests/peek-wait.test.ts:149
#[test]
fn wait_reads_saved_output_from_an_exited_session() {
    let rig = Rig::new();
    let name = unique_id("pw");
    rig.daemon(&name, &["sh", "-c", "echo TEST_PASSED; exit 0"], DaemonOpts::keep());
    rig.wait_for_exit(&name);
    std::thread::sleep(Duration::from_millis(500));
    let r = rig.pty(&["peek", "--wait", "TEST_PASSED", "-t", "5", "--plain", &name]);
    expect_status(&r, 0);
    expect_contains(&r.stdout(), "TEST_PASSED");
}

/// node: tests/peek-wait.test.ts:162
#[test]
fn wait_on_an_exited_session_errors_when_the_pattern_is_missing() {
    let rig = Rig::new();
    let name = unique_id("pw");
    rig.daemon(&name, &["sh", "-c", "echo nope; exit 0"], DaemonOpts::keep());
    rig.wait_for_exit(&name);
    std::thread::sleep(Duration::from_millis(500));
    let r = rig.pty(&["peek", "--wait", "MISSING", "-t", "5", "--plain", &name]);
    expect_status(&r, 1);
    let err = r.stderr();
    expect_contains(&err, "exited");
    expect_contains(&err, "MISSING");
}

/// node: tests/peek-wait.test.ts:175
#[test]
fn peek_plain_on_an_exited_session_shows_saved_output() {
    let rig = Rig::new();
    let name = unique_id("pw");
    rig.daemon(&name, &["sh", "-c", "echo SAVED_OUTPUT; exit 0"], DaemonOpts::keep());
    rig.wait_for_exit(&name);
    std::thread::sleep(Duration::from_millis(500));
    let r = rig.pty(&["peek", "--plain", &name]);
    expect_status(&r, 0);
    expect_contains(&r.stdout(), "SAVED_OUTPUT");
}

// ── pty events --wait ──

/// node: tests/peek-wait.test.ts:189
#[test]
fn events_wait_for_a_specific_event_type() {
    let rig = Rig::new();
    let name = unique_id("pw");
    rig.daemon(
        &name,
        &["sh", "-c", "sleep 2; printf '\\007'; exec cat"],
        DaemonOpts::no_display_name(),
    );
    let r = rig.pty(&["events", "--wait", "bell", "-t", "10", &name]);
    expect_status(&r, 0);
    expect_contains(&r.stdout(), "bell");
}

/// node: tests/peek-wait.test.ts:199
#[test]
fn events_wait_times_out_when_the_event_never_occurs() {
    let rig = Rig::new();
    let name = unique_id("pw");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["events", "--wait", "bell", "-t", "1", &name]);
    expect_status(&r, 1);
    expect_contains(&r.stderr(), "Timed out");
}

/// `--timeout inf` used to end the command with a Rust panic. It is an
/// ordinary thing to type and `f64::from_str` accepts it, so it reached
/// `Duration::from_secs_f64`, which panics on a value that is not finite.
/// `--timeout NaN` quietly turned the bounded wait into an unbounded one.
///
/// Both now wait, which is what the Node tool does with the same input.
#[test]
fn a_timeout_that_is_not_a_finite_number_waits_instead_of_crashing() {
    let rig = Rig::new();
    rig.daemon("pw-inf", &["cat"], DaemonOpts::no_display_name());
    for value in ["inf", "Infinity", "NaN", "-1"] {
        let mut child = rig
            .command(&["peek", "--wait", "NEVERMATCH", "--timeout", value, "pw-inf"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn pty peek --wait");
        // Long enough that a crash would already have happened, short
        // enough to keep the suite quick.
        std::thread::sleep(Duration::from_millis(600));
        let finished = child.try_wait().expect("try_wait");
        let _ = child.kill();
        let out = child.wait_with_output().expect("wait");
        let said = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            finished.is_none(),
            "--timeout {value} ended early with {finished:?}: {said}"
        );
        assert!(
            !said.contains("panicked"),
            "--timeout {value} panicked: {said}"
        );
    }
}
