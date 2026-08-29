//! Port of tests/up-down.test.ts: `pty up <dir> [names...]` from a
//! `pty.toml` (start, skip running, tag sync, env, cwd, prefix, ptyfile
//! tags, errors) and `pty down`.

use pty_conformance::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn project(rig: &Rig, toml: &str) -> PathBuf {
    let dir = rig.make_dir(&unique_id("proj-"));
    std::fs::write(dir.join("pty.toml"), toml).unwrap();
    dir
}

fn write_toml(dir: &Path, toml: &str) {
    std::fs::write(dir.join("pty.toml"), toml).unwrap();
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn running(rig: &Rig) -> Vec<Value> {
    rig.list_json().into_iter().filter(|x| x["status"] == "running").collect()
}

fn by_display_name(rig: &Rig, dn: &str) -> Value {
    rig.list_json()
        .into_iter()
        .find(|x| x["displayName"] == dn)
        .unwrap_or_else(|| panic!("no session with displayName {dn}"))
}

// ── pty up ──

/// node: tests/up-down.test.ts:83
#[test]
fn up_starts_every_session() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.one]\ncommand = \"cat\"\n[sessions.two]\ncommand = \"cat\"\n");
    let r = rig.pty(&["up", &s(&proj)]);
    expect_status(&r, 0);
    let out = r.stdout();
    expect_contains(&out, "one (started)");
    expect_contains(&out, "two (started)");
    expect_contains(&out, "Started 2 sessions");
    assert_eq!(running(&rig).len(), 2);
}

/// node: tests/up-down.test.ts:100
#[test]
fn up_starts_only_named_sessions() {
    let rig = Rig::new();
    let proj = project(
        &rig,
        "\n[sessions.web]\ncommand = \"cat\"\n[sessions.worker]\ncommand = \"cat\"\n[sessions.db]\ncommand = \"cat\"\n",
    );
    let r = rig.pty(&["up", &s(&proj), "web", "db"]);
    expect_status(&r, 0);
    let out = r.stdout();
    expect_contains(&out, "web (started)");
    expect_contains(&out, "db (started)");
    expect_not_contains(&out, "worker");
    let mut names: Vec<String> = running(&rig)
        .iter()
        .map(|x| x["displayName"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    assert_eq!(names, vec!["db", "web"]);
}

/// node: tests/up-down.test.ts:121
#[test]
fn up_propagates_env_from_the_manifest() {
    let rig = Rig::new();
    let dir = rig.make_dir(&unique_id("proj-"));
    let out_file = dir.join("envcheck.out");
    write_toml(
        &dir,
        &format!(
            "\n[sessions.envprobe]\ncommand = \"echo \\\"$MY_VAR|$ANOTHER\\\" > '{}'; cat\"\n[sessions.envprobe.env]\nMY_VAR = \"hello\"\nANOTHER = \"world\"\n",
            out_file.display()
        ),
    );
    let r = rig.pty(&["up", &s(&dir)]);
    expect_status(&r, 0);
    expect_contains(&r.stdout(), "envprobe (started)");
    let _ = poll_for(Duration::from_secs(3), || out_file.exists());
    assert!(out_file.exists());
    assert_eq!(std::fs::read_to_string(&out_file).unwrap().trim(), "hello|world");
}

/// node: tests/up-down.test.ts:143
#[test]
fn up_skips_already_running_sessions() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.mycat]\ncommand = \"cat\"\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let r = rig.pty(&["up", &s(&proj)]);
    let out = r.stdout();
    expect_contains(&out, "mycat (already running)");
    expect_contains(&out, "All sessions already running");
}

/// node: tests/up-down.test.ts:155
#[test]
fn up_syncs_tags_to_running_sessions() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.syncme]\ncommand = \"cat\"\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    write_toml(
        &proj,
        "\n[sessions.syncme]\ncommand = \"cat\"\ntags = { strategy = \"permanent\", role = \"server\" }\n",
    );
    let r = rig.pty(&["up", &s(&proj)]);
    expect_contains(&r.stdout(), "updated tags: strategy=permanent, role=server");
    let session = by_display_name(&rig, "syncme");
    assert_eq!(session["tags"]["strategy"], "permanent");
    assert_eq!(session["tags"]["role"], "server");
    assert!(session["tags"]["ptyfile"].is_string(), "{session}");
}

/// node: tests/up-down.test.ts:175
#[test]
fn up_keeps_manually_added_tags() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.manual]\ncommand = \"cat\"\ntags = { role = \"server\" }\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    expect_status(&rig.pty(&["tag", "manual", "custom=yes"]), 0);
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let session = by_display_name(&rig, "manual");
    assert_eq!(session["tags"]["role"], "server");
    assert_eq!(session["tags"]["custom"], "yes");
}

/// node: tests/up-down.test.ts:190
#[test]
fn up_removes_tags_removed_from_the_manifest() {
    let rig = Rig::new();
    let proj = project(
        &rig,
        "\n[sessions.remover]\ncommand = \"cat\"\ntags = { role = \"server\", env = \"dev\" }\n",
    );
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let session = by_display_name(&rig, "remover");
    assert_eq!(session["tags"]["role"], "server");
    assert_eq!(session["tags"]["env"], "dev");
    write_toml(&proj, "\n[sessions.remover]\ncommand = \"cat\"\ntags = { role = \"server\" }\n");
    let r = rig.pty(&["up", &s(&proj)]);
    expect_contains(&r.stdout(), "-env");
    let session = by_display_name(&rig, "remover");
    assert_eq!(session["tags"]["role"], "server");
    assert!(session["tags"].get("env").is_none(), "{session}");
    assert!(session["tags"]["ptyfile"].is_string(), "{session}");
    assert_eq!(session["tags"]["ptyfile.session"], "remover");
}

/// node: tests/up-down.test.ts:215
#[test]
fn up_removes_every_manifest_tag_when_the_table_is_deleted() {
    let rig = Rig::new();
    let proj = project(
        &rig,
        "\n[sessions.cleared]\ncommand = \"cat\"\ntags = { role = \"server\", env = \"dev\" }\n",
    );
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    write_toml(&proj, "\n[sessions.cleared]\ncommand = \"cat\"\n");
    let r = rig.pty(&["up", &s(&proj)]);
    let out = r.stdout();
    expect_contains(&out, "-env");
    expect_contains(&out, "-role");
    let session = by_display_name(&rig, "cleared");
    assert!(session["tags"].get("role").is_none(), "{session}");
    assert!(session["tags"].get("env").is_none(), "{session}");
    assert!(session["tags"]["ptyfile"].is_string(), "{session}");
}

/// node: tests/up-down.test.ts:236
#[test]
fn up_preserves_manual_tags_when_manifest_tags_are_removed() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.mixer]\ncommand = \"cat\"\ntags = { role = \"server\" }\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    expect_status(&rig.pty(&["tag", "mixer", "custom=yes"]), 0);
    write_toml(&proj, "\n[sessions.mixer]\ncommand = \"cat\"\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let session = by_display_name(&rig, "mixer");
    assert!(session["tags"].get("role").is_none(), "{session}");
    assert_eq!(session["tags"]["custom"], "yes");
}

/// node: tests/up-down.test.ts:254
#[test]
fn up_replaces_a_manifest_tag_value() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.mover]\ncommand = \"cat\"\ntags = { env = \"dev\" }\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    write_toml(&proj, "\n[sessions.mover]\ncommand = \"cat\"\ntags = { env = \"prod\" }\n");
    let r = rig.pty(&["up", &s(&proj)]);
    let out = r.stdout();
    expect_contains(&out, "env=prod");
    expect_not_contains(&out, "-env");
    assert_eq!(by_display_name(&rig, "mover")["tags"]["env"], "prod");
}

/// node: tests/up-down.test.ts:273
#[test]
fn up_is_quiet_for_running_sessions_with_matching_tags() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.unchanged]\ncommand = \"cat\"\ntags = { role = \"server\" }\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let r = rig.pty(&["up", &s(&proj)]);
    let out = r.stdout();
    expect_contains(&out, "unchanged (already running)");
    expect_not_contains(&out, "updated tags");
}

/// node: tests/up-down.test.ts:286
#[test]
fn up_propagates_tags_from_the_manifest() {
    let rig = Rig::new();
    let proj = project(
        &rig,
        "\n[sessions.tagged]\ncommand = \"cat\"\ntags = { role = \"server\", env = \"dev\" }\n",
    );
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let session = by_display_name(&rig, "tagged");
    assert_eq!(session["tags"]["role"], "server");
    assert_eq!(session["tags"]["env"], "dev");
    assert!(session["tags"]["ptyfile"].is_string(), "{session}");
}

/// node: tests/up-down.test.ts:302
#[test]
fn up_sets_cwd_to_the_project_dir() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.checkdir]\ncommand = \"cat\"\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    assert_eq!(by_display_name(&rig, "checkdir")["cwd"], s(&proj));
}

/// node: tests/up-down.test.ts:315
#[test]
fn up_honors_an_explicit_absolute_cwd() {
    let rig = Rig::new();
    let run_dir = rig.make_dir(&unique_id("run-"));
    let proj = project(
        &rig,
        &format!("\n[sessions.elsewhere]\ncommand = \"cat\"\ncwd = \"{}\"\n", run_dir.display()),
    );
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let session = by_display_name(&rig, "elsewhere");
    assert_eq!(session["cwd"], s(&run_dir));
    assert_ne!(session["cwd"], s(&proj));
}

/// node: tests/up-down.test.ts:330
#[test]
fn up_resolves_a_relative_cwd_against_the_manifest_dir() {
    let rig = Rig::new();
    let proj = rig.make_dir(&unique_id("proj-"));
    let convoy = proj.join(".convoy");
    std::fs::create_dir_all(&convoy).unwrap();
    write_toml(&convoy, "\n[sessions.rooted]\ncommand = \"cat\"\ncwd = \"..\"\n");
    expect_status(&rig.pty(&["up", &s(&convoy)]), 0);
    let session = by_display_name(&rig, "rooted");
    assert_eq!(session["cwd"], s(&proj));
    assert_ne!(session["cwd"], s(&convoy));
}

/// node: tests/up-down.test.ts:346
#[test]
fn up_uses_the_prefix_for_session_names() {
    let rig = Rig::new();
    let proj = project(
        &rig,
        "\nprefix = \"myapp\"\n[sessions.web]\ncommand = \"cat\"\n[sessions.worker]\ncommand = \"cat\"\n",
    );
    let r = rig.pty(&["up", &s(&proj)]);
    expect_status(&r, 0);
    let out = r.stdout();
    expect_contains(&out, "myapp-web (started)");
    expect_contains(&out, "myapp-worker (started)");
    let mut names: Vec<String> = running(&rig)
        .iter()
        .map(|x| x["displayName"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    assert_eq!(names, vec!["myapp-web", "myapp-worker"]);
}

/// node: tests/up-down.test.ts:365
#[test]
fn up_filters_by_short_name_with_a_prefix() {
    let rig = Rig::new();
    let proj = project(
        &rig,
        "\nprefix = \"myapp\"\n[sessions.web]\ncommand = \"cat\"\n[sessions.worker]\ncommand = \"cat\"\n",
    );
    let r = rig.pty(&["up", &s(&proj), "web"]);
    expect_status(&r, 0);
    let out = r.stdout();
    expect_contains(&out, "myapp-web (started)");
    expect_not_contains(&out, "worker");
    let running = running(&rig);
    assert_eq!(running.len(), 1);
    assert_eq!(running[0]["displayName"], "myapp-web");
}

/// node: tests/up-down.test.ts:383
#[test]
fn up_sets_ptyfile_tags() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.tracked]\ncommand = \"cat\"\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let session = by_display_name(&rig, "tracked");
    assert_eq!(session["tags"]["ptyfile"], format!("{}/pty.toml", s(&proj)));
    assert_eq!(session["tags"]["ptyfile.session"], "tracked");
}

/// node: tests/up-down.test.ts:397
#[test]
fn up_errors_on_an_unknown_session_name() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.real]\ncommand = \"cat\"\n");
    let r = rig.pty(&["up", &s(&proj), "fake"]);
    expect_failure(&r);
    let err = r.stderr();
    expect_contains(&err, "Unknown session: fake");
    expect_contains(&err, "Available: real");
}

/// node: tests/up-down.test.ts:409
#[test]
fn up_errors_without_a_manifest() {
    let rig = Rig::new();
    let proj = rig.make_dir(&unique_id("proj-"));
    let r = rig.pty(&["up", &s(&proj)]);
    expect_failure(&r);
    expect_contains(&r.stderr(), "No pty.toml found");
}

/// node: tests/up-down.test.ts:416
#[test]
fn up_errors_on_a_manifest_without_sessions() {
    let rig = Rig::new();
    let proj = project(&rig, "# empty config\n");
    let r = rig.pty(&["up", &s(&proj)]);
    expect_failure(&r);
    expect_contains(&r.stderr(), "No sessions defined");
}

/// node: tests/up-down.test.ts:424
#[test]
fn up_errors_on_a_session_without_a_command() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.bad]\ntags = { foo = \"bar\" }\n");
    let r = rig.pty(&["up", &s(&proj)]);
    expect_failure(&r);
    expect_contains(&r.stderr(), "missing a \"command\" field");
}

// ── pty down ──

/// node: tests/up-down.test.ts:436
#[test]
fn down_stops_every_running_session() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.alpha]\ncommand = \"cat\"\n[sessions.beta]\ncommand = \"cat\"\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    assert_eq!(running(&rig).len(), 2);
    let r = rig.pty(&["down", &s(&proj)]);
    expect_status(&r, 0);
    let out = r.stdout();
    expect_contains(&out, "alpha (stopped)");
    expect_contains(&out, "beta (stopped)");
    expect_contains(&out, "Stopped 2 sessions");
}

/// node: tests/up-down.test.ts:455
#[test]
fn down_stops_only_named_sessions() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.keep]\ncommand = \"cat\"\n[sessions.stop]\ncommand = \"cat\"\n");
    expect_status(&rig.pty(&["up", &s(&proj)]), 0);
    let r = rig.pty(&["down", &s(&proj), "stop"]);
    let out = r.stdout();
    expect_contains(&out, "stop (stopped)");
    expect_not_contains(&out, "keep");
    let running = running(&rig);
    assert_eq!(running.len(), 1);
    assert_eq!(running[0]["displayName"], "keep");
}

/// node: tests/up-down.test.ts:472
#[test]
fn down_reports_nothing_to_stop() {
    let rig = Rig::new();
    let proj = project(&rig, "\n[sessions.ghost]\ncommand = \"cat\"\n");
    let r = rig.pty(&["down", &s(&proj)]);
    expect_contains(&r.stdout(), "No sessions to stop");
}
