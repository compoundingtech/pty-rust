//! Port of tests/exec.test.ts: `pty exec -- <cmd>` inside a session (env
//! `PTY_SESSION` and `PTY_SESSION_GENERATION`) rewrites the stored command
//! under the metadata lock, appends `session_exec`, and replaces itself with
//! the command. Daemons run `cat` (Node used `bash`); the assertions on the
//! stored command are adjusted for that.

use pty_conformance::*;
use serde_json::{Value, json};

fn start(rig: &Rig, id: &str, tags: &[(&str, &str)]) -> Daemon {
    let mut opts = DaemonOpts::no_display_name();
    for (k, v) in tags {
        opts = opts.tag(k, v);
    }
    rig.daemon(id, &["cat"], opts)
}

fn generation(rig: &Rig, id: &str) -> String {
    rig.meta(id).expect("metadata")["generation"].as_str().expect("generation").to_string()
}

fn exec_in(rig: &Rig, id: &str, generation: Option<&str>, cmd: &[&str]) -> Out {
    let generation_value = generation.map(|g| g.to_string()).unwrap_or_else(|| self::generation(rig, id));
    let mut args = vec!["exec", "--"];
    args.extend_from_slice(cmd);
    rig.pty_env(&[("PTY_SESSION", id), ("PTY_SESSION_GENERATION", &generation_value)], &args)
}

fn events(rig: &Rig, id: &str) -> Vec<Value> {
    let raw = std::fs::read_to_string(rig.root().join(format!("{id}.events.jsonl"))).unwrap_or_default();
    raw.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
}

/// node: tests/exec.test.ts:112
#[test]
fn updates_metadata_and_runs_the_new_command() {
    let rig = Rig::new();
    start(&rig, "ex1", &[]);
    let out = exec_in(&rig, "ex1", None, &["echo", "hello-from-exec"]);
    expect_status(&out, 0);
    expect_contains(&out.stdout(), "hello-from-exec");
    let meta = rig.meta("ex1").unwrap();
    assert_eq!(meta["displayCommand"], "echo hello-from-exec");
    assert_eq!(meta["args"], json!(["hello-from-exec"]));
}

/// node: tests/exec.test.ts:133
#[test]
fn errors_when_not_inside_a_pty_session() {
    let rig = Rig::new();
    let out = rig.pty(&["exec", "--", "echo", "hi"]);
    expect_failure(&out);
    expect_contains(&out.stderr(), "not inside a pty session");
}

/// node: tests/exec.test.ts:148
#[test]
fn errors_on_toml_managed_sessions() {
    let rig = Rig::new();
    start(&rig, "ex3", &[("ptyfile", "/some/path/pty.toml"), ("ptyfile.session", "test")]);
    let out = exec_in(&rig, "ex3", None, &["echo", "hi"]);
    expect_failure(&out);
    expect_contains(&out.stderr(), "pty.toml");
}

/// node: tests/exec.test.ts:166
#[test]
fn preserves_existing_tags() {
    let rig = Rig::new();
    start(&rig, "ex4", &[("role", "dev"), ("strategy", "permanent")]);
    exec_in(&rig, "ex4", None, &["echo", "tagged"]);
    let meta = rig.meta("ex4").unwrap();
    assert_eq!(meta["displayCommand"], "echo tagged");
    assert_eq!(meta["tags"]["role"], "dev");
    assert_eq!(meta["tags"]["strategy"], "permanent");
}

/// node: tests/exec.test.ts:183
#[test]
fn propagates_the_exit_code() {
    let rig = Rig::new();
    start(&rig, "ex5", &[]);
    let out = exec_in(&rig, "ex5", None, &["sh", "-c", "exit 42"]);
    expect_status(&out, 42);
}

/// node: tests/exec.test.ts:197
#[test]
fn errors_when_no_command_is_given() {
    let rig = Rig::new();
    let out = rig.pty_env(&[("PTY_SESSION", "test")], &["exec"]);
    expect_failure(&out);
    // This command's usage line. Every other command prints one too, so
    // the bare word proved only that something printed a usage line.
    expect_contains(&out.stderr(), "Usage: pty exec -- <command> [args...]");
}

/// node: tests/exec.test.ts:208
#[test]
fn emits_a_session_exec_event() {
    let rig = Rig::new();
    start(&rig, "ex7", &[]);
    exec_in(&rig, "ex7", None, &["echo", "swapped"]);
    let evs: Vec<Value> = events(&rig, "ex7").into_iter().filter(|e| e["type"] == "session_exec").collect();
    assert!(!evs.is_empty(), "no session_exec event");
    let ev = evs.last().unwrap();
    assert_eq!(ev["session"], "ex7");
    assert_eq!(ev["command"], "echo swapped");
    assert!(!ev["previousCommand"].is_null(), "{ev}");
}

/// node: tests/exec.test.ts:229
#[test]
fn errors_on_a_nonexistent_command() {
    let rig = Rig::new();
    start(&rig, "ex8", &[]);
    let out = exec_in(&rig, "ex8", None, &["/nonexistent/cmd"]);
    expect_failure(&out);
    // Which thing was not found. `pty exec` also reports a session whose
    // metadata it cannot read as "not found", so the bare phrase could not
    // tell a missing command from a broken session lookup.
    expect_contains(&out.stderr(), "Command not found: /nonexistent/cmd");
}

/// node: tests/exec.test.ts:244
#[test]
fn does_not_overwrite_a_writer_holding_the_lock() {
    let rig = Rig::new();
    start(&rig, "ex9", &[("role", "original")]);
    // Hold the metadata lock as a live process (this test's own pid).
    std::fs::write(rig.root().join("ex9.lock"), std::process::id().to_string()).unwrap();
    let mut current = rig.meta("ex9").unwrap();
    current["displayName"] = json!("concurrent-label");
    current["tags"]["role"] = json!("concurrent");
    std::fs::write(rig.meta_path("ex9"), current.to_string()).unwrap();
    let out = exec_in(&rig, "ex9", None, &["echo", "must-not-run"]);
    expect_failure(&out);
    expect_contains(&out.stderr(), "busy");
    let meta = rig.meta("ex9").unwrap();
    assert_eq!(meta["displayName"], "concurrent-label");
    assert_eq!(meta["tags"]["role"], "concurrent");
    assert_eq!(meta["displayCommand"], "cat");
    let _ = std::fs::remove_file(rig.root().join("ex9.lock"));
}

/// node: tests/exec.test.ts:277
#[test]
fn refuses_a_replacement_generation() {
    let rig = Rig::new();
    start(&rig, "ex10", &[("role", "old")]);
    let old_generation = generation(&rig, "ex10");
    let marker = rig.tmp().join("replacement-race-marker");
    let mut replacement = rig.meta("ex10").unwrap();
    replacement["generation"] = json!("replacement-generation");
    replacement["command"] = json!("/bin/sh");
    replacement["args"] = json!(["-c", "sleep 30"]);
    replacement["displayCommand"] = json!("replacement command");
    replacement["displayName"] = json!("replacement label");
    replacement["tags"] = json!({"role": "replacement"});
    std::fs::write(rig.meta_path("ex10"), replacement.to_string()).unwrap();
    let touch = format!("touch '{}'", marker.display());
    let out = exec_in(&rig, "ex10", Some(&old_generation), &["/bin/sh", "-c", &touch]);
    expect_failure(&out);
    expect_contains(&out.stderr(), "replacement generation");
    assert!(!marker.exists(), "command ran");
    assert_eq!(rig.meta("ex10").unwrap(), replacement);
    assert!(events(&rig, "ex10").iter().all(|e| e["type"] != "session_exec"));
}
