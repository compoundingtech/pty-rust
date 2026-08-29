//! Port of tests/nesting.test.ts: `PTY_SESSION` in the child, and `pty run`
//! inside a session running the command directly instead of nesting.

use pty_conformance::*;
use std::time::Duration;

/// node: tests/nesting.test.ts:106
#[test]
fn sets_pty_session_in_the_child_environment() {
    let rig = Rig::new();
    let name = unique_id("nest");
    rig.daemon(
        &name,
        &["sh", "-c", "echo PTY_SESSION=$PTY_SESSION; exec cat"],
        DaemonOpts::no_display_name(),
    );
    let needle = format!("PTY_SESSION={name}");
    wait_until("PTY_SESSION echoed on screen", || {
        rig.pty(&["peek", "--plain", &name]).stdout().contains(&needle)
    });
}

/// node: tests/nesting.test.ts:119
#[test]
fn nested_run_runs_the_command_directly() {
    let rig = Rig::new();
    let out = rig.pty_env(&[("PTY_SESSION", "outer-session")], &["run", "--", "echo", "hello"]);
    expect_contains(&out.stdout(), "hello");
    let err = out.stderr();
    expect_contains(&err, "Already inside pty session");
    expect_contains(&err, "outer-session");
    expect_status(&out, 0);
    assert_eq!(rig.list_json(), Vec::<serde_json::Value>::new());
}

/// node: tests/nesting.test.ts:137
#[test]
fn nested_run_a_runs_the_command_directly() {
    let rig = Rig::new();
    let out = rig.pty_env(
        &[("PTY_SESSION", "outer-session")],
        &["run", "-a", "--", "echo", "wrapped"],
    );
    expect_contains(&out.stdout(), "wrapped");
    expect_contains(&out.stderr(), "Already inside pty session");
    expect_status(&out, 0);
    assert_eq!(rig.list_json(), Vec::<serde_json::Value>::new());
}

/// node: tests/nesting.test.ts:154
#[test]
fn detached_run_bypasses_the_nesting_check() {
    let rig = Rig::new();
    let name = unique_id("nest");
    let out = rig.pty_env(
        &[("PTY_SESSION", "outer-session")],
        &["run", "-d", "--id", &name, "--", "cat"],
    );
    expect_status(&out, 0);
    expect_not_contains(&out.stderr(), "Already inside pty session");
    let found = rig.list_entry(&name).expect("session created");
    assert_eq!(found["status"], "running");
    assert!(found["pid"].is_number(), "{found}");
}

/// node: tests/nesting.test.ts:177
#[test]
fn nested_run_propagates_the_exit_code() {
    let rig = Rig::new();
    let out = rig.pty_env(
        &[("PTY_SESSION", "outer-session")],
        &["run", "--", "sh", "-c", "exit 42"],
    );
    expect_status(&out, 42);
    expect_contains(&out.stderr(), "Already inside pty session");
}

/// node: tests/nesting.test.ts:189
#[test]
fn no_nesting_check_without_pty_session() {
    let rig = Rig::new();
    let name = unique_id("nest");
    let out = rig.pty_env_unset(&["PTY_SESSION"], &[], &["run", "-d", "--id", &name, "--", "cat"]);
    expect_status(&out, 0);
    expect_not_contains(&out.stderr(), "Already inside pty session");
    wait_until_for("session listed", Duration::from_secs(5), &mut || rig.list_entry(&name).is_some());
}
