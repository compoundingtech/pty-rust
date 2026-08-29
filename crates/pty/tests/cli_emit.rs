//! `pty emit`: payloads, type validation, the `$PTY_SESSION` default, and
//! the help paths.
//!
//! node: tests/events-emit.test.ts

mod cli_common;

use cli_common::Rig;
use serde_json::json;

/// node: tests/events-emit.test.ts:138-233
#[test]
fn emits_user_events() {
    let rig = Rig::new();
    rig.write_meta("s", json!({}));
    let out = rig.ok(&["emit", "s", "user.tests-passed", "--json", "{\"count\": 42}"]);
    assert_eq!(out.stdout, "");
    let ev = rig.events("s");
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["type"], "user.tests-passed");
    assert_eq!(ev[0]["data"], json!({"count": 42}));
    assert_eq!(ev[0]["session"], "s");
    let keys: Vec<&String> = ev[0].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["session", "type", "ts", "data"]);

    rig.run_env(&["emit", "user.from-inside"], &[("PTY_SESSION", "s")]);
    assert_eq!(rig.events("s")[1]["type"], "user.from-inside");

    rig.ok(&["emit", "s", "user.note", "--text", "checkpoint reached"]);
    let last = rig.events("s").pop().unwrap();
    assert_eq!(last["text"], "checkpoint reached");
    assert!(last.get("data").is_none());
    rig.ok(&["emit", "s", "user.both", "--json", "{\"ok\":true}", "--text", "done"]);
    let last = rig.events("s").pop().unwrap();
    assert_eq!(last["data"], json!({"ok": true}));
    assert_eq!(last["text"], "done");

    let before = rig.events("s").len();
    let out = rig.run(&["emit", "s", "user.bad", "--json", "{not-valid-json"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.starts_with("pty emit: --json payload is not valid JSON: "), "{:?}", out.stderr);
    assert_eq!(rig.events("s").len(), before);
}

/// node: tests/events-emit.test.ts:92-110, 165-187
#[test]
fn validates_the_type_and_the_ref() {
    let rig = Rig::new();
    rig.write_meta("s", json!({}));
    let cases: [(&str, &str); 6] = [
        ("bogus-type", "custom events must start with \"user.\" (got \"bogus-type\")\n"),
        ("session_start", "custom events must start with \"user.\" (got \"session_start\")\n"),
        ("user.", "event type \"user.\" needs a suffix (e.g. \"user.build-done\")\n"),
        ("", "event type must be a non-empty string\n"),
        ("user.has space", "event type may not contain whitespace or control characters\n"),
        ("user.tab\tfoo", "event type may not contain whitespace or control characters\n"),
    ];
    for (ty, expected) in cases {
        let out = rig.run(&["emit", "s", ty]);
        assert_eq!(out.code, 1, "{ty:?}");
        assert_eq!(out.stderr, expected, "{ty:?}");
    }
    assert!(rig.events("s").is_empty());

    let out = rig.run(&["emit", "user.whatever"]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr,
        "pty emit: no session ref given and not running inside a pty session\n  tip: run inside a pty session, or: pty emit <session-ref> <type>\n"
    );
    let out = rig.run(&["emit", "ghost", "user.x"]);
    assert_eq!(out.stderr, "Session \"ghost\" not found.\n");
}

/// node: src/cli.ts:3530-3541, 3576-3579 — help to stdout: exit 0 for
/// `-h`, exit 1 for a wrong positional count.
#[test]
fn help_paths() {
    let rig = Rig::new();
    let help = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/help/emit.txt"
    ))
    .unwrap();
    let out = rig.ok(&["emit", "s", "user.x", "-h"]);
    assert_eq!(out.stdout, help);
    let out = rig.run(&["emit"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stdout, help);
    let out = rig.run(&["emit", "a", "b", "c"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stdout, help);
}
