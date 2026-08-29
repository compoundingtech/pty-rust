//! `pty tag-multi`: selectors, read and write modes, `--yes`, errors, and
//! its own help.
//!
//! node: tests/tag-multi.test.ts

mod cli_common;

use cli_common::Rig;
use serde_json::json;

fn setup() -> Rig {
    let rig = Rig::new();
    rig.write_meta("a", json!({"tags": {"role": "web", "env": "prod"}}));
    rig.write_meta("b", json!({"tags": {"env": "dev"}, "displayName": "bee"}));
    rig.write_meta("c", json!({}));
    rig
}

/// node: tests/tag-multi.test.ts:120-253
#[test]
fn read_mode() {
    let rig = setup();
    assert_eq!(rig.ok(&["tag-multi", "a"]).stdout, "a:\n  role=web\n  env=prod\n");
    assert_eq!(rig.ok(&["tag-multi", "a", "c"]).stdout, "a:\n  role=web\n  env=prod\nc: (no tags)\n");
    assert_eq!(
        rig.ok(&["tag-multi", "a", "bee", "--json"]).json(),
        json!({"a": {"role": "web", "env": "prod"}, "b": {"env": "dev"}})
    );
    assert_eq!(rig.ok(&["tag-multi", "c", "--json"]).json(), json!({"c": {}}));
    assert_eq!(
        rig.ok(&["tag-multi", "--filter-tag", "role=web", "--json"]).json(),
        json!({"a": {"role": "web", "env": "prod"}})
    );
    assert_eq!(
        rig.ok(&["tag-multi", "--filter-tag", "role=web", "--filter-tag", "env=dev", "--json"]).json(),
        json!({})
    );
    assert_eq!(rig.ok(&["tag-multi", "--filter-tag", "role=none"]).stdout, "0 sessions matched.\n");
    assert_eq!(
        rig.ok(&["tag-multi", "--all", "--json"]).json(),
        json!({"a": {"role": "web", "env": "prod"}, "b": {"env": "dev"}, "c": {}})
    );
    let out = rig.run(&["tag-multi", "no-such-session"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "pty tag-multi: session \"no-such-session\" not found.\n");
}

/// node: tests/tag-multi.test.ts:261-423
#[test]
fn write_mode() {
    let rig = setup();
    assert_eq!(rig.ok(&["tag-multi", "a", "b", "audit=2026-04-25"]).stdout, "2 session(s) processed.\n");
    assert_eq!(rig.read_meta("a").unwrap()["tags"]["audit"], "2026-04-25");
    assert_eq!(rig.read_meta("b").unwrap()["tags"]["audit"], "2026-04-25");
    assert_eq!(rig.events("a").len(), 1);
    assert_eq!(
        rig.ok(&["tag-multi", "a", "b", "--rm", "env", "--rm", "audit", "--json"]).json(),
        json!({"a": {"role": "web"}, "b": {}})
    );
    assert!(rig.read_meta("b").unwrap().get("tags").is_none());
    // A no-op session emits no event.
    rig.ok(&["tag-multi", "a", "role=web"]);
    assert_eq!(rig.events("a").len(), 2);
    // An unresolvable name aborts before any write.
    let out = rig.run(&["tag-multi", "a", "ghost", "x=1"]);
    assert_eq!(out.code, 1);
    assert!(rig.read_meta("a").unwrap()["tags"].get("x").is_none());
    assert_eq!(
        rig.ok(&["tag-multi", "--filter-tag", "role=none", "x=1"]).stdout,
        "0 sessions matched. No writes performed.\n"
    );
    assert_eq!(rig.ok(&["tag-multi", "--filter-tag", "role=none", "x=1", "--json"]).stdout, "{}\n");

    let out = rig.run(&["tag-multi", "--all", "role=web"]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr,
        "pty tag-multi: --all writes are destructive across 3 session(s). Re-run with --yes to apply.\n"
    );
    assert!(rig.read_meta("c").unwrap().get("tags").is_none());
    assert_eq!(rig.ok(&["tag-multi", "--all", "--yes", "stamped=1"]).stdout, "3 session(s) processed.\n");
    assert_eq!(rig.read_meta("c").unwrap()["tags"]["stamped"], "1");
    rig.ok(&["tag-multi", "--all", "-y", "--rm", "stamped"]);
    assert!(rig.read_meta("c").unwrap().get("tags").is_none());
}

/// node: tests/tag-multi.test.ts:431-540
#[test]
fn selector_and_op_errors() {
    let rig = setup();
    let cases: [(&[&str], &str); 11] = [
        (&["tag-multi", "--all", "--filter-tag", "k=v"], "pty tag-multi: selectors are mutually exclusive — pick one of <names>, --filter-tag, --all\n"),
        (&["tag-multi", "--all", "a"], "pty tag-multi: selectors are mutually exclusive — pick one of <names>, --filter-tag, --all\n"),
        (&["tag-multi", "--filter-tag", "k=v", "a"], "pty tag-multi: selectors are mutually exclusive — pick one of <names>, --filter-tag, --all\n"),
        (&["tag-multi"], "pty tag-multi: no selector — pass session names, --filter-tag k=v, or --all\n"),
        (&["tag-multi", "role=web"], "pty tag-multi: no selector — pass session names, --filter-tag k=v, or --all\n"),
        (&["tag-multi", "a", "=value"], "pty tag-multi: empty key in \"=value\". Tag keys must be non-empty.\n"),
        (&["tag-multi", "a", "--rm"], "pty tag-multi: --rm requires a key (e.g. --rm role)\n"),
        (&["tag-multi", "a", "--rm", ""], "pty tag-multi: --rm requires a non-empty key\n"),
        (&["tag-multi", "--filter-tag"], "pty tag-multi: --filter-tag requires k=v\n"),
        (&["tag-multi", "--filter-tag", "no-equals"], "pty tag-multi: --filter-tag value \"no-equals\" must be k=v\n"),
        (&["tag-multi", "--filter-tag", "=v"], "pty tag-multi: --filter-tag key must be non-empty\n"),
    ];
    for (args, expected) in cases {
        let out = rig.run(args);
        assert_eq!(out.code, 1, "{args:?}");
        assert_eq!(out.stderr, expected, "{args:?}");
    }
}

/// node: src/cli.ts:3314-3317, 3489-3511 — the parser's own help.
#[test]
fn parser_help_after_other_args() {
    let rig = setup();
    let parser = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/help/tag-multi-parser.txt"
    ))
    .unwrap();
    let out = rig.ok(&["tag-multi", "--all", "--help"]);
    assert_eq!(out.stdout, parser);
    let entry = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/help/tag-multi.txt"
    ))
    .unwrap();
    assert_eq!(rig.ok(&["tag-multi", "--help"]).stdout, entry);
}
