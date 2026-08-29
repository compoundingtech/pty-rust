//! Port of tests/tag-bulk.test.ts: `pty tag` bulk set / remove / combined
//! calls, the error surface, one event per call, and reads by display name.

use pty_conformance::*;
use serde_json::{Value, json};

fn tagged(tags: &[(&str, &str)]) -> DaemonOpts {
    let mut o = DaemonOpts::no_display_name();
    for (k, v) in tags {
        o = o.tag(k, v);
    }
    o
}

fn tags(rig: &Rig, name: &str) -> Value {
    rig.meta(name).unwrap().get("tags").cloned().unwrap_or(Value::Null)
}

fn tags_change_events(rig: &Rig, name: &str) -> Vec<Value> {
    std::fs::read_to_string(rig.root().join(format!("{name}.events.jsonl")))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|e| e["type"] == "tags_change")
        .collect()
}

// ── bulk set ──

/// node: tests/tag-bulk.test.ts:116
#[test]
fn sets_a_single_key_value() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "role=web"]), 0);
    assert_eq!(tags(&rig, &name), json!({"role": "web"}));
}

/// node: tests/tag-bulk.test.ts:125
#[test]
fn sets_multiple_keys_in_one_call() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "role=web", "owner=forge", "env=dev"]), 0);
    assert_eq!(tags(&rig, &name), json!({"role": "web", "owner": "forge", "env": "dev"}));
}

/// node: tests/tag-bulk.test.ts:136
#[test]
fn last_value_wins_for_a_repeated_key() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "color=red", "color=blue"]), 0);
    assert_eq!(tags(&rig, &name)["color"], "blue");
}

/// node: tests/tag-bulk.test.ts:145
#[test]
fn merges_with_existing_tags() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], tagged(&[("existing", "yes")]));
    expect_status(&rig.pty(&["tag", &name, "fresh=1", "another=2"]), 0);
    assert_eq!(tags(&rig, &name), json!({"existing": "yes", "fresh": "1", "another": "2"}));
}

/// node: tests/tag-bulk.test.ts:155
#[test]
fn allows_an_empty_value() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "key="]), 0);
    assert_eq!(tags(&rig, &name), json!({"key": ""}));
}

/// node: tests/tag-bulk.test.ts:164
#[test]
fn splits_on_the_first_equals() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "foo=bar=baz"]), 0);
    assert_eq!(tags(&rig, &name), json!({"foo": "bar=baz"}));
}

/// node: tests/tag-bulk.test.ts:172
#[test]
fn supports_many_tags_in_one_call() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let args: Vec<String> = (0..30).map(|i| format!("k{i}={i}")).collect();
    let mut argv = vec!["tag", name.as_str()];
    argv.extend(args.iter().map(String::as_str));
    expect_status(&rig.pty(&argv), 0);
    let t = tags(&rig, &name);
    assert_eq!(t.as_object().unwrap().len(), 30);
    for i in 0..30 {
        assert_eq!(t[format!("k{i}")], i.to_string());
    }
}

// ── bulk remove ──

/// node: tests/tag-bulk.test.ts:186
#[test]
fn removes_a_single_key() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], tagged(&[("a", "1"), ("b", "2")]));
    expect_status(&rig.pty(&["tag", &name, "--rm", "a"]), 0);
    assert_eq!(tags(&rig, &name), json!({"b": "2"}));
}

/// node: tests/tag-bulk.test.ts:194
#[test]
fn removes_multiple_keys_in_one_call() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], tagged(&[("a", "1"), ("b", "2"), ("c", "3")]));
    expect_status(&rig.pty(&["tag", &name, "--rm", "a", "--rm", "c"]), 0);
    assert_eq!(tags(&rig, &name), json!({"b": "2"}));
}

/// node: tests/tag-bulk.test.ts:202
#[test]
fn rm_of_a_nonexistent_key_is_a_silent_no_op() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], tagged(&[("a", "1")]));
    expect_status(&rig.pty(&["tag", &name, "--rm", "never-was-set"]), 0);
    assert_eq!(tags(&rig, &name), json!({"a": "1"}));
}

/// node: tests/tag-bulk.test.ts:211
#[test]
fn rm_of_the_same_key_twice_is_idempotent() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], tagged(&[("dup", "1")]));
    expect_status(&rig.pty(&["tag", &name, "--rm", "dup", "--rm", "dup"]), 0);
    assert_eq!(tags(&rig, &name), Value::Null);
}

/// node: tests/tag-bulk.test.ts:219
#[test]
fn removing_every_tag_drops_the_field() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], tagged(&[("only", "x")]));
    expect_status(&rig.pty(&["tag", &name, "--rm", "only"]), 0);
    assert_eq!(tags(&rig, &name), Value::Null);
}

// ── combined set + remove ──

/// node: tests/tag-bulk.test.ts:229
#[test]
fn combines_set_and_rm_in_one_call() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], tagged(&[("keep", "yes"), ("drop", "yes")]));
    expect_status(&rig.pty(&["tag", &name, "added=new", "--rm", "drop"]), 0);
    assert_eq!(tags(&rig, &name), json!({"keep": "yes", "added": "new"}));
}

/// node: tests/tag-bulk.test.ts:237
#[test]
fn rm_wins_over_set_of_the_same_key() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag", &name, "k=v", "--rm", "k"]), 0);
    assert_eq!(tags(&rig, &name), Value::Null);
}

/// node: tests/tag-bulk.test.ts:247
#[test]
fn set_and_rm_are_position_independent() {
    let rig = Rig::new();
    let a = unique_id("tba");
    let b = unique_id("tbb");
    rig.daemon(&a, &["cat"], tagged(&[("drop", "yes")]));
    rig.daemon(&b, &["cat"], tagged(&[("drop", "yes")]));
    expect_status(&rig.pty(&["tag", &a, "fresh=1", "--rm", "drop", "another=2"]), 0);
    expect_status(&rig.pty(&["tag", &b, "--rm", "drop", "another=2", "fresh=1"]), 0);
    assert_eq!(tags(&rig, &a), tags(&rig, &b));
    assert_eq!(tags(&rig, &a), json!({"fresh": "1", "another": "2"}));
}

/// node: tests/tag-bulk.test.ts:259
#[test]
fn interleaved_set_and_rm_apply_updates_first() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], tagged(&[("x", "old"), ("y", "keep")]));
    expect_status(&rig.pty(&["tag", &name, "x=new", "--rm", "y", "z=new", "--rm", "x"]), 0);
    assert_eq!(tags(&rig, &name), json!({"z": "new"}));
}

// ── error surface ──

/// node: tests/tag-bulk.test.ts:271
#[test]
fn rejects_a_positional_without_equals() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["tag", &name, "no-equals-here"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "key=value|--rm");
    assert_eq!(tags(&rig, &name), Value::Null);
}

/// node: tests/tag-bulk.test.ts:281
#[test]
fn rejects_an_empty_key_positional() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["tag", &name, "=value"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "(?i)key");
    assert_eq!(tags(&rig, &name), Value::Null);
}

/// node: tests/tag-bulk.test.ts:290
#[test]
fn rejects_rm_at_the_end_without_a_key() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], tagged(&[("keep", "yes")]));
    let r = rig.pty(&["tag", &name, "--rm"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "--rm");
    assert_eq!(tags(&rig, &name), json!({"keep": "yes"}));
}

/// node: tests/tag-bulk.test.ts:300
#[test]
fn rejects_rm_with_an_empty_key() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["tag", &name, "--rm", ""]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "(?i)key");
}

/// node: tests/tag-bulk.test.ts:308
#[test]
fn errors_on_a_nonexistent_session_ref() {
    let rig = Rig::new();
    let r = rig.pty(&["tag", "no-such-session", "k=v"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "not found");
}

/// node: tests/tag-bulk.test.ts:314
#[test]
fn rejects_bad_shape_with_no_partial_application() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["tag", &name, "good=yes", "no-equals"]);
    expect_failure(&r);
    assert_eq!(tags(&rig, &name), Value::Null);
}

// ── events ──

/// node: tests/tag-bulk.test.ts:330
#[test]
fn bulk_write_fires_exactly_one_event() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let before = tags_change_events(&rig, &name).len();
    expect_status(&rig.pty(&["tag", &name, "a=1", "b=2", "c=3", "--rm", "z"]), 0);
    let after = tags_change_events(&rig, &name);
    assert_eq!(after.len(), before + 1, "{after:?}");
    let last = after.last().unwrap();
    assert_eq!(last["previous"], json!({}));
    assert_eq!(last["value"], json!({"a": "1", "b": "2", "c": "3"}));
}

/// node: tests/tag-bulk.test.ts:344
#[test]
fn no_op_bulk_write_fires_no_event() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], tagged(&[("stable", "x")]));
    let before = tags_change_events(&rig, &name).len();
    expect_status(&rig.pty(&["tag", &name, "stable=x", "--rm", "nope"]), 0);
    assert_eq!(tags_change_events(&rig, &name).len(), before);
}

/// node: tests/tag-bulk.test.ts:356
#[test]
fn partial_no_op_plus_real_change_fires_one_event() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], tagged(&[("same", "x")]));
    let before = tags_change_events(&rig, &name).len();
    expect_status(&rig.pty(&["tag", &name, "same=x", "new=y"]), 0);
    let after = tags_change_events(&rig, &name);
    assert_eq!(after.len(), before + 1);
    assert_eq!(after.last().unwrap()["value"], json!({"same": "x", "new": "y"}));
}

/// node: tests/tag-bulk.test.ts:367
#[test]
fn set_then_rm_of_a_never_present_key_is_a_no_op() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let before = tags_change_events(&rig, &name).len();
    expect_status(&rig.pty(&["tag", &name, "k=v", "--rm", "k"]), 0);
    assert_eq!(tags_change_events(&rig, &name).len(), before);
}

// ── reads + resolution ──

/// node: tests/tag-bulk.test.ts:382
#[test]
fn no_positionals_dumps_current_tags() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], tagged(&[("role", "web")]));
    let r = rig.pty(&["tag", &name]);
    expect_status(&r, 0);
    expect_contains(&r.stdout(), "role=web");
}

/// node: tests/tag-bulk.test.ts:391
#[test]
fn dump_on_empty_says_no_tags() {
    let rig = Rig::new();
    let name = unique_id("tb");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["tag", &name]);
    expect_regex(&r.stdout(), "No tags");
}

/// node: tests/tag-bulk.test.ts:399
#[test]
fn resolves_a_display_name_for_bulk_ops() {
    let rig = Rig::new();
    let stable = unique_id("tb");
    let friendly = unique_id("friendly-");
    let mut opts = DaemonOpts::default();
    opts.display_name = Some(friendly.clone());
    rig.daemon(&stable, &["cat"], opts);
    let r = rig.pty(&["tag", &friendly, "via=displayname", "another=ok"]);
    expect_status(&r, 0);
    assert_eq!(tags(&rig, &stable), json!({"via": "displayname", "another": "ok"}));
}
