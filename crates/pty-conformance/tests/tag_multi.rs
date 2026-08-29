//! Port of tests/tag-multi.test.ts: `pty tag-multi` read and write modes
//! over an explicit list, `--filter-tag`, or `--all` (`--yes` for writes),
//! the selector mutex, ops parsing errors, and per-session events.

use pty_conformance::*;
use serde_json::{Value, json};

fn tagged(tags: &[(&str, &str)]) -> DaemonOpts {
    let mut o = DaemonOpts::no_display_name();
    for (k, v) in tags {
        o = o.tag(k, v);
    }
    o
}

fn named(display_name: &str) -> DaemonOpts {
    let mut o = DaemonOpts::default();
    o.display_name = Some(display_name.to_string());
    o
}

fn tags(rig: &Rig, name: &str) -> Value {
    rig.meta(name).unwrap().get("tags").cloned().unwrap_or(Value::Null)
}

fn events(rig: &Rig, name: &str) -> Vec<Value> {
    std::fs::read_to_string(rig.root().join(format!("{name}.events.jsonl")))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect()
}

fn tags_changes(rig: &Rig, name: &str) -> Vec<Value> {
    events(rig, name).into_iter().filter(|e| e["type"] == "tags_change").collect()
}

fn sorted_keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

// ── read mode (explicit list) ──

/// node: tests/tag-multi.test.ts:112
#[test]
fn read_dumps_tags_for_a_single_session() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web")]));
    let r = rig.pty(&["tag-multi", &a]);
    expect_status(&r, 0);
    let s = r.stdout();
    expect_contains(&s, &a);
    expect_contains(&s, "role=web");
}

/// node: tests/tag-multi.test.ts:122
#[test]
fn read_dumps_tags_for_multiple_sessions() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web")]));
    rig.daemon(&b, &["cat"], tagged(&[("role", "db")]));
    let r = rig.pty(&["tag-multi", &a, &b]);
    expect_status(&r, 0);
    let s = r.stdout();
    expect_contains(&s, &a);
    expect_contains(&s, &b);
    expect_contains(&s, "role=web");
    expect_contains(&s, "role=db");
}

/// node: tests/tag-multi.test.ts:136
#[test]
fn read_json_is_keyed_by_session_name() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web")]));
    rig.daemon(&b, &["cat"], tagged(&[("env", "dev")]));
    let v = expect_json(&rig.pty(&["tag-multi", &a, &b, "--json"]));
    assert_eq!(v[&a], json!({"role": "web"}));
    assert_eq!(v[&b], json!({"env": "dev"}));
}

/// node: tests/tag-multi.test.ts:149
#[test]
fn read_json_renders_an_untagged_session_as_empty_object() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    let v = expect_json(&rig.pty(&["tag-multi", &a, "--json"]));
    assert_eq!(v, json!({a.clone(): {}}));
}

/// node: tests/tag-multi.test.ts:158
#[test]
fn read_resolves_display_names() {
    let rig = Rig::new();
    let stable = unique_id("tmu");
    let friendly = unique_id("friendly-");
    rig.daemon(&stable, &["cat"], named(&friendly).tag("role", "web"));
    let v = expect_json(&rig.pty(&["tag-multi", &friendly, "--json"]));
    assert_eq!(v, json!({stable.clone(): {"role": "web"}}));
}

/// node: tests/tag-multi.test.ts:168
#[test]
fn read_errors_when_a_name_is_unresolvable() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["tag-multi", &a, "no-such-session"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "not found|no-such-session");
}

// ── read mode (selectors) ──

/// node: tests/tag-multi.test.ts:180
#[test]
fn filter_tag_matches_sessions_with_that_tag() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    let c = unique_id("tmuc");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web")]));
    rig.daemon(&b, &["cat"], tagged(&[("role", "db")]));
    rig.daemon(&c, &["cat"], tagged(&[("role", "web")]));
    let v = expect_json(&rig.pty(&["tag-multi", "--filter-tag", "role=web", "--json"]));
    let mut expected = vec![a, c];
    expected.sort();
    assert_eq!(sorted_keys(&v), expected);
}

/// node: tests/tag-multi.test.ts:194
#[test]
fn multiple_filter_tags_are_anded() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web"), ("env", "prod")]));
    rig.daemon(&b, &["cat"], tagged(&[("role", "web"), ("env", "dev")]));
    let v = expect_json(&rig.pty(&[
        "tag-multi", "--filter-tag", "role=web", "--filter-tag", "env=prod", "--json",
    ]));
    assert_eq!(v, json!({a.clone(): {"role": "web", "env": "prod"}}));
}

/// node: tests/tag-multi.test.ts:205
#[test]
fn filter_tag_with_zero_matches_is_an_empty_object() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web")]));
    let v = expect_json(&rig.pty(&["tag-multi", "--filter-tag", "role=ghost", "--json"]));
    assert_eq!(v, json!({}));
}

/// node: tests/tag-multi.test.ts:214
#[test]
fn all_reads_every_session_without_yes() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web")]));
    rig.daemon(&b, &["cat"], tagged(&[("role", "db")]));
    let v = expect_json(&rig.pty(&["tag-multi", "--all", "--json"]));
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(sorted_keys(&v), expected);
}

/// node: tests/tag-multi.test.ts:226
#[test]
fn all_on_an_empty_root_is_an_empty_object() {
    let rig = Rig::new();
    let v = expect_json(&rig.pty(&["tag-multi", "--all", "--json"]));
    assert_eq!(v, json!({}));
}

// ── write mode (explicit list) ──

/// node: tests/tag-multi.test.ts:235
#[test]
fn write_sets_a_tag_on_each_named_session() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    rig.daemon(&b, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag-multi", &a, &b, "audit=2026-04-25"]), 0);
    assert_eq!(tags(&rig, &a), json!({"audit": "2026-04-25"}));
    assert_eq!(tags(&rig, &b), json!({"audit": "2026-04-25"}));
}

/// node: tests/tag-multi.test.ts:247
#[test]
fn write_removes_a_tag_on_each_named_session() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web"), ("env", "prod")]));
    rig.daemon(&b, &["cat"], tagged(&[("role", "web")]));
    expect_status(&rig.pty(&["tag-multi", &a, &b, "--rm", "role"]), 0);
    assert_eq!(tags(&rig, &a), json!({"env": "prod"}));
    assert_eq!(tags(&rig, &b), Value::Null);
}

/// node: tests/tag-multi.test.ts:259
#[test]
fn write_combines_set_and_rm_for_each_session() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    rig.daemon(&a, &["cat"], tagged(&[("drop", "yes")]));
    rig.daemon(&b, &["cat"], tagged(&[("drop", "yes")]));
    expect_status(&rig.pty(&["tag-multi", &a, &b, "fresh=1", "--rm", "drop"]), 0);
    assert_eq!(tags(&rig, &a), json!({"fresh": "1"}));
    assert_eq!(tags(&rig, &b), json!({"fresh": "1"}));
}

/// node: tests/tag-multi.test.ts:270
#[test]
fn each_write_fires_its_own_tags_change_event() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    rig.daemon(&b, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag-multi", &a, &b, "role=web"]), 0);
    let ea = tags_changes(&rig, &a);
    let eb = tags_changes(&rig, &b);
    assert_eq!(ea.len(), 1, "{ea:?}");
    assert_eq!(eb.len(), 1, "{eb:?}");
    assert_eq!(ea[0]["value"], json!({"role": "web"}));
    assert_eq!(eb[0]["value"], json!({"role": "web"}));
}

/// node: tests/tag-multi.test.ts:284
#[test]
fn no_op_on_one_session_emits_no_event_for_it() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web")]));
    rig.daemon(&b, &["cat"], DaemonOpts::no_display_name());
    let before_a = tags_changes(&rig, &a).len();
    let before_b = tags_changes(&rig, &b).len();
    expect_status(&rig.pty(&["tag-multi", &a, &b, "role=web"]), 0);
    assert_eq!(tags_changes(&rig, &a).len(), before_a);
    assert_eq!(tags_changes(&rig, &b).len(), before_b + 1);
}

/// node: tests/tag-multi.test.ts:297
#[test]
fn unresolvable_name_means_no_writes_at_all() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["tag-multi", &a, "no-such", "role=web"]);
    expect_failure(&r);
    assert_eq!(tags(&rig, &a), Value::Null);
}

/// node: tests/tag-multi.test.ts:306
#[test]
fn write_resolves_display_names_in_the_list() {
    let rig = Rig::new();
    let a_id = unique_id("tmua");
    let b_id = unique_id("tmub");
    let a_friendly = unique_id("f1-");
    let b_friendly = unique_id("f2-");
    rig.daemon(&a_id, &["cat"], named(&a_friendly));
    rig.daemon(&b_id, &["cat"], named(&b_friendly));
    expect_status(&rig.pty(&["tag-multi", &a_friendly, &b_friendly, "role=web"]), 0);
    assert_eq!(tags(&rig, &a_id), json!({"role": "web"}));
    assert_eq!(tags(&rig, &b_id), json!({"role": "web"}));
}

// ── write mode (selectors) ──

/// node: tests/tag-multi.test.ts:321
#[test]
fn filter_tag_writes_to_each_matching_session() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    let c = unique_id("tmuc");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web")]));
    rig.daemon(&b, &["cat"], tagged(&[("role", "db")]));
    rig.daemon(&c, &["cat"], tagged(&[("role", "web")]));
    expect_status(&rig.pty(&["tag-multi", "--filter-tag", "role=web", "audit=2026-04-25"]), 0);
    assert_eq!(tags(&rig, &a), json!({"role": "web", "audit": "2026-04-25"}));
    assert_eq!(tags(&rig, &b), json!({"role": "db"}));
    assert_eq!(tags(&rig, &c), json!({"role": "web", "audit": "2026-04-25"}));
}

/// node: tests/tag-multi.test.ts:335
#[test]
fn filter_tag_matching_nothing_writes_nothing() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web")]));
    let r = rig.pty(&["tag-multi", "--filter-tag", "role=ghost", "x=1"]);
    expect_status(&r, 0);
    assert_eq!(tags(&rig, &a), json!({"role": "web"}));
}

/// node: tests/tag-multi.test.ts:344
#[test]
fn all_without_yes_is_rejected_for_writes() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["tag-multi", "--all", "role=web"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "--yes");
    assert_eq!(tags(&rig, &a), Value::Null);
}

/// node: tests/tag-multi.test.ts:354
#[test]
fn all_yes_applies_to_every_session() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    rig.daemon(&b, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag-multi", "--all", "--yes", "stamped=1"]), 0);
    assert_eq!(tags(&rig, &a), json!({"stamped": "1"}));
    assert_eq!(tags(&rig, &b), json!({"stamped": "1"}));
}

/// node: tests/tag-multi.test.ts:366
#[test]
fn y_short_form_works_like_yes() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag-multi", "--all", "-y", "role=web"]), 0);
    assert_eq!(tags(&rig, &a), json!({"role": "web"}));
}

// ── selector mutex ──

/// node: tests/tag-multi.test.ts:377
#[test]
fn rejects_all_with_filter_tag() {
    let rig = Rig::new();
    let r = rig.pty(&["tag-multi", "--all", "--filter-tag", "k=v"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "(?i)mutually exclusive|pick one");
}

/// node: tests/tag-multi.test.ts:384
#[test]
fn rejects_all_with_explicit_names() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["tag-multi", "--all", &a]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "(?i)mutually exclusive|pick one");
}

/// node: tests/tag-multi.test.ts:393
#[test]
fn rejects_filter_tag_with_explicit_names() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["tag-multi", "--filter-tag", "k=v", &a]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "(?i)mutually exclusive|pick one");
}

/// node: tests/tag-multi.test.ts:402
#[test]
fn rejects_no_selector() {
    let rig = Rig::new();
    let r = rig.pty(&["tag-multi"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "(?i)selector");
}

/// node: tests/tag-multi.test.ts:409
#[test]
fn rejects_ops_without_a_selector() {
    let rig = Rig::new();
    let r = rig.pty(&["tag-multi", "role=web"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "(?i)selector");
}

// ── ops parsing errors ──

/// node: tests/tag-multi.test.ts:419
#[test]
fn rejects_an_empty_key() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["tag-multi", &a, "=value"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "(?i)key");
    assert_eq!(tags(&rig, &a), Value::Null);
}

/// node: tests/tag-multi.test.ts:429
#[test]
fn rejects_trailing_rm_without_a_key() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], tagged(&[("keep", "yes")]));
    let r = rig.pty(&["tag-multi", &a, "--rm"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "--rm");
    assert_eq!(tags(&rig, &a), json!({"keep": "yes"}));
}

/// node: tests/tag-multi.test.ts:439
#[test]
fn rejects_rm_with_an_empty_key() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    let r = rig.pty(&["tag-multi", &a, "--rm", ""]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "(?i)key");
}

/// node: tests/tag-multi.test.ts:448
#[test]
fn rejects_filter_tag_without_a_value() {
    let rig = Rig::new();
    let r = rig.pty(&["tag-multi", "--filter-tag"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "(?i)--filter-tag|k=v");
}

/// node: tests/tag-multi.test.ts:455
#[test]
fn rejects_filter_tag_without_an_equals_sign() {
    let rig = Rig::new();
    let r = rig.pty(&["tag-multi", "--filter-tag", "no-equals"]);
    expect_failure(&r);
    expect_regex(&r.stderr(), "(?i)filter|k=v");
}

/// node: tests/tag-multi.test.ts:462
#[test]
fn splits_on_the_first_equals() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag-multi", &a, "foo=bar=baz"]), 0);
    assert_eq!(tags(&rig, &a), json!({"foo": "bar=baz"}));
}

/// node: tests/tag-multi.test.ts:470
#[test]
fn rm_wins_over_set_of_the_same_key() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    expect_status(&rig.pty(&["tag-multi", &a, "k=v", "--rm", "k"]), 0);
    assert_eq!(tags(&rig, &a), Value::Null);
}

// ── misc ──

/// node: tests/tag-multi.test.ts:481
#[test]
fn mixed_tagged_and_untagged_reads_are_consistent() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    let c = unique_id("tmuc");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web")]));
    rig.daemon(&b, &["cat"], DaemonOpts::no_display_name());
    rig.daemon(&c, &["cat"], tagged(&[("env", "prod")]));
    let v = expect_json(&rig.pty(&["tag-multi", &a, &b, &c, "--json"]));
    assert_eq!(v, json!({a.clone(): {"role": "web"}, b.clone(): {}, c.clone(): {"env": "prod"}}));
}

/// node: tests/tag-multi.test.ts:497
#[test]
fn write_to_an_empty_selection_emits_no_events() {
    let rig = Rig::new();
    let a = unique_id("tmu");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web")]));
    let before = events(&rig, &a).len();
    expect_status(&rig.pty(&["tag-multi", "--filter-tag", "role=ghost", "x=1"]), 0);
    assert_eq!(events(&rig, &a).len(), before);
}

/// node: tests/tag-multi.test.ts:507
#[test]
fn all_read_matches_per_name_reads() {
    let rig = Rig::new();
    let a = unique_id("tmua");
    let b = unique_id("tmub");
    rig.daemon(&a, &["cat"], tagged(&[("role", "web")]));
    rig.daemon(&b, &["cat"], tagged(&[("env", "prod")]));
    let all = expect_json(&rig.pty(&["tag-multi", "--all", "--json"]));
    let explicit = expect_json(&rig.pty(&["tag-multi", &a, &b, "--json"]));
    assert_eq!(all, explicit);
}
