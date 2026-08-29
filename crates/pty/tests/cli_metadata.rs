//! `pty metadata patch --id <id>`: the stdin patch, the result line, and
//! every argument/validation error text.
//!
//! node: tests/metadata-events.test.ts:339-417

mod cli_common;

use cli_common::Rig;
use serde_json::json;

/// node: tests/metadata-events.test.ts:371-388
#[test]
fn patches_display_name_and_tags() {
    let rig = Rig::new();
    rig.write_meta("w", json!({"tags": {"keep": "true"}}));
    let out = rig.run_stdin(
        &["metadata", "patch", "--id", "w"],
        "{\"displayName\":\"CLI Worker\",\"tags\":{\"role\":\"worker\"}}",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let v = out.json();
    assert_eq!(v["changed"], true);
    assert_eq!(v["metadata"]["displayName"], "CLI Worker");
    assert_eq!(v["metadata"]["tags"], json!({"keep": "true", "role": "worker"}));
    assert_eq!(v["metadata"]["command"], "sh");
    assert_eq!(out.stdout.lines().count(), 1);
    let ev = rig.events("w");
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["type"], "metadata_change");
    assert_eq!(ev[0]["previous"], json!({"displayName": null, "tags": {"role": null}}));
    assert_eq!(ev[0]["value"], json!({"displayName": "CLI Worker", "tags": {"role": "worker"}}));
    // A no-op patch: changed false, no event.
    let out = rig.run_stdin(&["metadata", "patch", "--id", "w"], "{\"displayName\":\"CLI Worker\"}");
    assert_eq!(out.json()["changed"], false);
    assert_eq!(rig.events("w").len(), 1);
    // `null` clears; `tags: {k: null}` removes.
    let out = rig.run_stdin(
        &["metadata", "patch", "--id", "w"],
        "{\"displayName\":null,\"tags\":{\"role\":null,\"keep\":null}}",
    );
    assert_eq!(out.json()["changed"], true);
    let meta = rig.read_meta("w").unwrap();
    assert!(meta.get("displayName").is_none());
    assert!(meta.get("tags").is_none());
}

/// node: tests/metadata-events.test.ts:390-417, src/cli.ts:2815-2873
#[test]
fn argument_and_validation_errors() {
    let rig = Rig::new();
    rig.write_meta("target", json!({"displayName": "missing-id"}));
    let out = rig.run_stdin(&["metadata", "patch", "--id", "missing-id"], "{}");
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "pty metadata patch: Session id \"missing-id\" not found.\n");
    let out = rig.run_stdin(&["metadata", "patch"], "{}");
    assert_eq!(out.stderr, "pty metadata patch: missing required --id <stable-id>.\n");
    let out = rig.run_stdin(&["metadata", "patch", "--id", "target"], "not-json");
    assert!(out.stderr.starts_with("pty metadata patch: invalid JSON on stdin: "), "{:?}", out.stderr);
    let out = rig.run_stdin(&["metadata", "patch", "--id", "target"], "[]");
    assert_eq!(out.stderr, "pty metadata patch: Metadata patch must be a JSON object.\n");
    let out = rig.run_stdin(&["metadata", "patch", "--id", "target"], "");
    assert_eq!(
        out.stderr,
        "pty metadata patch: expected one JSON patch object on stdin.\n  Example: printf '%s' '{\"displayName\":\"Worker\"}' | pty metadata patch --id a1b2c3d4\n"
    );
    let out = rig.run_stdin(&["metadata", "patch", "--id"], "{}");
    assert_eq!(out.stderr, "pty metadata patch: --id requires a stable session id.\n");
    let out = rig.run_stdin(&["metadata", "patch", "--id", "a", "--id", "b"], "{}");
    assert_eq!(out.stderr, "pty metadata patch: --id may only be provided once.\n");
    let out = rig.run_stdin(&["metadata", "patch", "--id", "a", "extra"], "{}");
    assert_eq!(
        out.stderr,
        "pty metadata patch: unexpected argument \"extra\".\n  Usage: pty metadata patch --id <stable-id>\n"
    );
    let out = rig.run_stdin(&["metadata", "get"], "{}");
    assert_eq!(
        out.stderr,
        "pty metadata: expected subcommand \"patch\".\n  Usage: pty metadata patch --id <stable-id>\n"
    );
    let cases: [(&str, &str); 5] = [
        ("{\"displayName\":\" Worker\"}", "pty metadata patch: Invalid displayName: Display name must be trimmed.\n"),
        ("{\"tags\":{\"\":\"value\"}}", "pty metadata patch: Metadata patch tag keys must be non-empty.\n"),
        ("{\"tags\":{\"role\":1}}", "pty metadata patch: Metadata patch tag values must be strings or null (invalid key: \"role\").\n"),
        ("{\"unknown\":true}", "pty metadata patch: Metadata patch has unknown field \"unknown\". Allowed fields: displayName, tags.\n"),
        ("{\"displayName\":5}", "pty metadata patch: Metadata patch displayName must be a string or null.\n"),
    ];
    for (input, expected) in cases {
        let out = rig.run_stdin(&["metadata", "patch", "--id", "target"], input);
        assert_eq!(out.code, 1, "{input}");
        assert_eq!(out.stderr, expected, "{input}");
    }
    assert!(rig.events("target").is_empty());
    let help = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/help/metadata.txt"
    ))
    .unwrap();
    assert_eq!(rig.ok(&["metadata", "patch", "--help"]).stdout, help);
}
