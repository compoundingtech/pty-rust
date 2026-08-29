//! Port of tests/nesting-prevention.test.ts: `attach`, `restart`, the
//! interactive picker, and `run -a` refuse to start a client inside a session
//! (`PTY_SESSION` set); `--force` overrides. Commands that would block on an
//! attach run inside a real tty via the testkit.

use pty_conformance::*;
use std::time::Duration;

const NESTED: &[(&str, &str)] = &[("PTY_SESSION", "outer-session")];

// ── pty attach ──

/// node: tests/nesting-prevention.test.ts:113
#[test]
fn attach_refuses_when_nested() {
    let rig = Rig::new();
    let target = unique_id("nst");
    rig.daemon(&target, &["cat"], DaemonOpts::no_display_name());
    let out = rig.pty_env(NESTED, &["attach", &target]);
    expect_failure(&out);
    let err = out.stderr();
    expect_contains(&err, "already inside pty session \"outer-session\"");
    expect_regex(&err, "(?i)--force");
}

/// node: tests/nesting-prevention.test.ts:124
#[test]
fn attach_refuses_even_for_a_dead_session() {
    let rig = Rig::new();
    let target = unique_id("nst");
    write_fake_metadata(rig.root(), &target, FakeMeta::created(0).exited(0, 0));
    let out = rig.pty_env(NESTED, &["attach", "-r", &target]);
    expect_failure(&out);
    expect_contains(&out.stderr(), "already inside pty session \"outer-session\"");
    expect_not_contains(&out.stdout(), "Restart?");
}

/// node: tests/nesting-prevention.test.ts:145
#[test]
fn attach_force_bypasses_the_guard() {
    let rig = Rig::new();
    let bogus = unique_id("no-such-");
    let refused = rig.pty_env(NESTED, &["attach", &bogus]);
    expect_failure(&refused);
    expect_contains(&refused.stderr(), "already inside pty session \"outer-session\"");

    let forced = rig.pty_env(NESTED, &["attach", "--force", &bogus]);
    expect_failure(&forced);
    let err = forced.stderr();
    expect_not_contains(&err, "already inside pty session");
    expect_regex(&err, "not found");
}

/// node: tests/nesting-prevention.test.ts:162
#[test]
fn attach_force_may_appear_before_or_after_r() {
    let rig = Rig::new();
    let bogus = unique_id("no-such-");
    let a = rig.pty_env(&[("PTY_SESSION", "outer")], &["attach", "--force", "-r", &bogus]);
    expect_not_contains(&a.stderr(), "already inside pty session");
    expect_regex(&a.stderr(), "not found");
    let b = rig.pty_env(&[("PTY_SESSION", "outer")], &["attach", "-r", "--force", &bogus]);
    expect_not_contains(&b.stderr(), "already inside pty session");
    expect_regex(&b.stderr(), "not found");
}

// ── pty restart ──

/// node: tests/nesting-prevention.test.ts:180
#[test]
fn restart_skips_the_trailing_attach_when_nested() {
    let rig = Rig::new();
    let target = unique_id("nst");
    rig.daemon(&target, &["cat"], DaemonOpts::no_display_name());
    let out = rig.pty_env(NESTED, &["restart", "-y", &target]);
    expect_status(&out, 0);
    let stdout = out.stdout();
    expect_contains(&stdout, &format!("Session \"{target}\" restarted."));
    expect_regex(&stdout, "not attached.*outer-session");
}

/// node: tests/nesting-prevention.test.ts:191
#[test]
fn restart_force_restores_the_attach() {
    let rig = Rig::new();
    let target = unique_id("nst");
    rig.daemon(&target, &["cat"], DaemonOpts::no_display_name());
    // With --force the old behavior returns: restart, then attach. The attach
    // blocks without a tty, so the client is killed after a few seconds; the
    // "not attached" notice must not have been printed.
    let mut cmd = rig.command(&["restart", "-y", "--force", &target]);
    for (k, v) in NESTED {
        cmd.env(k, v);
    }
    let out = rig.run_with_timeout(cmd, None, Duration::from_secs(3));
    let stdout = out.stdout();
    expect_contains(&stdout, &format!("Session \"{target}\" restarted."));
    expect_not_regex(&stdout, "not attached");
}

/// node: tests/nesting-prevention.test.ts:204
#[test]
fn non_nested_restart_attaches_after_restart() {
    let rig = Rig::new();
    let target = unique_id("nst");
    rig.daemon(&target, &["cat"], DaemonOpts::no_display_name());
    let cmd = rig.command(&["restart", "-y", &target]);
    let out = rig.run_with_timeout(cmd, None, Duration::from_secs(3));
    let stdout = out.stdout();
    expect_contains(&stdout, &format!("Session \"{target}\" restarted."));
    expect_not_regex(&stdout, "not attached");
}

// ── pty interactive / bare pty ──

/// node: tests/nesting-prevention.test.ts:217
#[test]
fn bare_pty_refuses_when_nested() {
    let rig = Rig::new();
    let out = rig.pty_env(NESTED, &[]);
    expect_failure(&out);
    let err = out.stderr();
    expect_contains(&err, "already inside pty session");
    expect_regex(&err, "(?i)interactive picker|Ctrl");
}

/// node: tests/nesting-prevention.test.ts:225
#[test]
fn pty_i_refuses_when_nested() {
    let rig = Rig::new();
    let out = rig.pty_env(NESTED, &["i"]);
    expect_failure(&out);
    expect_contains(&out.stderr(), "already inside pty session");
}

/// node: tests/nesting-prevention.test.ts:232
#[test]
fn pty_interactive_refuses_when_nested() {
    let rig = Rig::new();
    let out = rig.pty_env(NESTED, &["interactive"]);
    expect_failure(&out);
    expect_contains(&out.stderr(), "already inside pty session");
}

/// node: tests/nesting-prevention.test.ts:239
#[test]
fn force_bypasses_the_picker_guard() {
    let rig = Rig::new();
    let out = rig.pty_env(NESTED, &["--force"]);
    expect_not_contains(&out.stderr(), "already inside pty session");
}

// ── pty run -a ──

/// node: tests/nesting-prevention.test.ts:249
#[test]
fn run_a_refuses_when_target_is_running_and_nested() {
    let rig = Rig::new();
    let target = unique_id("nst");
    rig.daemon(&target, &["cat"], DaemonOpts::no_display_name());
    let out = rig.pty_env(NESTED, &["run", "-a", "--id", &target, "--", "cat"]);
    expect_failure(&out);
    let err = out.stderr();
    expect_contains(&err, "already inside pty session \"outer-session\"");
    expect_contains(&err, &target);
}

/// node: tests/nesting-prevention.test.ts:260
#[test]
fn run_a_falls_through_to_run_directly_when_target_is_not_running() {
    let rig = Rig::new();
    let target = unique_id("nst");
    let out = rig.pty_env(NESTED, &["run", "-a", "--id", &target, "--", "true"]);
    let err = out.stderr();
    expect_contains(&err, "Already inside pty session");
    expect_contains(&err, "running directly");
}

/// node: tests/nesting-prevention.test.ts:271
#[test]
fn run_force_nested_creates_a_session() {
    let rig = Rig::new();
    let target = unique_id("nst");
    let mut tty = rig.pty_tty_env(NESTED, &["run", "--force", "--id", &target, "--", "cat"], 24, 80);
    wait_until("nested --force session running", || {
        let _ = tty.screenshot();
        rig.list_entry(&target).map(|s| s["status"] == "running").unwrap_or(false)
    });
    tty.close();
}

/// node: tests/nesting-prevention.test.ts:298
#[test]
fn plain_nested_run_runs_directly() {
    let rig = Rig::new();
    let out = rig.pty_env(NESTED, &["run", "--", "true"]);
    let err = out.stderr();
    expect_contains(&err, "Already inside pty session");
    expect_contains(&err, "running directly");
    expect_status(&out, 0);
}
