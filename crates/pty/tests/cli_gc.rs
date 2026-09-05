//! `pty gc`: debris, orphan children, the sweep and `keep`, layout-tag
//! pruning, dry runs, the footer, and `--print-launchd-plist`.
//!
//! node: tests/gc.test.ts, tests/gc-parent-child.test.ts,
//! tests/exit-reap.test.ts:875-932, tests/pty-root.test.ts:146-194

mod cli_common;

use cli_common::{DEAD_PID, Rig, iso_now};
use serde_json::json;

/// node: tests/gc.test.ts:121-135, 198-229, 268-286; tests/exit-reap.test.ts:875-932
#[test]
fn sweeps_gone_sessions_and_keeps_kept_ones() {
    let rig = Rig::new();
    assert_eq!(rig.ok(&["gc"]).stdout, "Nothing to clean up.\n");
    assert_eq!(rig.ok(&["gc", "--dry-run"]).stdout, "Nothing would be cleaned up.\n");
    rig.write_meta("van", json!({}));
    rig.write_meta("ex", json!({"exitCode": 1, "exitedAt": iso_now(0)}));
    rig.write_meta("kept", json!({"exitCode": 0, "exitedAt": iso_now(0), "tags": {"keep": "true"}}));
    rig.write_meta("perm", json!({"tags": {"strategy": "permanent"}}));
    let out = rig.ok(&["gc", "-n"]);
    assert_eq!(
        out.stdout,
        "Would remove: ex\nWould remove: van\nKept (keep tag): kept — swept once dead for 7d, or remove the keep tag to reap it now\nWould clean up 2 stale sessions. (Dry run — no changes made.)\n"
    );
    assert!(rig.exists("van.json") && rig.exists("ex.json"));
    let out = rig.ok(&["gc"]);
    assert_eq!(
        out.stdout,
        "Removed: ex\nRemoved: van\nKept (keep tag): kept — swept once dead for 7d, or remove the keep tag to reap it now\nCleaned up 2 stale sessions.\n"
    );
    assert!(!rig.exists("van.json") && !rig.exists("ex.json"));
    assert!(rig.exists("kept.json") && rig.exists("perm.json"));
    let out = rig.ok(&["gc"]);
    assert_eq!(out.stdout, "Kept (keep tag): kept — swept once dead for 7d, or remove the keep tag to reap it now\nNothing to clean up.\n");
    rig.write_meta("one", json!({}));
    assert_eq!(
        rig.ok(&["gc"]).stdout,
        "Removed: one\nKept (keep tag): kept — swept once dead for 7d, or remove the keep tag to reap it now\nCleaned up 1 stale session.\n"
    );
}

/// node: src/sessions.ts:643-705 — raw debris with a dead pid and no
/// metadata is reclaimed; a live pid file is not.
#[test]
fn reclaims_raw_debris() {
    let rig = Rig::new();
    std::fs::write(rig.path("debris.pid"), DEAD_PID.to_string()).unwrap();
    std::fs::write(rig.path("debris.events.jsonl"), "").unwrap();
    std::fs::write(rig.path("broken.json"), "not json").unwrap();
    std::fs::write(rig.path("mine.pid"), std::process::id().to_string()).unwrap();
    let out = rig.ok(&["gc"]);
    assert_eq!(out.stdout, "Removed: broken\nRemoved: debris\nCleaned up 2 stale sessions.\n");
    assert!(!rig.exists("debris.pid") && !rig.exists("debris.events.jsonl") && !rig.exists("broken.json"));
    assert!(rig.exists("mine.pid"));
}

/// node: tests/gc-parent-child.test.ts:121-224
#[test]
fn kills_orphan_children() {
    let rig = Rig::new();
    rig.write_meta("child-missing", json!({"tags": {"parent": "nonexistent-parent"}}));
    rig.write_meta("dead-parent", json!({"exitCode": 0, "exitedAt": iso_now(0), "tags": {"keep": "true"}}));
    rig.write_meta("child-dead", json!({"tags": {"parent": "dead-parent"}}));
    let out = rig.ok(&["gc", "--dry-run"]);
    assert_eq!(
        out.stdout,
        "Would kill orphan child: child-dead (parent dead-parent dead)\nWould kill orphan child: child-missing (parent nonexistent-parent missing)\nWould remove: child-dead\nWould remove: child-missing\nKept (keep tag): dead-parent — swept once dead for 7d, or remove the keep tag to reap it now\nWould clean up 2 orphan children, 2 stale sessions. (Dry run — no changes made.)\n"
    );
    assert!(rig.exists("child-dead.json"));

    // A live parent keeps its child.
    rig.spawn_cat("live-parent", &[]);
    rig.spawn_cat("happy-child", &["--tag", "parent=live-parent"]);
    let out = rig.ok(&["gc"]);
    assert_eq!(
        out.stdout,
        "Killed orphan child: child-dead (parent dead-parent dead)\nKilled orphan child: child-missing (parent nonexistent-parent missing)\nKept (keep tag): dead-parent — swept once dead for 7d, or remove the keep tag to reap it now\nCleaned up 2 orphan children.\n"
    );
    assert!(!rig.exists("child-dead.json") && !rig.exists("child-missing.json"));
    assert!(rig.exists("happy-child.json") && rig.exists("live-parent.json"));
    let names: Vec<String> = rig
        .ok(&["list", "--json", "--status", "running"])
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["happy-child", "live-parent"]);
}

/// node: tests/gc-parent-child.test.ts:121-138 — a live orphan is SIGTERMed,
/// waited for, and its files removed. The observation is re-checked by
/// `generation`; the interim daemon publishes none, so its exit rewrite
/// reads as stale (`Skipped orphan reap: ... (stale, after signalling)`)
/// until the daemon lane lands.
#[test]
#[ignore = "needs the daemon to publish `generation` (daemon lane)"]
fn reaps_a_live_orphan() {
    let rig = Rig::new();
    rig.spawn_cat("orphan", &["--tag", "parent=nonexistent-parent", "--tag", "strategy=permanent"]);
    let out = rig.ok(&["gc"]);
    assert_eq!(
        out.stdout,
        "Killed orphan child: orphan (parent nonexistent-parent missing)\nCleaned up 1 orphan child.\n"
    );
    assert!(!rig.exists("orphan.json") && !rig.exists("orphan.sock"));
}

/// node: tests/gc.test.ts:137-196, 231-253
#[test]
fn prunes_dead_layout_tags_on_running_sessions() {
    let rig = Rig::new();
    let dead = format!(":l{DEAD_PID}-abc=1");
    let live = format!(":l{}-xyz=1", std::process::id());
    rig.spawn_cat("lay", &["--tag", &dead, "--tag", &live, "--tag", ":layout=grid", "--tag", "role=web"]);
    let out = rig.ok(&["gc", "-n"]);
    assert_eq!(
        out.stdout,
        format!("Would prune orphan tags on lay: #:l{DEAD_PID}-abc\nWould clean up 1 orphan tag. (Dry run — no changes made.)\n")
    );
    let out = rig.ok(&["gc"]);
    assert_eq!(
        out.stdout,
        format!("Pruned orphan tags on lay: #:l{DEAD_PID}-abc\nCleaned up 1 orphan tag.\n")
    );
    let tags = rig.read_meta("lay").unwrap()["tags"].clone();
    assert!(tags.get(format!(":l{DEAD_PID}-abc")).is_none());
    assert_eq!(tags[format!(":l{}-xyz", std::process::id())], "1");
    assert_eq!(tags[":layout"], "grid");
    assert_eq!(tags["role"], "web");
    assert_eq!(rig.ok(&["gc"]).stdout, "Nothing to clean up.\n");
    // Pruning is a tag write: one tags_change event.
    assert_eq!(rig.events("lay").iter().filter(|e| e["type"] == "tags_change").count(), 1);
}

/// node: tests/gc.test.ts:288-327, tests/pty-root.test.ts:146-194
#[test]
fn launchd_plist() {
    let rig = Rig::new();
    let out = rig.ok(&["gc", "--print-launchd-plist"]);
    let base = rig.root.file_name().unwrap().to_str().unwrap();
    let expected = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>com.compoundingtech.pty.gc.{base}</string>\n  <key>ProgramArguments</key>\n  <array>\n    <string>{bin}</string>\n    <string>gc</string>\n  </array>\n  <key>StartInterval</key>\n  <integer>30</integer>\n  <key>RunAtLoad</key>\n  <true/>\n  <key>StandardOutPath</key>\n  <string>{root}/gc.log</string>\n  <key>StandardErrorPath</key>\n  <string>{root}/gc.log</string>\n  <key>EnvironmentVariables</key>\n  <dict>\n    <key>PATH</key>\n    <string>{path}</string>\n    <key>PTY_ROOT</key>\n    <string>{root}</string>\n  </dict>\n</dict>\n</plist>\n",
        bin = cli_common::pty_bin(),
        root = rig.root.display(),
        path = std::env::var("PATH").unwrap_or_default()
    );
    assert_eq!(out.stdout, expected);
    assert!(out.stdout.contains(&format!("<string>{}</string>", cli_common::pty_bin())));
    assert!(!out.stdout.contains("PTY_SESSION_DIR"));

    let out = rig.ok(&["gc", "--print-launchd-plist", "--interval=15"]);
    assert!(out.stdout.contains("<integer>15</integer>"));
    let out = rig.ok(&["gc", "--print-launchd-plist", "--interval", "45"]);
    assert!(out.stdout.contains("<integer>45</integer>"));
    for bad in ["--interval=0", "--interval=abc"] {
        let out = rig.run(&["gc", "--print-launchd-plist", bad]);
        assert_eq!(out.code, 1, "{bad}");
        assert_eq!(
            out.stderr,
            format!("pty gc: --interval expects a positive integer (got \"{}\")\n", &bad["--interval=".len()..])
        );
    }

    // The label suffix is the sanitized basename; the default root has none.
    let weird = rig.scratch.join("weird name with spaces");
    std::fs::create_dir_all(&weird).unwrap();
    let out = rig.run(&["--root", weird.to_str().unwrap(), "gc", "--print-launchd-plist"]);
    assert!(out.stdout.contains("<string>com.compoundingtech.pty.gc.weird-name-with-spaces</string>"));
    let ampersand = rig.scratch.join("a&b");
    std::fs::create_dir_all(&ampersand).unwrap();
    let out = rig.run(&["--root", ampersand.to_str().unwrap(), "gc", "--print-launchd-plist"]);
    assert!(out.stdout.contains(&format!("<string>{}/a&amp;b</string>", rig.scratch.display())));
    let home = rig.scratch.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut c = rig.cmd(&["gc", "--print-launchd-plist"]);
    c.env_remove("PTY_ROOT").env("HOME", &home);
    let out = c.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("<string>com.compoundingtech.pty.gc</string>"), "{stdout}");
    assert!(!stdout.contains("<string>com.compoundingtech.pty.gc."));
}

/// docs/parity.md §12 — the tuning flags of the dropped steps are accepted
/// and ignored.
#[test]
fn dropped_tuning_flags_are_ignored() {
    let rig = Rig::new();
    let out = rig.ok(&["gc", "--idle-days", "14", "--fast-fail-window=10", "--fast-fail-limit", "2"]);
    assert_eq!(out.stdout, "Nothing to clean up.\n");
    assert_eq!(out.stderr, "");
}
