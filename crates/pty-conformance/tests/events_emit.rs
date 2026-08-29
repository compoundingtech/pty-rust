//! CLI half of tests/events-emit.test.ts: `pty emit` publishes `user.*`
//! events with `--json` / `--text`, resolves `$PTY_SESSION`, and rejects bad
//! types and payloads. The `validateUserEventType` rules are pinned through
//! the CLI's error text. The `emitUserEvent` / retention / `EventFollower`
//! library cases stay in Node.

use pty_conformance::*;
use serde_json::{Value, json};

fn events(rig: &Rig, name: &str) -> Vec<Value> {
    std::fs::read_to_string(rig.root().join(format!("{name}.events.jsonl")))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect()
}

/// node: tests/events-emit.test.ts:126
#[test]
fn publishes_a_user_event_on_a_running_session() {
    let rig = Rig::new();
    let name = unique_id("em");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["emit", &name, "user.tests-passed", "--json", "{\"count\": 42}"]);
    expect_status(&r, 0);
    let ev = events(&rig, &name);
    let latest = ev.last().expect("an event");
    assert_eq!(latest["type"], "user.tests-passed");
    assert_eq!(latest["data"], json!({"count": 42}));
}

/// node: tests/events-emit.test.ts:139
#[test]
fn resolves_pty_session_when_no_ref_is_given() {
    let rig = Rig::new();
    let name = unique_id("em");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty_env(&[("PTY_SESSION", &name)], &["emit", "user.from-inside"]);
    expect_status(&r, 0);
    assert!(events(&rig, &name).iter().any(|e| e["type"] == "user.from-inside"));
}

/// node: tests/events-emit.test.ts:149
#[test]
fn rejects_non_user_types() {
    let rig = Rig::new();
    let name = unique_id("em");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["emit", &name, "bogus-type"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "must start with");
}

/// node: tests/events-emit.test.ts:158
#[test]
fn errors_without_a_ref_or_pty_session() {
    let rig = Rig::new();
    let r = rig.pty(&["emit", "user.whatever"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "not running inside a pty session|no session ref");
}

/// node: tests/events-emit.test.ts:172
#[test]
fn text_lands_on_the_text_field() {
    let rig = Rig::new();
    let name = unique_id("em");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["emit", &name, "user.note", "--text", "checkpoint reached"]);
    expect_status(&r, 0);
    let ev = events(&rig, &name);
    let latest = ev.last().unwrap();
    assert_eq!(latest["type"], "user.note");
    assert_eq!(latest["text"], "checkpoint reached");
    assert!(latest.get("data").is_none(), "{latest}");
}

/// node: tests/events-emit.test.ts:185
#[test]
fn json_and_text_together_land_both_fields() {
    let rig = Rig::new();
    let name = unique_id("em");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["emit", &name, "user.mixed", "--json", "{\"ok\":true}", "--text", "done"]);
    expect_status(&r, 0);
    let ev = events(&rig, &name);
    let latest = ev.last().unwrap();
    assert_eq!(latest["data"], json!({"ok": true}));
    assert_eq!(latest["text"], "done");
}

/// node: tests/events-emit.test.ts:198
#[test]
fn rejects_invalid_json_without_writing() {
    let rig = Rig::new();
    let name = unique_id("em");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let before = events(&rig, &name).len();
    let r = rig.pty(&["emit", &name, "user.bad", "--json", "{not-valid-json"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "not valid JSON|--json");
    assert_eq!(events(&rig, &name).len(), before);
}

/// node: tests/events-emit.test.ts:83
#[test]
fn event_type_validation_is_reported_by_the_cli() {
    let rig = Rig::new();
    let name = unique_id("em");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let before = events(&rig, &name).len();
    for (type_, message) in [
        ("build-done", "must start with"),
        ("session_start", "must start with"),
        ("state.set", "must start with"),
        ("user.", "suffix"),
        ("user.has space", "whitespace"),
        ("user.tab\tfoo", "whitespace"),
    ] {
        let r = rig.pty(&["emit", &name, type_]);
        assert_ne!(r.status, 0, "{type_:?}: {}", r.summary());
        expect_regex(&r.stderr(), message);
    }
    for ok in ["user.build-done", "user.a"] {
        expect_status(&rig.pty(&["emit", &name, ok]), 0);
    }
    assert_eq!(events(&rig, &name).len(), before + 2);
}
