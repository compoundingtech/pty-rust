//! Port of tests/parity-fixtures.test.ts: the shared plain-screen fixtures in
//! `tests/fixtures/parity/screens.json` (Node-owned, vendored byte-identical
//! at the workspace root). For each fixture: start the session, wait
//! `settleMs`, run `peek --plain`, and assert the exact plain-screen bytes
//! (plus status / exit code for the after-exit kinds).
//!
//! Node starts the daemon from `PTY_SERVER_CONFIG` with the fixture's rows and
//! cols; every current fixture is 24x80, which is `pty run -d`'s default. A
//! fixture with another size is started attached inside a tty of that size.

use pty_conformance::*;
use serde_json::Value;
use std::time::Duration;

fn screens() -> Value {
    let raw = std::fs::read_to_string(workspace_root().join("tests/fixtures/parity/screens.json"))
        .expect("read screens.json");
    serde_json::from_str(&raw).expect("screens.json is JSON")
}

fn strip_one_trailing_nl(s: &str) -> String {
    s.strip_suffix('\n').unwrap_or(s).to_string()
}

fn env_pairs(fx: &Value) -> Vec<(String, String)> {
    fx["env"]
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default()
}

/// Start the fixture's session; returns the tty client when one was needed.
fn start(rig: &Rig, id: &str, fx: &Value) -> Option<pty_testkit::Session> {
    let command = fx["spawn"]["command"].as_str().expect("spawn.command");
    let args: Vec<&str> = fx["spawn"]["args"]
        .as_array()
        .map(|a| a.iter().map(|v| v.as_str().unwrap_or("")).collect())
        .unwrap_or_default();
    let rows = fx["spawn"]["rows"].as_u64().unwrap_or(24) as u16;
    let cols = fx["spawn"]["cols"].as_u64().unwrap_or(80) as u16;
    let env = env_pairs(fx);
    let env_refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut cmd = vec![command];
    cmd.extend(args);
    if (rows, cols) == (24, 80) {
        let mut opts = DaemonOpts::no_display_name();
        opts.invoke_env = env.clone();
        let d = rig.daemon_try(id, &cmd, opts);
        assert_eq!(d.launch.status, 0, "[{id}] run failed: {}", d.launch.summary());
        None
    } else {
        let mut argv = vec!["run", "--id", id, "--no-display-name", "--"];
        argv.extend(cmd);
        let tty = rig.pty_tty_env(&env_refs, &argv, rows, cols);
        wait_until_for(&format!("[{id}] session files"), Duration::from_secs(30), &mut || {
            rig.meta(id).is_some()
        });
        Some(tty)
    }
}

/// node: tests/parity-fixtures.test.ts:1
#[test]
fn screens_json_matches_the_node_checkout() {
    let Some(node) = node_checkout() else {
        return;
    };
    let ours = std::fs::read(workspace_root().join("tests/fixtures/parity/screens.json")).unwrap();
    let theirs = std::fs::read(node.join("tests/fixtures/parity/screens.json")).unwrap();
    assert!(ours == theirs, "screens.json drifted from the Node checkout");
}

/// node: tests/parity-fixtures.test.ts:130
#[test]
fn shared_parity_fixtures_pass() {
    let doc = screens();
    let arr = doc["fixtures"].as_array().expect("doc.fixtures is an array");
    assert!(!arr.is_empty(), "no fixtures loaded");

    for fx in arr {
        let id = fx["id"].as_str().unwrap_or("?");
        let kind = fx["kind"].as_str().unwrap_or("");
        let settle = fx["settleMs"].as_u64().unwrap_or(500);
        let rig = Rig::new();
        let _tty = start(&rig, id, fx);
        std::thread::sleep(Duration::from_millis(settle));

        match kind {
            "plain-screen" | "plain-screen-after-exit" => {
                let expect_screen = fx["expect"]["plainScreen"].as_str().expect("expect.plainScreen");
                let out = rig.pty(&["peek", "--plain", id]);
                assert_eq!(out.status, 0, "[{id}] peek failed: {}", out.summary());
                let screen = strip_one_trailing_nl(&out.stdout());
                assert_eq!(screen, expect_screen, "[{id}] plainScreen mismatch");
                if let Some(len) = fx["expect"]["plainScreenLength"].as_u64() {
                    assert_eq!(screen.chars().count() as u64, len, "[{id}] plainScreenLength: {screen:?}");
                }
                if let Some(status) = fx["expect"]["status"].as_str() {
                    let entry = rig.list_entry(id).unwrap_or_else(|| panic!("[{id}] not listed"));
                    assert_eq!(entry["status"], status, "[{id}] {entry}");
                }
                if let Some(code) = fx["expect"]["exitCode"].as_i64() {
                    let entry = rig.list_entry(id).unwrap_or_else(|| panic!("[{id}] not listed"));
                    assert_eq!(entry["exitCode"], code, "[{id}] {entry}");
                    // Idempotent: a second peek is byte-identical.
                    let again = rig.pty(&["peek", "--plain", id]);
                    assert_eq!(strip_one_trailing_nl(&again.stdout()), expect_screen, "[{id}] post-exit peek not idempotent");
                }
            }
            "reaped-after-exit" => {
                let out = rig.pty(&["peek", "--plain", id]);
                assert_ne!(out.status, 0, "[{id}] reaped session should not be peekable");
                assert!(rig.list_entry(id).is_none(), "[{id}] reaped session still listed");
            }
            other => panic!("[{id}] unknown fixture kind {other:?}"),
        }
        let _ = rig.pty(&["kill", id]);
    }
}
