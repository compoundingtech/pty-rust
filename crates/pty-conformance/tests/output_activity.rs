//! `lastOutputAtMs`: the daemon records when the child last printed.
//!
//! The Node tool grew this field in its PR #168, merged on 2026-08-29 (the
//! Node pty repository's `docs/vrs/requirements.md` R14 and
//! `docs/disk-layout.md`, not this repository's `docs/`). The contract is:
//!
//! - absent until the session produces output, and absent on a record an
//!   older daemon wrote — never zero, and never a claim of idleness;
//! - a unix-millisecond number, not an ISO string, so a reader doing
//!   freshness arithmetic needs no date parser;
//! - persisted at most once a second while output flows, so a chatty session
//!   costs one metadata write per second rather than one per chunk;
//! - carried into the exit record even when that once-a-second write was
//!   still pending, so the last thing a child printed is never lost.
//!
//! The pinned Node reference includes this field even though its package
//! version remains 0.12.0, so these are cross-binary contract tests.

use pty_conformance::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn stamp(rig: &Rig, id: &str) -> Option<i64> {
    rig.meta(id)?.get("lastOutputAtMs")?.as_i64()
}

/// node: tests/output-activity.test.ts:132
#[test]
fn the_stamp_is_absent_until_the_child_prints() {
    let rig = Rig::new();
    rig.daemon("act-silent", &["cat"], DaemonOpts::no_display_name());
    // Long enough that a write would have happened if one were coming.
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        stamp(&rig, "act-silent"),
        None,
        "a session that has printed nothing must carry no stamp, not a zero"
    );
}

/// node: tests/output-activity.test.ts:141
#[test]
fn the_stamp_appears_after_output_and_reads_as_now() {
    let rig = Rig::new();
    rig.daemon("act-print", &["cat"], DaemonOpts::no_display_name());
    let before = now_ms();
    rig.pty(&["send", "act-print", "--seq", "hello", "--seq", "key:return"]);
    wait_until("the output stamp is written", || {
        stamp(&rig, "act-print").is_some()
    });
    let at = stamp(&rig, "act-print").expect("stamp");
    assert!(
        at >= before - 1000 && at <= now_ms() + 1000,
        "stamp {at} is not a recent unix-millisecond reading (now is {})",
        now_ms()
    );
}

/// node: tests/output-activity.test.ts:159
#[test]
fn a_later_burst_moves_the_stamp_forward() {
    let rig = Rig::new();
    rig.daemon("act-again", &["cat"], DaemonOpts::no_display_name());
    rig.pty(&["send", "act-again", "--seq", "first", "--seq", "key:return"]);
    wait_until("the first stamp", || stamp(&rig, "act-again").is_some());
    let first = stamp(&rig, "act-again").expect("first stamp");

    // Past the debounce, so the second write is a separate one.
    std::thread::sleep(Duration::from_millis(1200));
    rig.pty(&["send", "act-again", "--seq", "second", "--seq", "key:return"]);
    wait_until("the stamp moves", || {
        stamp(&rig, "act-again").is_some_and(|at| at > first)
    });
    assert!(stamp(&rig, "act-again").unwrap() > first);
}

/// The one that the debounce could lose: a child that prints and exits
/// inside the same second never gets its own scheduled write.
/// node: tests/output-activity.test.ts:179
#[test]
fn a_child_that_prints_and_exits_at_once_keeps_its_stamp() {
    let rig = Rig::new();
    let before = now_ms();
    rig.daemon(
        "act-quick",
        &["sh", "-c", "printf 'quick\\n'"],
        DaemonOpts::keep(),
    );
    wait_until("the exit record", || {
        rig.meta("act-quick")
            .is_some_and(|m| m.get("exitCode").is_some())
    });
    let at = stamp(&rig, "act-quick")
        .expect("the exit record must carry the last output stamp, debounce or not");
    assert!(
        at >= before - 1000 && at <= now_ms() + 1000,
        "exit stamp {at} is not a recent reading"
    );
}

/// A busy session must not write its record once per chunk.
/// No Node counterpart: the debounce is stated as a requirement (at most one
/// write a second) rather than tested there, so this is a Rust-owned fixture
/// for a promise the Node implementation makes and does not check.
#[test]
fn a_busy_session_writes_the_stamp_about_once_a_second() {
    let rig = Rig::new();
    // Print steadily for about three seconds.
    rig.daemon(
        "act-busy",
        &["sh", "-c", "i=0; while [ $i -lt 300 ]; do printf 'tick %s\\n' $i; i=$((i+1)); sleep 0.01; done; exec cat"],
        DaemonOpts::no_display_name(),
    );
    wait_until("the first stamp", || stamp(&rig, "act-busy").is_some());

    let mut seen = std::collections::BTreeSet::new();
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < until {
        if let Some(at) = stamp(&rig, "act-busy") {
            seen.insert(at);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // Three seconds of output at one write per second is a handful of
    // distinct values. One write per chunk would be hundreds.
    assert!(
        seen.len() <= 10,
        "the stamp changed {} times in three seconds; the debounce is not holding",
        seen.len()
    );
    assert!(!seen.is_empty(), "no stamp was written at all");
}
