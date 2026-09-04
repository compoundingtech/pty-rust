//! CLI half of tests/tags-helpers.test.ts. The Node file unit-tests
//! `extractFilterTags`, `matchesAllTags`, and `isReservedTagKey`; each rule
//! is observable through `pty list --filter-tag` and the default hashtag
//! rendering, which is what is pinned here. The pure-function half lives in
//! pty-core's unit tests.

use pty_conformance::*;

fn listed_names(rig: &Rig, args: &[&str]) -> Vec<String> {
    let mut argv = vec!["list", "--json"];
    argv.extend_from_slice(args);
    let out = expect_json(&rig.pty(&argv));
    out.as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect()
}

/// node: tests/tags-helpers.test.ts:5
#[test]
fn no_filter_tag_lists_everything() {
    let rig = Rig::new();
    write_fake_metadata(rig.root(), "plain", FakeMeta::created(0));
    write_fake_metadata(rig.root(), "tagged", FakeMeta::created(0).tag("role", "web"));
    assert_eq!(listed_names(&rig, &[]), vec!["plain", "tagged"]);
}

/// node: tests/tags-helpers.test.ts:11
#[test]
fn filter_tag_is_position_independent() {
    let rig = Rig::new();
    write_fake_metadata(rig.root(), "web", FakeMeta::created(0).tag("role", "web"));
    write_fake_metadata(rig.root(), "db", FakeMeta::created(0).tag("role", "db"));
    let before = expect_json(&rig.pty(&["list", "--filter-tag", "role=web", "--json"]));
    let after = expect_json(&rig.pty(&["list", "--json", "--filter-tag", "role=web"]));
    assert_eq!(before, after);
    assert_eq!(before.as_array().unwrap().len(), 1);
    assert_eq!(before[0]["name"], "web");
}

/// node: tests/tags-helpers.test.ts:23
#[test]
fn filter_tag_preserves_equals_signs_in_values() {
    let rig = Rig::new();
    write_fake_metadata(rig.root(), "kv", FakeMeta::created(0).tag("note", "key=value"));
    write_fake_metadata(rig.root(), "other", FakeMeta::created(0).tag("note", "key"));
    assert_eq!(listed_names(&rig, &["--filter-tag", "note=key=value"]), vec!["kv"]);
}

/// node: tests/tags-helpers.test.ts:28
#[test]
fn filter_tag_without_a_value_is_rejected() {
    let rig = Rig::new();
    let out = rig.pty(&["list", "--filter-tag"]);
    expect_failure(&out);
    expect_regex(&out.stderr(), "key=value");
}

/// node: tests/tags-helpers.test.ts:32
#[test]
fn filter_tag_without_an_equals_sign_is_rejected() {
    let rig = Rig::new();
    let out = rig.pty(&["list", "--filter-tag", "nope"]);
    expect_failure(&out);
    expect_regex(&out.stderr(), "key=value");
}

/// node: tests/tags-helpers.test.ts:44
#[test]
fn a_session_with_no_tags_fails_a_non_empty_filter() {
    let rig = Rig::new();
    write_fake_metadata(rig.root(), "untagged", FakeMeta::created(0));
    write_fake_metadata(rig.root(), "empty", FakeMeta::created(0).extra("tags", serde_json::json!({})));
    assert!(listed_names(&rig, &["--filter-tag", "role=web"]).is_empty());
}

/// node: tests/tags-helpers.test.ts:49
#[test]
fn every_filter_tag_must_match() {
    let rig = Rig::new();
    write_fake_metadata(
        rig.root(),
        "both",
        FakeMeta::created(0).tag("role", "web").tag("env", "prod"),
    );
    write_fake_metadata(rig.root(), "one", FakeMeta::created(0).tag("role", "web"));
    assert_eq!(listed_names(&rig, &["--filter-tag", "role=web"]), vec!["both", "one"]);
    assert_eq!(
        listed_names(&rig, &["--filter-tag", "role=web", "--filter-tag", "env=prod"]),
        vec!["both"]
    );
    assert!(listed_names(&rig, &["--filter-tag", "role=web", "--filter-tag", "env=dev"]).is_empty());
}

/// node: tests/tags-helpers.test.ts:56
#[test]
fn filter_values_must_match_exactly() {
    let rig = Rig::new();
    write_fake_metadata(rig.root(), "web", FakeMeta::created(0).tag("role", "web"));
    assert!(listed_names(&rig, &["--filter-tag", "role=Web"]).is_empty());
    assert!(listed_names(&rig, &["--filter-tag", "role="]).is_empty());
}

/// node: tests/tags-helpers.test.ts:64
#[test]
fn reserved_keys_are_hidden_from_the_default_listing() {
    let rig = Rig::new();
    write_fake_metadata(
        rig.root(),
        "svc",
        FakeMeta::created(0)
            .tag("ptyfile", "/p/pty.toml")
            .tag("ptyfile.session", "s")
            .tag("ptyfile.tags", "role")
            .tag("strategy", "permanent")
            .tag(":layout", "grid")
            .tag(":x", "1")
            .tag("role", "web")
            .tag("parent", "root")
            .tag("supervisor.status", "up")
            .tag("ptyfile-extra", "1")
            .tag("strategy.extra", "2"),
    );
    let s = rig.pty(&["list"]).stdout();
    for hidden in ["#ptyfile=", "#ptyfile.session=", "#ptyfile.tags=", "#strategy=", "#:layout=", "#:x="] {
        expect_not_contains(&s, hidden);
    }
    for shown in [
        "#role=web",
        "#parent=root",
        "#supervisor.status=up",
        "#ptyfile-extra=1",
        "#strategy.extra=2",
    ] {
        expect_contains(&s, shown);
    }
}
