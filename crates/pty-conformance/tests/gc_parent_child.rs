//! Port of tests/gc-parent-child.test.ts: the orphan-kill step of `pty gc`
//! for sessions tagged `parent=<id>`. Daemons come from `pty run -d`
//! instead of `node dist/server.js`. The first Node case (a live lock makes
//! gc skip one child and report `reapSkipped`) inspects the library result
//! and is left out.

use pty_conformance::*;
use std::time::Duration;

fn start(rig: &Rig, id: &str, tags: &[(&str, &str)]) -> Daemon {
    let mut opts = DaemonOpts::no_display_name();
    for (k, v) in tags {
        opts = opts.tag(k, v);
    }
    rig.daemon(id, &["cat"], opts)
}

fn kill_hard(pid: i32) {
    kill_pid(pid, libc::SIGKILL);
    assert!(poll_for(Duration::from_secs(5), || !pid_alive(pid)), "daemon {pid} did not die");
    std::thread::sleep(Duration::from_millis(300));
}

/// node: tests/gc-parent-child.test.ts:121
#[test]
fn kills_a_child_whose_parent_daemon_is_dead() {
    let rig = Rig::new();
    let parent = start(&rig, "par1", &[]);
    let child = start(&rig, "ch1", &[("parent", "par1")]);
    kill_hard(parent.pid());
    let out = rig.pty(&["gc"]);
    expect_status(&out, 0);
    expect_contains(&out.stdout(), "Killed orphan child: ch1 (parent par1");
    assert!(!child.meta_path().exists());
}

/// node: tests/gc-parent-child.test.ts:140
#[test]
fn kills_a_child_whose_parent_metadata_is_missing() {
    let rig = Rig::new();
    let child = start(&rig, "ch2", &[("parent", "nonexistent-parent")]);
    let out = rig.pty(&["gc"]);
    expect_status(&out, 0);
    expect_contains(&out.stdout(), "Killed orphan child: ch2 (parent nonexistent-parent missing)");
    assert!(!child.meta_path().exists());
}

/// node: tests/gc-parent-child.test.ts:153
#[test]
fn preserves_a_child_whose_parent_is_alive() {
    let rig = Rig::new();
    let parent = start(&rig, "par3", &[]);
    let child = start(&rig, "ch3", &[("parent", "par3")]);
    let out = rig.pty(&["gc"]);
    expect_status(&out, 0);
    expect_not_contains(&out.stdout(), "Killed orphan child: ch3");
    assert!(parent.meta_path().exists());
    assert!(child.meta_path().exists());
}

/// node: tests/gc-parent-child.test.ts:169
#[test]
fn cycle_kills_both_in_name_order() {
    let rig = Rig::new();
    let a = start(&rig, "acyc", &[("parent", "bcyc")]);
    let b = start(&rig, "bcyc", &[("parent", "acyc")]);
    kill_pid(a.pid(), libc::SIGKILL);
    kill_pid(b.pid(), libc::SIGKILL);
    assert!(poll_for(Duration::from_secs(5), || !pid_alive(a.pid()) && !pid_alive(b.pid())));
    std::thread::sleep(Duration::from_millis(300));
    let out = rig.pty(&["gc"]);
    expect_status(&out, 0);
    let s = out.stdout();
    expect_contains(&s, "Killed orphan child: acyc");
    expect_contains(&s, "Killed orphan child: bcyc");
    let ia = s.find("Killed orphan child: acyc").unwrap();
    let ib = s.find("Killed orphan child: bcyc").unwrap();
    assert!(ia < ib, "a must be handled before b:\n{s}");
}

/// node: tests/gc-parent-child.test.ts:192
#[test]
fn parent_plus_permanent_child_is_killed_not_respawned() {
    let rig = Rig::new();
    let parent = start(&rig, "par5", &[]);
    let child = start(&rig, "ch5", &[("parent", "par5"), ("strategy", "permanent")]);
    kill_hard(parent.pid());
    let out = rig.pty(&["gc"]);
    expect_status(&out, 0);
    expect_contains(&out.stdout(), "Killed orphan child: ch5");
    expect_not_contains(&out.stdout(), "Respawned: ch5");
    assert!(!child.meta_path().exists());
}

/// node: tests/gc-parent-child.test.ts:213
#[test]
fn dry_run_previews_orphan_kill_without_mutating() {
    let rig = Rig::new();
    let child = start(&rig, "ch6", &[("parent", "nonexistent-parent")]);
    let dry = rig.pty(&["gc", "--dry-run"]);
    expect_status(&dry, 0);
    expect_contains(&dry.stdout(), "Would kill orphan child: ch6 (parent nonexistent-parent missing)");
    expect_contains(&dry.stdout(), "Dry run");
    assert!(child.meta_path().exists());
    assert!(pid_alive(child.pid()));
}
