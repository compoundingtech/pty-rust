//! Port of tests/stats-cli.test.ts: `pty stats` text and `--json` shapes,
//! all-sessions mode, the missing-session error, and exited sessions.
//!
//! A note for anyone reading a green run here as new. Before the daemon and
//! the socket verbs landed, four of these tests reported a state nobody had
//! measured. `pty stats` printed its JSON object whatever was asked of it,
//! and two of the tests matched on the bare word "exited" — which appeared
//! inside a registry summary the CLI fell back to when the daemon was gone,
//! not in the reading block the test is about. They passed for the wrong
//! reason. The daemon work then kept a preserved session's daemon alive, the
//! fallback stopped being reached, and the two tests went red while the
//! product got better. They were never really green. If a suite here matches
//! on a bare word rather than a shape, treat a pass as unproven.

use pty_conformance::*;

/// node: tests/stats-cli.test.ts:105
#[test]
fn prints_stats_for_a_named_session() {
    let rig = Rig::new();
    let name = unique_id("s");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let out = rig.pty(&["stats", &name]);
    expect_status(&out, 0);
    let s = out.stdout();
    expect_contains(&s, &format!("Session: {name}"));
    for needle in [
        "Terminal:", "Scrollback:", "Clients:", "Process:", "Modes:", "running", "CPU:", "Memory:",
        "Daemon:",
    ] {
        expect_contains(&s, needle);
    }
}

/// node: tests/stats-cli.test.ts:124
#[test]
fn json_flag_returns_the_documented_shape() {
    let rig = Rig::new();
    let name = unique_id("s");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let stats = expect_json(&rig.pty(&["stats", "--json", &name]));
    assert_eq!(stats["name"], name);
    assert!(stats["terminal"].is_object(), "{stats}");
    assert_eq!(stats["terminal"]["cols"], 80);
    assert_eq!(stats["terminal"]["rows"], 24);
    assert_eq!(stats["terminal"]["scrollbackCapacity"], 24 + 10000);
    assert_eq!(stats["process"]["alive"], true);
    assert!(stats["process"]["pid"].is_number(), "{stats}");
    assert!(stats["process"]["resources"].is_object(), "{stats}");
    assert!(stats["process"]["resources"]["rssKb"].is_number(), "{stats}");
    assert!(stats["process"]["resources"]["cpuPercent"].is_number(), "{stats}");
    assert!(stats["daemon"].is_object(), "{stats}");
    assert!(stats["daemon"]["pid"].is_number(), "{stats}");
    assert!(stats["daemon"]["resources"]["rssKb"].is_number(), "{stats}");
    assert!(stats["clients"].is_object(), "{stats}");
    assert!(stats["modes"].is_object(), "{stats}");
}

/// node: tests/stats-cli.test.ts:149
#[test]
fn queries_all_running_sessions_without_a_name() {
    let rig = Rig::new();
    let a = unique_id("s");
    let b = unique_id("s");
    rig.daemon(&a, &["cat"], DaemonOpts::no_display_name());
    rig.daemon(&b, &["cat"], DaemonOpts::no_display_name());
    let out = rig.pty(&["stats"]);
    expect_status(&out, 0);
    let s = out.stdout();
    expect_contains(&s, &format!("Session: {a}"));
    expect_contains(&s, &format!("Session: {b}"));
}

/// node: tests/stats-cli.test.ts:162
#[test]
fn fails_for_a_nonexistent_session() {
    let rig = Rig::new();
    let out = rig.pty(&["stats", "nonexistent"]);
    expect_failure(&out);
}

/// node: tests/stats-cli.test.ts:173
#[test]
fn shows_exited_for_a_dead_session() {
    let rig = Rig::new();
    let name = unique_id("s");
    rig.daemon(&name, &["true"], DaemonOpts::keep());
    rig.wait_for_exit(&name);
    let out = rig.pty(&["stats", &name]);
    expect_status(&out, 0);
    expect_contains(&out.stdout(), "exited");
}

/// node: tests/stats-cli.test.ts:185
#[test]
fn reports_reasonable_resource_usage() {
    let rig = Rig::new();
    let name = unique_id("s");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let stats = expect_json(&rig.pty(&["stats", "--json", &name]));
    assert!(stats["process"]["resources"]["rssKb"].as_f64().unwrap() > 0.0, "{stats}");
    assert!(stats["process"]["resources"]["cpuPercent"].as_f64().unwrap() >= 0.0, "{stats}");
    assert!(stats["daemon"]["resources"]["rssKb"].as_f64().unwrap() > 0.0, "{stats}");
    assert!(stats["daemon"]["resources"]["cpuPercent"].as_f64().unwrap() >= 0.0, "{stats}");
    let ppid = stats["process"]["pid"].as_i64().unwrap();
    let dpid = stats["daemon"]["pid"].as_i64().unwrap();
    assert!(ppid > 0 && dpid > 0, "{stats}");
    assert_ne!(ppid, dpid, "{stats}");
}

/// node: tests/stats-cli.test.ts:207
#[test]
fn hides_cpu_and_memory_for_exited_sessions() {
    let rig = Rig::new();
    let name = unique_id("s");
    rig.daemon(&name, &["true"], DaemonOpts::keep());
    rig.wait_for_exit(&name);
    let out = rig.pty(&["stats", &name]);
    expect_status(&out, 0);
    let s = out.stdout();
    expect_contains(&s, "exited");
    expect_not_contains(&s, "CPU:");
    expect_not_contains(&s, "Memory:");
}
