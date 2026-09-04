//! `pty events`: `--recent`, `--wait` with and without a timeout, the
//! usage errors, and the text format.
//!
//! node: tests/peek-wait.test.ts:205-226, tests/metadata-events.test.ts:632-692,
//! src/cli.ts:1219-1248, 3965-4051

mod cli_common;

use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use cli_common::Rig;
use serde_json::json;

fn append_event(rig: &Rig, name: &str, line: &str) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rig.path(&format!("{name}.events.jsonl")))
        .unwrap();
    writeln!(f, "{line}").unwrap();
}

/// node: src/cli.ts:1226-1236, 3969-3982
#[test]
fn recent_and_usage() {
    let rig = Rig::new();
    rig.write_meta("s", json!({"displayName": "Friendly"}));
    assert_eq!(rig.ok(&["events", "--recent", "s"]).stdout, "No recent events for \"s\".\n");
    append_event(&rig, "s", r#"{"session":"s","type":"session_start","ts":"2026-01-02T03:04:05.000Z","tags":{"role":"web"}}"#);
    append_event(&rig, "s", r#"{"session":"s","type":"user.note","ts":"2026-01-02T03:04:06.000Z","text":"hi"}"#);
    append_event(&rig, "s", r#"{"session":"s","type":"display_name_change","ts":"2026-01-02T03:04:07.000Z","previous":"old","value":"new"}"#);
    let out = rig.ok(&["events", "--recent", "--json", "Friendly"]);
    assert_eq!(out.stdout.lines().count(), 3);
    assert_eq!(
        out.stdout.lines().next().unwrap(),
        r#"{"session":"s","type":"session_start","ts":"2026-01-02T03:04:05.000Z","tags":{"role":"web"}}"#
    );
    let text = rig.ok(&["events", "--recent", "s"]).stdout;
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].ends_with("] s: started role=web"), "{:?}", lines[0]);
    assert!(lines[1].ends_with("] s: user.note \"hi\""), "{:?}", lines[1]);
    assert!(lines[2].ends_with("] s: display_name -> \"new\" (was \"old\")"), "{:?}", lines[2]);
    assert!(lines[0].starts_with('['));
    assert_eq!(lines[0].as_bytes()[9], b']');

    let out = rig.run(&["events"]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr,
        "Usage: pty events [--all] [--recent] [--json] [--wait <type>] [-t <seconds>] [<name>]\n"
    );
    let out = rig.run(&["events", "--recent", "--all"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "--recent requires a session name.\n");
    let out = rig.run(&["events", "--wait", "bell", "--all"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "--wait requires a session name.\n");
    let out = rig.run(&["events", "--recent", "ghost"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "Session \"ghost\" not found.\n");
    // An unknown dash token ends the flag loop and becomes the ref.
    let out = rig.run(&["events", "--bogus"]);
    assert_eq!(out.stderr, "Session \"--bogus\" not found.\n");
}

/// node: tests/peek-wait.test.ts:205-226
#[test]
fn wait_prints_the_first_matching_event_or_times_out() {
    let rig = Rig::new();
    rig.write_meta("w", json!({}));
    let out = rig.run(&["events", "--wait", "bell", "-t", "1", "w"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "Timed out after 1s waiting for \"bell\" event.\n");
    let out = rig.run(&["events", "--wait", "bell", "--timeout", "0.5", "w"]);
    assert_eq!(out.stderr, "Timed out after 0.5s waiting for \"bell\" event.\n");

    let mut child = rig
        .cmd(&["events", "--wait", "bell", "-t", "10", "--json", "w"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    append_event(&rig, "w", r#"{"session":"w","type":"user.other","ts":"2026-01-02T03:04:05.000Z"}"#);
    append_event(&rig, "w", r#"{"session":"w","type":"bell","ts":"2026-01-02T03:04:06.000Z"}"#);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "{\"session\":\"w\",\"type\":\"bell\",\"ts\":\"2026-01-02T03:04:06.000Z\"}\n"
    );
}

/// node: src/cli.ts:4028-4031, 4044-4047 — SIGINT ends a follow with exit 0.
#[test]
fn follow_exits_zero_on_sigint() {
    let rig = Rig::new();
    rig.write_meta("f", json!({}));
    let mut child = rig
        .cmd(&["events", "f"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    append_event(&rig, "f", r#"{"session":"f","type":"bell","ts":"2026-01-02T03:04:06.000Z"}"#);
    std::thread::sleep(Duration::from_millis(600));
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).ends_with("] f: bell\n"));
}
