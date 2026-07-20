//! End-to-end tests for the `pty` CLI + daemon. Each test uses an isolated
//! `PTY_ROOT` temp dir so it can't touch the real registry or other tests.
//!
//! The interactive `attach` test is itself driven *through* the libghostty test
//! harness — we spawn `pty attach` inside a PTY and assert on the emulated
//! screen. So the CLI's own terminal handling is validated by libghostty too.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pty_testkit::{Session, SpawnOptions};

fn pty_bin() -> &'static str {
    env!("CARGO_BIN_EXE_pty")
}

/// Serialize these heavy real-daemon tests. Rust runs a binary's tests in
/// parallel by default; each of these spawns detached daemons + PTYs, and
/// enough of them at once starve each other's daemon-startup timing (the "green
/// in isolation, red under load" class). Holding this lock keeps at most one
/// running at a time — the same approach the upstream vitest config takes for
/// its heavy PTY tests. Poison-tolerant so one failure doesn't cascade.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn unique_root() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pty-e2e-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run a `pty` subcommand with the given registry root; return
/// `(stdout, stderr, exit_code)`, all trimmed.
fn run_pty(root: &PathBuf, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(pty_bin())
        .args(args)
        .env("PTY_ROOT", root)
        // These tests deliberately CREATE sessions; scrub any ambient
        // PTY_SESSION so nesting-prevention (correct in production) doesn't turn
        // `pty run` into a direct exec when the test harness itself runs inside
        // a pty session.
        .env_remove("PTY_SESSION")
        .output()
        .expect("spawn pty");
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// Run a `pty` subcommand and assert it exited 0; return stdout (trimmed).
fn ok_pty(root: &PathBuf, args: &[&str]) -> String {
    let (out, err, code) = run_pty(root, args);
    assert_eq!(code, 0, "`pty {}` exited {code}: {err}", args.join(" "));
    out
}

#[test]
fn run_ls_peek_send_kill_lifecycle() {
    let _serial = serial();
    let root = unique_root();
    // Spawn a persistent bash session.
    let (name, _e, code) = run_pty(
        &root,
        &["run", "--rows", "24", "--cols", "80", "--", "bash", "--norc", "--noprofile"],
    );
    assert_eq!(code, 0, "run failed: {_e}");
    assert!(!name.is_empty(), "run printed no session name");

    // ls shows it running.
    let ls = ok_pty(&root, &["ls"]);
    assert!(ls.contains(&name), "ls missing session:\n{ls}");
    assert!(ls.contains("running"), "session not running:\n{ls}");

    // peek shows the bash prompt.
    let mut prompt_seen = false;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let screen = ok_pty(&root, &["peek", "--plain", &name]);
        if screen.contains('$') {
            prompt_seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(prompt_seen, "never saw a bash prompt via peek");

    // send a command, then peek for its output.
    let (_o, _e, code) = run_pty(
        &root,
        &["send", &name, "--seq", "echo hello-e2e", "--seq", "key:return"],
    );
    assert_eq!(code, 0);

    let mut output_seen = false;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let screen = ok_pty(&root, &["peek", "--plain", &name]);
        // The standalone output line, not just the echoed command.
        if screen.lines().any(|l| l.trim() == "hello-e2e") {
            output_seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(output_seen, "never saw command output via peek");

    // status returns JSON with the name.
    let status = ok_pty(&root, &["status", &name]);
    assert!(status.contains(&name), "status missing name:\n{status}");

    // kill tears it down.
    let (_o, _e, code) = run_pty(&root, &["kill", &name]);
    assert_eq!(code, 0);
    std::thread::sleep(Duration::from_millis(300));
    let ls2 = ok_pty(&root, &["ls"]);
    assert!(
        !ls2.contains(&name) || ls2.contains("exited") || ls2.contains("No sessions"),
        "session still running after kill:\n{ls2}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn up_and_down_from_a_manifest() {
    let _serial = serial();
    let root = unique_root();
    let work = unique_root();
    std::fs::write(
        work.join("pty.toml"),
        "[sessions.echoer]\ncommand = \"cat\"\n\n[sessions.echoer.env]\nGREETING = \"hi-from-env\"\n",
    )
    .unwrap();
    let work_str = work.to_string_lossy().to_string();

    // up starts the session.
    let (up, _e, code) = run_pty(&root, &["up", &work_str]);
    assert_eq!(code, 0, "up failed: {_e}");
    assert!(up.contains("echoer"), "up output:\n{up}");

    // It shows up in ls as running (on-disk name == the short name).
    let mut running = false;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let ls = ok_pty(&root, &["ls"]);
        if ls.contains("echoer") && ls.contains("running") {
            running = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(running, "manifest session not running after up");

    // down stops it.
    let (_o, _e, code) = run_pty(&root, &["down", &work_str]);
    assert_eq!(code, 0);
    std::thread::sleep(Duration::from_millis(400));
    let ls = ok_pty(&root, &["ls"]);
    assert!(
        !ls.contains("echoer") || ls.contains("No sessions"),
        "session still running after down:\n{ls}"
    );

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn restart_rename_rm() {
    let _serial = serial();
    let root = unique_root();
    // A session that prints its own pid so we can tell restarts apart.
    let (name, _e, code) = run_pty(
        &root,
        &["run", "--id", "svc", "--", "sh", "-c", "echo pid-$$; cat"],
    );
    assert_eq!(code, 0, "run failed: {_e}");
    assert_eq!(name, "svc");

    // Capture the first pid via the pid file.
    let pid_path = root.join("svc.pid");
    let wait_pid = |after: &str| -> String {
        let start = Instant::now();
        loop {
            if let Ok(p) = std::fs::read_to_string(&pid_path) {
                let p = p.trim().to_string();
                if !p.is_empty() && p != after {
                    return p;
                }
            }
            if start.elapsed() > Duration::from_secs(5) {
                panic!("pid file never settled ({after})");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let pid1 = wait_pid("");

    // rename sets a display label that resolves as a lookup key.
    let (_o, _e, code) = run_pty(&root, &["rename", "svc", "My Service"]);
    assert_eq!(code, 0);
    let peek = ok_pty(&root, &["peek", "--plain", "My Service"]);
    assert!(peek.contains("pid-"), "rename lookup failed:\n{peek}");

    // restart respawns with a new pid.
    let (_o, _e, code) = run_pty(&root, &["restart", "svc"]);
    assert_eq!(code, 0);
    let pid2 = wait_pid(&pid1);
    assert_ne!(pid1, pid2, "restart should change the pid");

    // rm removes it entirely.
    let (_o, _e, code) = run_pty(&root, &["rm", "svc"]);
    assert_eq!(code, 0);
    std::thread::sleep(Duration::from_millis(300));
    let ls = ok_pty(&root, &["ls"]);
    assert!(
        !ls.contains("svc") || ls.contains("No sessions"),
        "session remained after rm:\n{ls}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn nesting_prevention_runs_directly_inside_a_session() {
    let _serial = serial();
    let root = unique_root();
    // PTY_SESSION set + no -d: `pty run` should run the command DIRECTLY (no
    // session-in-a-session), so stdout is the command's output, not a session id.
    let out = Command::new(pty_bin())
        .args(["run", "--", "echo", "nested-direct"])
        .env("PTY_ROOT", &root)
        .env("PTY_SESSION", "fake-parent")
        .output()
        .expect("spawn pty");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "nested-direct",
        "should have run the command directly"
    );
    // No session was created.
    let ls = ok_pty(&root, &["ls"]);
    assert!(ls.contains("No sessions"), "a nested session was created:\n{ls}");

    // With -d, it DOES create a background session even inside a session.
    let out = Command::new(pty_bin())
        .args(["run", "-d", "--id", "nbg", "--", "cat"])
        .env("PTY_ROOT", &root)
        .env("PTY_SESSION", "fake-parent")
        .output()
        .expect("spawn pty");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "nbg");
    let ls = ok_pty(&root, &["ls"]);
    assert!(ls.contains("nbg") && ls.contains("running"), "ls:\n{ls}");

    let _ = run_pty(&root, &["kill", "nbg"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn attach_is_interactive_and_detaches() {
    let _serial = serial();
    let root = unique_root();
    let root_str = root.to_string_lossy().to_string();

    // Spawn a session to attach to.
    let (name, _e, code) = run_pty(
        &root,
        &["run", "--rows", "24", "--cols", "80", "--", "bash", "--norc", "--noprofile"],
    );
    assert_eq!(code, 0, "run failed: {_e}");

    // Drive `pty attach` THROUGH the libghostty harness (real PTY stdin/stdout).
    let mut s = Session::spawn(
        pty_bin(),
        &["attach", &name],
        SpawnOptions {
            rows: Some(24),
            cols: Some(80),
            env: vec![("PTY_ROOT".to_string(), root_str.clone())],
            ..Default::default()
        },
    )
    .expect("spawn pty attach");

    // The attach replays the screen (bash prompt) and streams live output.
    s.wait_for_text("$", 8000).expect("attached prompt");
    s.type_str("echo attached-works\r");
    // wait_for_text asserts the live output appeared (errors on timeout).
    s.wait_for_text("attached-works", 8000).expect("live output");

    // Ctrl-] (0x1d) detaches without killing the session.
    s.type_str("\x1d");
    std::thread::sleep(Duration::from_millis(400));
    s.close();

    // The session must still be alive after detach.
    let ls = ok_pty(&root, &["ls"]);
    assert!(
        ls.contains(&name) && ls.contains("running"),
        "session should survive detach:\n{ls}"
    );

    let _ = run_pty(&root, &["kill", &name]);
    let _ = std::fs::remove_dir_all(&root);
}
