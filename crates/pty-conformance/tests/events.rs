//! Port of tests/events.test.ts, CLI half: what the daemon records in
//! `<id>.events.jsonl` (bell, title changes with deduplication, OSC 9 / OSC
//! 777 notifications, the log truncated at daemon start) as read back through
//! `pty events --recent [--json]`, the `formatEvent` text shapes as printed
//! by `pty events --recent` (fed through `pty emit` for the `user.*` cases),
//! and follow mode (`pty events <id>`) printing events appended after it
//! started.
//!
//! Left out: the EventWriter retention cap, `readRecentEvents(name, n)`,
//! `clearEvents`/`cleanupAll` on the raw file, and the EventFollower
//! offset/truncation cases — library-only.

use pty_conformance::*;
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

fn events_path(rig: &Rig, id: &str) -> std::path::PathBuf {
    rig.root().join(format!("{id}.events.jsonl"))
}

fn recent_json(rig: &Rig, id: &str) -> Vec<Value> {
    let out = rig.pty(&["events", "--recent", "--json", id]);
    expect_status(&out, 0);
    out.stdout()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{l}: {e}")))
        .collect()
}

fn recent_text(rig: &Rig, id: &str) -> String {
    let out = rig.pty(&["events", "--recent", id]);
    expect_status(&out, 0);
    out.stdout()
}

/// Wait until the events file has at least `n` events of `kind`.
fn wait_for_events(rig: &Rig, id: &str, kind: &str, n: usize) -> Vec<Value> {
    wait_until(&format!("{n} {kind} event(s) for {id}"), || {
        recent_json(rig, id).iter().filter(|e| e["type"] == kind).count() >= n
    });
    recent_json(rig, id)
}

/// A session whose child prints `printf_arg` (a printf format) and then
/// stays alive on `cat`.
fn emitting_session(rig: &Rig, id: &str, printf_arg: &str) {
    let script = format!("printf '{printf_arg}'; exec cat");
    rig.daemon(id, &["sh", "-c", &script], DaemonOpts::no_display_name());
}

/// node: tests/events.test.ts:414
#[test]
fn bell_is_logged() {
    let rig = Rig::new();
    emitting_session(&rig, "evb", "\\a");
    let events = wait_for_events(&rig, "evb", "bell", 1);
    let bells: Vec<&Value> = events.iter().filter(|e| e["type"] == "bell").collect();
    assert!(!bells.is_empty());
    assert_eq!(bells[0]["session"], "evb");
}

/// node: tests/events.test.ts:435
#[test]
fn title_change_is_logged() {
    let rig = Rig::new();
    emitting_session(&rig, "evt", "\\033]0;My Custom Title\\a");
    let events = wait_for_events(&rig, "evt", "title_change", 1);
    let titles: Vec<&Value> = events.iter().filter(|e| e["type"] == "title_change").collect();
    assert_eq!(titles[0]["value"], "My Custom Title");
}

/// node: tests/events.test.ts:453
#[test]
fn identical_title_changes_are_deduplicated() {
    let rig = Rig::new();
    emitting_session(
        &rig,
        "evd",
        "\\033]0;Same Title\\a'; sleep 0.2; printf '\\033]0;Same Title\\a'; sleep 0.2; printf '\\033]0;Different Title\\a",
    );
    wait_for_events(&rig, "evd", "title_change", 2);
    std::thread::sleep(Duration::from_millis(300));
    let events = recent_json(&rig, "evd");
    let values: Vec<&str> = events
        .iter()
        .filter(|e| e["type"] == "title_change")
        .filter_map(|e| e["value"].as_str())
        .collect();
    assert_eq!(values.iter().filter(|v| **v == "Same Title").count(), 1, "{values:?}");
    assert!(values.contains(&"Different Title"), "{values:?}");
}

/// node: tests/events.test.ts:484
#[test]
fn osc_9_notification_is_logged() {
    let rig = Rig::new();
    emitting_session(&rig, "ev9", "\\033]9;Build complete\\a");
    let events = wait_for_events(&rig, "ev9", "notification", 1);
    let n: Vec<&Value> = events.iter().filter(|e| e["type"] == "notification").collect();
    assert_eq!(n[0]["body"], "Build complete");
    assert_eq!(n[0]["source"], "osc9");
}

/// node: tests/events.test.ts:503
#[test]
fn osc_777_notification_is_logged() {
    let rig = Rig::new();
    emitting_session(&rig, "ev777", "\\033]777;notify;Build;All tests passed\\a");
    let events = wait_for_events(&rig, "ev777", "notification", 1);
    let n: Vec<&Value> = events.iter().filter(|e| e["type"] == "notification").collect();
    assert_eq!(n[0]["title"], "Build");
    assert_eq!(n[0]["body"], "All tests passed");
    assert_eq!(n[0]["source"], "osc777");
}

/// node: tests/events.test.ts:525
#[test]
fn events_file_is_cleared_at_session_start() {
    let rig = Rig::new();
    let id = "evclear";
    std::fs::write(
        events_path(&rig, id),
        format!("{}\n", serde_json::json!({"session": id, "type": "bell", "ts": "old"})),
    )
    .unwrap();
    rig.daemon(id, &["cat"], DaemonOpts::no_display_name());
    std::thread::sleep(Duration::from_millis(100));
    let events = recent_json(&rig, id);
    let stale: Vec<&Value> = events.iter().filter(|e| e["ts"] == "old").collect();
    assert!(stale.is_empty(), "{events:?}");
}

/// node: tests/events.test.ts:216
#[test]
fn formats_bell() {
    let rig = Rig::new();
    emitting_session(&rig, "fmtb", "\\a");
    wait_for_events(&rig, "fmtb", "bell", 1);
    let text = recent_text(&rig, "fmtb");
    expect_contains(&text, "fmtb:");
    expect_regex(&text, r"(?m)\] fmtb: bell$");
}

/// node: tests/events.test.ts:226
#[test]
fn formats_title_change() {
    let rig = Rig::new();
    emitting_session(&rig, "fmtt", "\\033]0;Building...\\a");
    wait_for_events(&rig, "fmtt", "title_change", 1);
    expect_contains(&recent_text(&rig, "fmtt"), "title -> \"Building...\"");
}

/// node: tests/events.test.ts:236
#[test]
fn formats_notification_with_title_and_body() {
    let rig = Rig::new();
    emitting_session(&rig, "fmtn", "\\033]777;notify;Done;Build succeeded\\a");
    wait_for_events(&rig, "fmtn", "notification", 1);
    let text = recent_text(&rig, "fmtn");
    expect_contains(&text, "-- \"Done\"");
    expect_contains(&text, "Build succeeded");
    expect_contains(&text, "notification -- \"Done\" Build succeeded");
}

/// node: tests/events.test.ts:249
#[test]
fn formats_focus_request() {
    let rig = Rig::new();
    emitting_session(&rig, "fmtf", "\\033[?1004h");
    wait_for_events(&rig, "fmtf", "focus_request", 1);
    expect_contains(&recent_text(&rig, "fmtf"), "focus requested");
}

/// node: tests/events.test.ts:258
#[test]
fn formats_cursor_visible() {
    let rig = Rig::new();
    emitting_session(&rig, "fmtc", "\\033[?25l'; sleep 0.1; printf '\\033[?25h");
    wait_for_events(&rig, "fmtc", "cursor_visible", 1);
    expect_contains(&recent_text(&rig, "fmtc"), "cursor restored");
}

/// node: tests/events.test.ts:267
#[test]
fn formats_user_event_with_text_quoted() {
    let rig = Rig::new();
    rig.daemon("fmtu", &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["emit", "fmtu", "user.note", "--text", "checkpoint"]), 0);
    expect_contains(&recent_text(&rig, "fmtu"), "user.note \"checkpoint\"");
}

/// node: tests/events.test.ts:277
#[test]
fn formats_user_event_with_data_as_json() {
    let rig = Rig::new();
    rig.daemon("fmtd", &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["emit", "fmtd", "user.progress", "--json", "{\"pct\": 40}"]), 0);
    expect_contains(&recent_text(&rig, "fmtd"), "user.progress {\"pct\":40}");
}

/// node: tests/events.test.ts:287
#[test]
fn formats_user_event_without_payload_as_just_the_type() {
    let rig = Rig::new();
    rig.daemon("fmtp", &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["emit", "fmtp", "user.ping"]), 0);
    let text = recent_text(&rig, "fmtp");
    expect_regex(&text, r"(?m)user\.ping\s*$");
}

/// node: tests/events.test.ts:299
#[test]
fn follow_mode_prints_events_appended_after_it_started() {
    let rig = Rig::new();
    rig.daemon("evfol", &["cat"], DaemonOpts::no_display_name());
    let mut cmd = rig.command(&["events", "--json", "evfol"]);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    std::thread::sleep(Duration::from_millis(400));
    expect_status(&rig.pty(&["emit", "evfol", "user.one"]), 0);
    expect_status(&rig.pty(&["emit", "evfol", "user.two"]), 0);
    std::thread::sleep(Duration::from_millis(600));
    kill_pid(child.id() as i32, libc::SIGINT);
    let _ = poll_for(Duration::from_secs(5), || child.try_wait().map(|s| s.is_some()).unwrap_or(true));
    let _ = child.kill();
    let _ = child.wait();
    let out = String::from_utf8_lossy(&reader.join().unwrap()).into_owned();
    let types: Vec<String> = out
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|e| e["type"].as_str().map(|s| s.to_string()))
        .collect();
    assert!(types.iter().any(|t| t == "user.one"), "{out}");
    assert!(types.iter().any(|t| t == "user.two"), "{out}");
    // Existing lines (the session_start) are not replayed: follow starts at EOF.
    assert!(!types.iter().any(|t| t == "session_start"), "{out}");
}
