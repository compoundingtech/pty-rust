//! Port of tests/tag-mutate.test.ts: `pty tag <ref> [k=v ...] [--rm k]`
//! sets, updates, removes, and shows tags on running and exited sessions.

use pty_conformance::*;
use serde_json::json;

fn tagged(tags: &[(&str, &str)]) -> DaemonOpts {
    let mut o = DaemonOpts::no_display_name();
    for (k, v) in tags {
        o = o.tag(k, v);
    }
    o
}

/// node: tests/tag-mutate.test.ts:87
#[test]
fn sets_tags_on_a_running_session() {
    let rig = Rig::new();
    let name = unique_id("tm");
    let d = rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let out = rig.pty(&["tag", &name, "role=server", "env=dev"]);
    expect_status(&out, 0);
    let s = out.stdout();
    expect_contains(&s, "role=server");
    expect_contains(&s, "env=dev");
    assert_eq!(d.meta()["tags"], json!({"role": "server", "env": "dev"}));
}

/// node: tests/tag-mutate.test.ts:100
#[test]
fn updates_existing_tags() {
    let rig = Rig::new();
    let name = unique_id("tm");
    let d = rig.daemon(&name, &["cat"], tagged(&[("role", "old")]));
    expect_status(&rig.pty(&["tag", &name, "role=new"]), 0);
    assert_eq!(d.meta()["tags"]["role"], "new");
}

/// node: tests/tag-mutate.test.ts:110
#[test]
fn removes_tags_with_rm() {
    let rig = Rig::new();
    let name = unique_id("tm");
    let d = rig.daemon(&name, &["cat"], tagged(&[("role", "server"), ("env", "dev")]));
    expect_status(&rig.pty(&["tag", &name, "--rm", "env"]), 0);
    assert_eq!(d.meta()["tags"], json!({"role": "server"}));
}

/// node: tests/tag-mutate.test.ts:121
#[test]
fn removing_every_tag_clears_the_field() {
    let rig = Rig::new();
    let name = unique_id("tm");
    let d = rig.daemon(&name, &["cat"], tagged(&[("only", "tag")]));
    expect_status(&rig.pty(&["tag", &name, "--rm", "only"]), 0);
    assert!(d.meta().get("tags").is_none(), "{}", d.meta());
}

/// node: tests/tag-mutate.test.ts:131
#[test]
fn shows_current_tags_with_no_arguments() {
    let rig = Rig::new();
    let name = unique_id("tm");
    rig.daemon(&name, &["cat"], tagged(&[("role", "server")]));
    let out = rig.pty(&["tag", &name]);
    expect_status(&out, 0);
    expect_contains(&out.stdout(), "role=server");
}

/// node: tests/tag-mutate.test.ts:141
#[test]
fn shows_no_tags_when_empty() {
    let rig = Rig::new();
    let name = unique_id("tm");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let out = rig.pty(&["tag", &name]);
    expect_contains(&out.stdout(), "No tags");
}

/// node: tests/tag-mutate.test.ts:150
#[test]
fn works_on_exited_sessions() {
    let rig = Rig::new();
    let name = unique_id("tm");
    let d = rig.daemon(&name, &["true"], DaemonOpts::keep());
    rig.wait_for_exit(&name);
    let out = rig.pty(&["tag", &name, "strategy=permanent"]);
    expect_status(&out, 0);
    assert_eq!(d.meta()["tags"], json!({"keep": "true", "strategy": "permanent"}));
}

/// node: tests/tag-mutate.test.ts:164
#[test]
fn errors_on_a_nonexistent_session() {
    let rig = Rig::new();
    let out = rig.pty(&["tag", "nonexistent", "foo=bar"]);
    expect_failure(&out);
    expect_contains(&out.stderr(), "not found");
}
