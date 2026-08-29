//! Port of tests/up-name-decouple.test.ts: `pty up` spawns sessions with a
//! random short id while the manifest label lives in `displayName`; re-runs
//! match by the `(ptyfile, ptyfile.session)` tag pair; `id` and
//! `display_name` in `pty.toml` pin either half.

use pty_conformance::*;
use std::path::{Path, PathBuf};

const ID_RE: &str = r"^[a-z0-9]{6,12}$";

fn project(rig: &Rig, toml: &str) -> PathBuf {
    let dir = rig.make_dir(&unique_id("p-"));
    std::fs::write(dir.join("pty.toml"), toml).unwrap();
    dir
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// node: tests/up-name-decouple.test.ts:72
#[test]
fn up_spawns_a_random_id_with_the_manifest_label_as_display_name() {
    let rig = Rig::new();
    let proj = project(&rig, "\nprefix = \"myapp\"\n[sessions.web]\ncommand = \"cat\"\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let sessions = rig.list_json();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    let x = &sessions[0];
    expect_regex(x["name"].as_str().unwrap(), ID_RE);
    assert_ne!(x["name"], "myapp-web");
    assert_eq!(x["displayName"], "myapp-web");
    assert_eq!(x["tags"]["ptyfile.session"], "web");
}

/// node: tests/up-name-decouple.test.ts:90
#[test]
fn up_supports_a_prefix_longer_than_the_sock_path_limit() {
    let rig = Rig::new();
    let long_prefix = "p".repeat(90);
    let proj = project(
        &rig,
        &format!("\nprefix = \"{long_prefix}\"\n[sessions.web]\ncommand = \"cat\"\n"),
    );
    let r = rig.pty(&["up", &s(&proj)]);
    expect_status(&r, 0);
    let sessions = rig.list_json();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    assert_eq!(sessions[0]["displayName"], format!("{long_prefix}-web"));
    assert!(sessions[0]["name"].as_str().unwrap().len() < 20, "{}", sessions[0]);
}

/// node: tests/up-name-decouple.test.ts:107
#[test]
fn up_matches_existing_sessions_by_ptyfile_tag_pair() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.svc]\ncommand = \"cat\"\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let first_id = rig
        .list_json()
        .into_iter()
        .find(|x| x["displayName"] == "svc")
        .unwrap()["name"]
        .as_str()
        .unwrap()
        .to_string();
    let second = rig.pty(&["up", &s(&proj)]);
    expect_contains(&second.stdout(), "svc (already running)");
    let sessions: Vec<_> = rig.list_json().into_iter().filter(|x| x["displayName"] == "svc").collect();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    assert_eq!(sessions[0]["name"], first_id);
}

/// node: tests/up-name-decouple.test.ts:124
#[test]
fn up_honors_a_pinned_id() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.svc]\nid = \"pinned\"\ncommand = \"cat\"\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let sessions = rig.list_json();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    assert_eq!(sessions[0]["name"], "pinned");
    assert_eq!(sessions[0]["displayName"], "svc");
}

/// node: tests/up-name-decouple.test.ts:139
#[test]
fn up_honors_display_name_override() {
    let rig = Rig::new();
    let proj = project(
        &rig,
        "\nprefix = \"myapp\"\n[sessions.web]\ndisplay_name = \"My Web Server\"\ncommand = \"cat\"\n",
    );
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let sessions = rig.list_json();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    assert_eq!(sessions[0]["displayName"], "My Web Server");
    expect_regex(sessions[0]["name"].as_str().unwrap(), ID_RE);
}

/// node: tests/up-name-decouple.test.ts:155
#[test]
fn operations_resolve_toml_sessions_by_display_name() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.svc]\ncommand = \"cat\"\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    expect_status(&rig.pty(&["kill", "svc"]), 0);
    let running: Vec<_> = rig.list_json().into_iter().filter(|x| x["status"] == "running").collect();
    assert!(running.is_empty(), "{running:?}");
}
