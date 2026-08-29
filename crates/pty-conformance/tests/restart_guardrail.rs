//! Port of tests/restart-guardrail.test.ts: `restart` refuses stateful agent
//! sessions (`role=agent`, or a `claude --resume` argv) unless `--force`.
//! A `claude` shim on PATH keeps the test from launching a real agent.

use pty_conformance::*;
use std::time::Duration;

const OUTER: (&str, &str) = ("PTY_SESSION", "outer");

fn shim_path(rig: &Rig) -> String {
    let bin = rig.make_dir("bin");
    let shim = bin.join("claude");
    std::fs::write(&shim, "#!/bin/sh\nexec sleep 300\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default())
}

fn create(rig: &Rig, name: &str, extra: &[&str], cmd: &[&str], path: Option<&str>) {
    let mut args = vec!["run", "-d", "--id", name];
    args.extend_from_slice(extra);
    args.push("--");
    args.extend_from_slice(cmd);
    let mut env = vec![OUTER];
    if let Some(p) = path {
        env.push(("PATH", p));
    }
    let r = rig.pty_env(&env, &args);
    expect_status(&r, 0);
}

/// node: tests/restart-guardrail.test.ts:36
#[test]
fn refuses_to_restart_a_role_agent_session() {
    let rig = Rig::new();
    create(&rig, "ag", &["--tag", "role=agent"], &["sleep", "300"], None);
    let r = rig.pty_env(&[OUTER], &["restart", "-y", "ag"]);
    expect_failure(&r);
    let err = r.stderr();
    expect_regex(&err, "stateful agent");
    expect_regex(&err, "role=agent");
    expect_regex(&err, "--force");
    expect_regex(&err, "convoy");
}

/// node: tests/restart-guardrail.test.ts:48
#[test]
fn refuses_to_restart_a_claude_resume_session() {
    let rig = Rig::new();
    let path = shim_path(&rig);
    create(&rig, "cr", &["--no-display-name"], &["claude", "--resume", "ABC-123"], Some(&path));
    let r = rig.pty_env(&[OUTER, ("PATH", &path)], &["restart", "-y", "cr"]);
    expect_failure(&r);
    let err = r.stderr();
    expect_regex(&err, "stateful agent");
    expect_regex(&err, "claude --resume");
}

/// node: tests/restart-guardrail.test.ts:59
#[test]
fn does_not_block_a_normal_session() {
    let rig = Rig::new();
    create(&rig, "plain", &[], &["sleep", "300"], None);
    let r = rig.pty_env(&[OUTER], &["restart", "-y", "plain"]);
    expect_status(&r, 0);
    expect_contains(&r.stdout(), "restarted");
    expect_not_regex(&r.stderr(), "stateful agent");
}

/// node: tests/restart-guardrail.test.ts:69
#[test]
fn force_overrides_the_guardrail() {
    let rig = Rig::new();
    create(&rig, "ag2", &["--tag", "role=agent"], &["sleep", "300"], None);
    // --force also bypasses the nesting guard, so the client goes on to
    // attach and blocks without a tty; it is killed after a few seconds.
    let mut cmd = rig.command(&["restart", "-y", "--force", "ag2"]);
    cmd.env(OUTER.0, OUTER.1);
    let r = rig.run_with_timeout(cmd, None, Duration::from_secs(4));
    expect_not_regex(&r.stderr(), "stateful agent");
    expect_contains(&r.stdout(), "restarted");
}
