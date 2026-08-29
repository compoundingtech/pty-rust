//! Port of tests/rm-kill-ephemeral.test.ts: `pty kill` (stop and keep),
//! `pty rm` (remove an exited session), and `--ephemeral` sessions.
//! Node's `runCli` folds stderr into `stdout` on failure, so "stdout" checks
//! on a failing command look at both streams here.

use pty_conformance::*;
use std::time::Duration;

fn session_files(rig: &Rig, name: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(rig.root())
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|f| f.starts_with(name))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn both(out: &Out) -> String {
    format!("{}{}", out.stdout(), out.stderr())
}

// ── pty kill ──

/// node: tests/rm-kill-ephemeral.test.ts:113
#[test]
fn kill_stops_a_running_session_and_keeps_metadata() {
    let rig = Rig::new();
    let name = unique_id("t");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let out = rig.pty(&["kill", &name]);
    expect_contains(&out.stdout(), "killed");
    std::thread::sleep(Duration::from_millis(500));
    let files = session_files(&rig, &name);
    assert!(files.iter().any(|f| f.ends_with(".json")), "{files:?}");
    assert!(!files.iter().any(|f| f.ends_with(".sock")), "{files:?}");
}

/// node: tests/rm-kill-ephemeral.test.ts:129
#[test]
fn kill_refuses_an_exited_session() {
    let rig = Rig::new();
    let name = unique_id("t");
    rig.daemon(&name, &["true"], DaemonOpts::keep());
    rig.wait_for_exit(&name);
    std::thread::sleep(Duration::from_millis(500));
    let out = rig.pty(&["kill", &name]);
    expect_failure(&out);
    expect_contains(&both(&out), "not running");
}

/// node: tests/rm-kill-ephemeral.test.ts:142
#[test]
fn kill_errors_for_a_nonexistent_session() {
    let rig = Rig::new();
    let out = rig.pty(&["kill", "nope"]);
    expect_failure(&out);
    expect_contains(&both(&out), "not found");
}

/// node: tests/rm-kill-ephemeral.test.ts:149
#[test]
fn kill_fails_loudly_when_the_daemon_does_not_stop() {
    let rig = Rig::new();
    let name = unique_id("t");
    let d = rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let pid = d.pid();
    kill_pid(pid, libc::SIGSTOP);
    let out = rig.pty(&["kill", &name]);
    kill_pid(pid, libc::SIGKILL);
    expect_failure(&out);
    let text = both(&out);
    expect_contains(&text, &format!("daemon PID {pid} is still running after 7s"));
    expect_contains(&text, &format!("{name}.sock may still be owned"));
}

// ── pty rm ──

/// node: tests/rm-kill-ephemeral.test.ts:167
#[test]
fn rm_removes_metadata_for_an_exited_session() {
    let rig = Rig::new();
    let name = unique_id("t");
    rig.daemon(&name, &["true"], DaemonOpts::keep());
    rig.wait_for_exit(&name);
    std::thread::sleep(Duration::from_millis(500));
    assert!(session_files(&rig, &name).iter().any(|f| f.ends_with(".json")));
    let out = rig.pty(&["rm", &name]);
    expect_contains(&out.stdout(), "removed");
    assert_eq!(session_files(&rig, &name), Vec::<String>::new());
}

/// node: tests/rm-kill-ephemeral.test.ts:184
#[test]
fn rm_refuses_a_running_session() {
    let rig = Rig::new();
    let name = unique_id("t");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name());
    let out = rig.pty(&["rm", &name]);
    expect_failure(&out);
    expect_contains(&both(&out), "still running");
}

/// node: tests/rm-kill-ephemeral.test.ts:194
#[test]
fn rm_errors_for_a_nonexistent_session() {
    let rig = Rig::new();
    let out = rig.pty(&["rm", "nope"]);
    expect_failure(&out);
    expect_contains(&both(&out), "not found");
}

// ── --ephemeral ──

/// node: tests/rm-kill-ephemeral.test.ts:204
#[test]
fn ephemeral_cleans_up_every_file_after_exit() {
    let rig = Rig::new();
    let name = unique_id("t");
    let d = rig.daemon_try(&name, &["sh", "-c", "exit 0"], DaemonOpts::no_display_name().ephemeral());
    expect_status(&d.launch, 0);
    let _ = poll_for(Duration::from_secs(6), || session_files(&rig, &name).is_empty());
    assert_eq!(session_files(&rig, &name), Vec::<String>::new());
}

/// node: tests/rm-kill-ephemeral.test.ts:216
#[test]
fn ephemeral_session_is_visible_while_running() {
    let rig = Rig::new();
    let name = unique_id("t");
    rig.daemon(&name, &["cat"], DaemonOpts::no_display_name().ephemeral());
    let out = expect_json(&rig.pty(&["ls", "--json"]));
    assert!(out.as_array().unwrap().iter().any(|s| s["name"] == name), "{out}");
}

/// node: tests/rm-kill-ephemeral.test.ts:227
#[test]
fn ephemeral_session_disappears_from_ls_after_exit() {
    let rig = Rig::new();
    let name = unique_id("t");
    let d = rig.daemon_try(&name, &["sh", "-c", "exit 0"], DaemonOpts::no_display_name().ephemeral());
    expect_status(&d.launch, 0);
    let _ = poll_for(Duration::from_secs(6), || session_files(&rig, &name).is_empty());
    let out = expect_json(&rig.pty(&["ls", "--json"]));
    assert!(!out.as_array().unwrap().iter().any(|s| s["name"] == name), "{out}");
}
