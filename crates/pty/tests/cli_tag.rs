//! `pty tag`: show, bulk set/remove, parse errors, and the one
//! `tags_change` event per write.
//!
//! node: tests/tag-mutate.test.ts, tests/tag-bulk.test.ts, tests/tags.test.ts

mod cli_common;

use cli_common::Rig;
use serde_json::json;

fn tags(rig: &Rig, name: &str) -> Option<serde_json::Value> {
    rig.read_meta(name).and_then(|m| m.get("tags").cloned())
}

/// node: tests/tag-mutate.test.ts:95-183
#[test]
fn set_show_remove_and_not_found() {
    let rig = Rig::new();
    rig.write_meta("s", json!({"exitCode": 0, "exitedAt": cli_common::iso_now(0), "tags": {"keep": "true"}}));
    assert_eq!(rig.ok(&["tag", "s"]).stdout, "  keep=true\n");
    let out = rig.ok(&["tag", "s", "role=server", "env=dev"]);
    assert_eq!(out.stdout, "Tags on \"s\":\n  keep=true\n  role=server\n  env=dev\n");
    assert_eq!(tags(&rig, "s"), Some(json!({"keep": "true", "role": "server", "env": "dev"})));
    rig.ok(&["tag", "s", "role=new"]);
    assert_eq!(tags(&rig, "s").unwrap()["role"], "new");
    rig.ok(&["tag", "s", "--rm", "env"]);
    assert!(tags(&rig, "s").unwrap().get("env").is_none());
    rig.ok(&["tag", "s", "--rm", "role", "--rm", "keep"]);
    assert_eq!(tags(&rig, "s"), None, "removing the last tag deletes the field");
    assert_eq!(rig.ok(&["tag", "s"]).stdout, "No tags on \"s\".\n");
    let out = rig.ok(&["tag", "s", "strategy=permanent"]);
    assert_eq!(out.stdout, "Tags on \"s\":\n  strategy=permanent\n");
    let out = rig.ok(&["tag", "s", "--rm", "strategy"]);
    assert_eq!(out.stdout, "Tags cleared on \"s\".\n");

    let out = rig.run(&["tag", "nonexistent", "foo=bar"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "Session \"nonexistent\" not found.\n");
    let out = rig.run(&["tag"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "Usage: pty tag <name> [key=value...] [--rm key...]\n");
}

/// node: tests/tag-bulk.test.ts:119-288
#[test]
fn bulk_semantics() {
    let rig = Rig::new();
    rig.write_meta("b", json!({}));
    rig.ok(&["tag", "b", "color=red", "color=blue", "key=", "foo=bar=baz"]);
    assert_eq!(tags(&rig, "b"), Some(json!({"color": "blue", "key": "", "foo": "bar=baz"})));
    rig.ok(&["tag", "b", "--rm", "never-was-set"]);
    assert_eq!(tags(&rig, "b"), Some(json!({"color": "blue", "key": "", "foo": "bar=baz"})));
    // Updates apply before removals, in any order.
    rig.ok(&["tag", "b", "--rm", "color", "k=v", "--rm", "k", "z=new"]);
    assert_eq!(tags(&rig, "b"), Some(json!({"key": "", "foo": "bar=baz", "z": "new"})));
    rig.ok(&["tag", "b", "--rm", "key", "--rm", "foo", "--rm", "z", "--rm", "z"]);
    assert_eq!(tags(&rig, "b"), None);
}

/// node: tests/tag-bulk.test.ts:292-356 — parse errors abort before any write.
#[test]
fn parse_errors_write_nothing() {
    let rig = Rig::new();
    rig.write_meta("p", json!({"tags": {"a": "1"}}));
    let cases: [(&[&str], &str); 5] = [
        (&["tag", "p", "no-equals-here"], "pty tag: invalid argument \"no-equals-here\". Use key=value or --rm key.\n"),
        (&["tag", "p", "=value"], "pty tag: empty key in \"=value\". Tag keys must be non-empty.\n"),
        (&["tag", "p", "--rm"], "pty tag: --rm requires a key (e.g. --rm role)\n"),
        (&["tag", "p", "--rm", ""], "pty tag: --rm requires a non-empty key\n"),
        (&["tag", "p", "good=yes", "no-equals"], "pty tag: invalid argument \"no-equals\". Use key=value or --rm key.\n"),
    ];
    for (args, expected) in cases {
        let out = rig.run(args);
        assert_eq!(out.code, 1, "{args:?}");
        assert_eq!(out.stderr, expected, "{args:?}");
        assert_eq!(tags(&rig, "p"), Some(json!({"a": "1"})), "{args:?}");
    }
}

/// node: tests/tag-bulk.test.ts:360-411, tests/tags.test.ts:415-445
#[test]
fn one_event_per_effective_write_and_display_name_refs() {
    let rig = Rig::new();
    rig.write_meta("e", json!({"displayName": "friendly"}));
    rig.ok(&["tag", "friendly", "a=1", "b=2", "c=3", "--rm", "z"]);
    let events = rig.events("e");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "tags_change");
    assert_eq!(events[0]["previous"], json!({}));
    assert_eq!(events[0]["value"], json!({"a": "1", "b": "2", "c": "3"}));
    rig.ok(&["tag", "e", "a=1"]);
    assert_eq!(rig.events("e").len(), 1, "a no-op write emits nothing");
    rig.ok(&["tag", "e", "a=x", "new=y"]);
    let events = rig.events("e");
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["value"], json!({"a": "x", "b": "2", "c": "3", "new": "y"}));
    assert!(rig.ok(&["tag", "e"]).stdout.contains("new=y"));
}

/// node: src/cli.ts:1530-1532 — the pty.toml warning after a write.
#[test]
fn warns_when_the_session_is_toml_managed() {
    let rig = Rig::new();
    rig.write_meta("m", json!({"tags": {"ptyfile": "/proj/pty.toml", "ptyfile.session": "m"}}));
    let out = rig.ok(&["tag", "m", "manual=yes"]);
    assert_eq!(
        out.stderr,
        "\nWarning: this session is managed by /proj/pty.toml\nRunning 'pty up' will sync tags from the toml and may overwrite this change.\nTo make it permanent, edit the pty.toml file directly.\n"
    );
}

/// node: tests/display-name.test.ts:340-367
#[test]
fn ambiguous_display_name_fails_closed() {
    let rig = Rig::new();
    rig.write_meta("beta", json!({"displayName": "shared"}));
    rig.write_meta("alpha", json!({"displayName": "shared"}));
    let out = rig.run(&["tag", "shared", "role=test"]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr,
        "Session reference \"shared\" is ambiguous. Matching stable session IDs:\n  alpha\n  beta\nUse a stable session ID instead.\n"
    );
}
