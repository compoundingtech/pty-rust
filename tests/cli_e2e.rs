//! End-to-end tests for the `pty` CLI + daemon. Each test uses an isolated
//! `PTY_ROOT` temp dir so it can't touch the real registry or other tests.
//!
//! The interactive `attach` test is itself driven *through* the libghostty test
//! harness — we spawn `pty attach` inside a PTY and assert on the emulated
//! screen. So the CLI's own terminal handling is validated by libghostty too.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use pty_testkit::{Session, SpawnOptions};

fn pty_bin() -> &'static str {
    env!("CARGO_BIN_EXE_pty")
}

fn unique_root() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pty-e2e-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run a `pty` subcommand with the given registry root; return stdout (trimmed).
fn run_pty(root: &PathBuf, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(pty_bin())
        .args(args)
        .env("PTY_ROOT", root)
        .output()
        .expect("spawn pty");
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn run_ls_peek_send_kill_lifecycle() {
    let root = unique_root();
    // Spawn a persistent bash session.
    let (name, _e, code) = run_pty(
        &root,
        &["run", "--rows", "24", "--cols", "80", "--", "bash", "--norc", "--noprofile"],
    );
    assert_eq!(code, 0, "run failed: {_e}");
    assert!(!name.is_empty(), "run printed no session name");

    // ls shows it running.
    let (ls, _e, _c) = run_pty(&root, &["ls"]);
    assert!(ls.contains(&name), "ls missing session:\n{ls}");
    assert!(ls.contains("running"), "session not running:\n{ls}");

    // peek shows the bash prompt.
    let mut prompt_seen = false;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let (screen, _e, _c) = run_pty(&root, &["peek", "--plain", &name]);
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
        let (screen, _e, _c) = run_pty(&root, &["peek", "--plain", &name]);
        // The standalone output line, not just the echoed command.
        if screen.lines().any(|l| l.trim() == "hello-e2e") {
            output_seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(output_seen, "never saw command output via peek");

    // status returns JSON with the name.
    let (status, _e, _c) = run_pty(&root, &["status", &name]);
    assert!(status.contains(&name), "status missing name:\n{status}");

    // kill tears it down.
    let (_o, _e, code) = run_pty(&root, &["kill", &name]);
    assert_eq!(code, 0);
    std::thread::sleep(Duration::from_millis(300));
    let (ls2, _e, _c) = run_pty(&root, &["ls"]);
    assert!(
        !ls2.contains(&name) || ls2.contains("exited") || ls2.contains("No sessions"),
        "session still running after kill:\n{ls2}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn attach_is_interactive_and_detaches() {
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
    let ss = s.wait_for_text("attached-works", 8000).expect("live output");
    assert!(ss.text.contains("attached-works"), "screen:\n{}", ss.text);

    // Ctrl-] (0x1d) detaches without killing the session.
    s.type_str("\x1d");
    std::thread::sleep(Duration::from_millis(400));
    s.close();

    // The session must still be alive after detach.
    let (ls, _e, _c) = run_pty(&root, &["ls"]);
    assert!(
        ls.contains(&name) && ls.contains("running"),
        "session should survive detach:\n{ls}"
    );

    let _ = run_pty(&root, &["kill", &name]);
    let _ = std::fs::remove_dir_all(&root);
}
