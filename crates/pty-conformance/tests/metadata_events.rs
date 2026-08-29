//! CLI half of tests/metadata-events.test.ts: `pty metadata patch --id`
//! (JSON on stdin, JSON on stdout, one `metadata_change` event), the
//! `display_name_change` event from `pty rename`, the `tags_change` event
//! from `pty tag`, and the event text rendering through `pty events
//! --recent`. The `patchMetadataById` / `setDisplayName` / `updateTags` /
//! `EventFollower` library cases without a CLI surface stay in Node.

use pty_conformance::*;
use serde_json::{Value, json};

fn read_events(rig: &Rig, name: &str) -> Vec<Value> {
    std::fs::read_to_string(rig.root().join(format!("{name}.events.jsonl")))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("event line is JSON"))
        .collect()
}

fn events_of(rig: &Rig, name: &str, type_: &str) -> Vec<Value> {
    read_events(rig, name).into_iter().filter(|e| e["type"] == type_).collect()
}

fn patch(rig: &Rig, name: &str, body: Value) -> Out {
    rig.pty_stdin(body.to_string().as_bytes(), &["metadata", "patch", "--id", name])
}

// ── metadata patch ──

/// node: tests/metadata-events.test.ts:371
#[test]
fn patch_exposes_the_atomic_operation_through_json_stdin_stdout() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let out = patch(&rig, &name, json!({"displayName": "CLI Worker", "tags": {"role": "worker"}}));
    let v = expect_json(&out);
    assert_eq!(v["changed"], true, "{v}");
    assert_eq!(v["metadata"]["displayName"], "CLI Worker", "{v}");
    assert_eq!(v["metadata"]["tags"], json!({"role": "worker"}), "{v}");
    assert_eq!(events_of(&rig, &name, "metadata_change").len(), 1);
}

/// node: tests/metadata-events.test.ts:390
#[test]
fn patch_refuses_a_same_string_display_name_alias() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["rename", &name, "missing-id"]), 0);
    let out = patch(&rig, "missing-id", json!({"tags": {"wrong": "target"}}));
    expect_failure(&out);
    expect_contains(&out.stderr(), "Session id \"missing-id\" not found");
    assert!(rig.meta(&name).unwrap()["tags"].get("wrong").is_none());
}

/// node: tests/metadata-events.test.ts:408
#[test]
fn patch_reports_actionable_input_errors() {
    let rig = Rig::new();
    let cases: &[(&[&str], &str, &str)] = &[
        (&[], "{}", "missing required --id"),
        (&["--id", "target"], "not-json", "invalid JSON on stdin"),
        (&["--id", "target"], "[]", "Metadata patch must be a JSON object"),
    ];
    for (args, input, message) in cases {
        let mut argv = vec!["metadata", "patch"];
        argv.extend_from_slice(args);
        let out = rig.pty_stdin(input.as_bytes(), &argv);
        assert_ne!(out.status, 0, "{args:?} {input}: {}", out.summary());
        expect_regex(&out.stderr(), message);
    }
}

/// node: tests/metadata-events.test.ts:265
#[test]
fn patch_changes_display_name_and_tags_atomically() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "keep=yes", "replace=old", "remove=old"]), 0);
    let out = patch(
        &rig,
        &name,
        json!({"displayName": "Worker", "tags": {"replace": "new", "remove": null, "added": "yes"}}),
    );
    let v = expect_json(&out);
    assert_eq!(v["changed"], true, "{v}");
    assert_eq!(v["metadata"]["displayName"], "Worker");
    assert_eq!(v["metadata"]["tags"], json!({"keep": "yes", "replace": "new", "added": "yes"}));
    let changes = events_of(&rig, &name, "metadata_change");
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(
        changes[0]["previous"],
        json!({"displayName": null, "tags": {"replace": "old", "remove": "old", "added": null}})
    );
    assert_eq!(
        changes[0]["value"],
        json!({"displayName": "Worker", "tags": {"replace": "new", "remove": null, "added": "yes"}})
    );
}

/// node: tests/metadata-events.test.ts:290
#[test]
fn patch_supports_clear_operations_with_one_event() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(
        &patch(&rig, &name, json!({"displayName": "Before", "tags": {"remove": "yes", "keep": "yes"}})),
        0,
    );
    let v = expect_json(&patch(&rig, &name, json!({"displayName": null, "tags": {"remove": null}})));
    assert!(v["metadata"].get("displayName").is_none(), "{v}");
    assert_eq!(v["metadata"]["tags"], json!({"keep": "yes"}));
    let changes = events_of(&rig, &name, "metadata_change");
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert_eq!(changes[1]["previous"], json!({"displayName": "Before", "tags": {"remove": "yes"}}));
    assert_eq!(changes[1]["value"], json!({"displayName": null, "tags": {"remove": null}}));
}

/// node: tests/metadata-events.test.ts:310
#[test]
fn patch_suppresses_result_and_event_for_a_no_op() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&patch(&rig, &name, json!({"displayName": "Stable", "tags": {"role": "worker"}})), 0);
    let before = events_of(&rig, &name, "metadata_change").len();
    let v = expect_json(&patch(
        &rig,
        &name,
        json!({"displayName": "Stable", "tags": {"role": "worker", "absent": null}}),
    ));
    assert_eq!(v["changed"], false, "{v}");
    assert_eq!(events_of(&rig, &name, "metadata_change").len(), before);
}

/// node: tests/metadata-events.test.ts:339
#[test]
fn patch_rejects_an_invalid_patch_without_writing() {
    let cases: Vec<(Value, &str)> = vec![
        (json!({"displayName": " Worker"}), "Invalid displayName"),
        (json!({"displayName": "Worker\u{2028}Next"}), "Invalid displayName"),
        (json!({"displayName": "Worker\u{2029}Next"}), "Invalid displayName"),
        (json!({"displayName": "😀".repeat(161)}), "Invalid displayName"),
        (json!({"tags": {"": "value"}}), "tag keys must be non-empty"),
        (json!({"tags": {"role": 1}}), "tag values must be strings or null"),
        (json!({"unknown": true}), "unknown field \"unknown\""),
    ];
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let before = std::fs::read(rig.meta_path(&name)).unwrap();
    for (body, message) in cases {
        let out = patch(&rig, &name, body.clone());
        assert_ne!(out.status, 0, "{body}: {}", out.summary());
        expect_contains(&out.stderr(), message);
        assert_eq!(std::fs::read(rig.meta_path(&name)).unwrap(), before, "{body}: metadata changed");
        assert_eq!(events_of(&rig, &name, "metadata_change").len(), 0, "{body}");
    }
}

/// node: tests/metadata-events.test.ts:359
#[test]
fn patch_accepts_the_160_scalar_boundary() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let display_name = format!("{}/a\\b", "😀".repeat(156));
    let v = expect_json(&patch(&rig, &name, json!({"displayName": display_name})));
    assert_eq!(v["metadata"]["displayName"], display_name);
}

// ── display_name_change from `pty rename` ──

/// node: tests/metadata-events.test.ts:428
#[test]
fn rename_emits_display_name_change_with_previous_and_value() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["rename", &name, "my-label"]), 0);
    let ev = events_of(&rig, &name, "display_name_change");
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(ev[0]["previous"], Value::Null);
    assert_eq!(ev[0]["value"], "my-label");
}

/// node: tests/metadata-events.test.ts:441
#[test]
fn rename_clear_emits_a_null_value() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["rename", &name, "initial"]), 0);
    expect_status(&rig.pty(&["rename", "--clear", &name]), 0);
    let changes = events_of(&rig, &name, "display_name_change");
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert_eq!(changes[0]["value"], "initial");
    assert_eq!(changes[1]["previous"], "initial");
    assert_eq!(changes[1]["value"], Value::Null);
}

/// node: tests/metadata-events.test.ts:456
#[test]
fn rename_does_not_emit_on_a_no_op() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["rename", &name, "stable"]), 0);
    let after_first = events_of(&rig, &name, "display_name_change").len();
    expect_status(&rig.pty(&["rename", &name, "stable"]), 0);
    assert_eq!(events_of(&rig, &name, "display_name_change").len(), after_first);
}

/// node: tests/metadata-events.test.ts:469
#[test]
fn rename_clear_does_not_emit_when_already_unset() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let before = events_of(&rig, &name, "display_name_change").len();
    expect_status(&rig.pty(&["rename", "--clear", &name]), 0);
    assert_eq!(events_of(&rig, &name, "display_name_change").len(), before);
}

/// node: tests/metadata-events.test.ts:481
#[test]
fn rename_cli_fires_display_name_change() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["rename", &name, "friendly"]), 0);
    let ev = events_of(&rig, &name, "display_name_change");
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(ev[0]["value"], "friendly");
}

// ── tags_change from `pty tag` ──

/// node: tests/metadata-events.test.ts:526
#[test]
fn tag_emits_tags_change_with_full_snapshots() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "role=web"]), 0);
    let ev = events_of(&rig, &name, "tags_change");
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(ev[0]["previous"], json!({}));
    assert_eq!(ev[0]["value"], json!({"role": "web"}));
}

/// node: tests/metadata-events.test.ts:539
#[test]
fn tag_emits_the_merge_with_previous_tags() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "role=web"]), 0);
    expect_status(&rig.pty(&["tag", &name, "owner=forge"]), 0);
    let changes = events_of(&rig, &name, "tags_change");
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert_eq!(changes[1]["previous"], json!({"role": "web"}));
    assert_eq!(changes[1]["value"], json!({"role": "web", "owner": "forge"}));
}

/// node: tests/metadata-events.test.ts:553
#[test]
fn tag_rm_emits_tags_change() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "a=1", "b=2"]), 0);
    expect_status(&rig.pty(&["tag", &name, "--rm", "a"]), 0);
    let changes = events_of(&rig, &name, "tags_change");
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert_eq!(changes[1]["previous"], json!({"a": "1", "b": "2"}));
    assert_eq!(changes[1]["value"], json!({"b": "2"}));
}

/// node: tests/metadata-events.test.ts:567
#[test]
fn tag_does_not_emit_on_a_no_op() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "role=web"]), 0);
    let before = events_of(&rig, &name, "tags_change").len();
    expect_status(&rig.pty(&["tag", &name, "role=web"]), 0);
    assert_eq!(events_of(&rig, &name, "tags_change").len(), before);
}

/// node: tests/metadata-events.test.ts:580
#[test]
fn tag_rm_of_an_absent_key_does_not_emit() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "role=web"]), 0);
    let before = events_of(&rig, &name, "tags_change").len();
    expect_status(&rig.pty(&["tag", &name, "--rm", "never-was-set"]), 0);
    assert_eq!(events_of(&rig, &name, "tags_change").len(), before);
}

/// node: tests/metadata-events.test.ts:593
#[test]
fn tag_cli_fires_tags_change() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "role=web"]), 0);
    let ev = events_of(&rig, &name, "tags_change");
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(ev[0]["value"]["role"], "web");
}

// ── event text rendering (formatEvent) through `pty events --recent` ──

fn recent_text(rig: &Rig, name: &str) -> String {
    let out = rig.pty(&["events", "--recent", name]);
    expect_status(&out, 0);
    out.stdout()
}

/// node: tests/metadata-events.test.ts:637
#[test]
fn events_text_shows_display_name_change_previous_and_new() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["rename", &name, "old"]), 0);
    expect_status(&rig.pty(&["rename", &name, "new"]), 0);
    let text = recent_text(&rig, &name);
    expect_contains(&text, "display_name ->");
    expect_contains(&text, "\"new\"");
    expect_contains(&text, "\"old\"");
}

/// node: tests/metadata-events.test.ts:649
#[test]
fn events_text_shows_display_name_clear_as_null() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["rename", &name, "old"]), 0);
    expect_status(&rig.pty(&["rename", "--clear", &name]), 0);
    let text = recent_text(&rig, &name);
    expect_contains(&text, "null");
    expect_contains(&text, "\"old\"");
}

/// node: tests/metadata-events.test.ts:660
#[test]
fn events_text_lists_tags_change_as_k_v() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "role=web"]), 0);
    expect_status(&rig.pty(&["tag", &name, "owner=forge"]), 0);
    let text = recent_text(&rig, &name);
    expect_contains(&text, "tags ->");
    expect_contains(&text, "role=web");
    expect_contains(&text, "owner=forge");
}

/// node: tests/metadata-events.test.ts:672
#[test]
fn events_text_renders_empty_tags_as_braces() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "role=web"]), 0);
    expect_status(&rig.pty(&["tag", &name, "--rm", "role"]), 0);
    let text = recent_text(&rig, &name);
    expect_contains(&text, "{}");
}

/// node: tests/metadata-events.test.ts:682
#[test]
fn events_text_renders_metadata_change_snapshots() {
    let rig = Rig::new();
    let name = unique_id("mev");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&patch(&rig, &name, json!({"displayName": "Worker", "tags": {"role": "worker"}})), 0);
    let text = recent_text(&rig, &name);
    expect_contains(&text, "metadata ->");
    expect_contains(&text, "\"Worker\"");
    expect_contains(&text, "\"worker\"");
}
