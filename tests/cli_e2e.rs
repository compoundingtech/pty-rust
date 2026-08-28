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

// ── The surface a supervisor drives ──────────────────────────────────────────
//
// These tests pin the exact command lines st2 issues. Each one failed before the
// v0 CLI grew the options and the list fields they cover.

/// Read the one-and-only session object out of `pty list --json`.
fn one_session(root: &PathBuf) -> serde_json::Value {
    let (out, _e, code) = run_pty(root, &["list", "--json"]);
    assert_eq!(code, 0, "list --json failed");
    let mut items: Vec<serde_json::Value> = serde_json::from_str(&out).expect("list --json parses");
    assert_eq!(items.len(), 1, "expected exactly one session:\n{out}");
    items.remove(0)
}

#[test]
fn run_accepts_the_full_supervisor_option_set() {
    let root = unique_root();
    let (name, err, code) = run_pty(
        &root,
        &[
            "run",
            "-d",
            "--force",
            "--id",
            "sup.agent",
            "--no-display-name",
            "--cwd",
            "/tmp",
            "--tag",
            "agent.presentation.schema=1",
            "--tag",
            "role=agent",
            "--env",
            "CATALOG=/somewhere",
            "--unset-env",
            "NO_COLOR",
            "--",
            "sh",
            "-c",
            "sleep 30",
        ],
    );
    assert_eq!(code, 0, "supervisor spawn line rejected: {err}");
    assert_eq!(name, "sup.agent");

    let session = one_session(&root);
    assert_eq!(session["status"], "running");
    assert_eq!(session["name"], "sup.agent");
    // A supervisor identifies one generation by pid + creation time. Without both it
    // cannot tell a restarted session from the one it already knew.
    assert!(session["pid"].is_number(), "no pid:\n{session}");
    assert!(session["createdAt"].is_string(), "no createdAt:\n{session}");
    // Tags and display name are read back exactly as they were given at spawn, which is
    // what lets a supervisor skip a separate metadata-patch step entirely.
    assert_eq!(session["tags"]["agent.presentation.schema"], "1");
    assert_eq!(session["tags"]["role"], "agent");
    assert!(session["displayName"].is_null());

    let _ = run_pty(&root, &["kill", "sup.agent"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn run_names_an_unknown_option_instead_of_running_it() {
    let root = unique_root();
    // The parser used to treat an unrecognised token as the start of the command, so
    // this ran `--bogus` as a program and reported only that the daemon never came up.
    let (_o, err, code) = run_pty(
        &root,
        &["run", "--bogus", "--id", "x", "--", "sh", "-c", "true"],
    );
    assert_eq!(code, 2, "unknown option was accepted");
    assert!(err.contains("--bogus"), "error does not name the flag: {err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn run_stores_a_display_name_and_no_display_name_clears_it() {
    let root = unique_root();
    let (_o, _e, code) = run_pty(
        &root,
        &[
            "run", "-d", "--id", "named", "--name", "Friendly", "--cwd", "/tmp", "--", "sh", "-c",
            "sleep 30",
        ],
    );
    assert_eq!(code, 0);
    assert_eq!(one_session(&root)["displayName"], "Friendly");
    let _ = run_pty(&root, &["kill", "named"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn kill_keeps_the_exit_evidence_and_rm_discards_it() {
    let root = unique_root();
    let (_o, _e, code) = run_pty(
        &root,
        &["run", "-d", "--id", "keeper", "--cwd", "/tmp", "--", "sh", "-c", "sleep 30"],
    );
    assert_eq!(code, 0);

    let (_o, _e, code) = run_pty(&root, &["kill", "keeper"]);
    assert_eq!(code, 0);
    std::thread::sleep(Duration::from_millis(400));

    // A supervisor reads how the session ended AFTER it kills it. `kill` used to delete
    // the metadata, which answered "how did it die" with "it was never here".
    let session = one_session(&root);
    assert_eq!(session["status"], "exited");
    assert!(session["exitCode"].is_number(), "no exit code:\n{session}");

    let (out, _e, code) = run_pty(&root, &["rm", "keeper"]);
    assert_eq!(code, 0, "rm failed on a dead session");
    assert!(out.contains("keeper"));

    let (out, _e, _c) = run_pty(&root, &["list", "--json"]);
    assert_eq!(out, "[]", "session survived rm");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn rm_refuses_to_strip_a_running_session() {
    let root = unique_root();
    let (_o, _e, code) = run_pty(
        &root,
        &["run", "-d", "--id", "live", "--cwd", "/tmp", "--", "sh", "-c", "sleep 30"],
    );
    assert_eq!(code, 0);
    let (_o, err, code) = run_pty(&root, &["rm", "live"]);
    assert_eq!(code, 1, "rm removed a live session");
    assert!(err.contains("still running"), "unexpected error: {err}");
    // The session is untouched and still serving.
    assert_eq!(one_session(&root)["status"], "running");
    let _ = run_pty(&root, &["kill", "live"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn send_with_delay_waits_between_sequences() {
    let root = unique_root();
    let (_o, _e, code) = run_pty(
        &root,
        &["run", "-d", "--id", "delayed", "--cwd", "/tmp", "--", "sh", "-c", "sleep 30"],
    );
    assert_eq!(code, 0);

    // Three sequences with a 0.3s gap must take at least two gaps to send. The flag was
    // once parsed and then ignored, which is the same defect as swallowing an unknown
    // option: accepted, not honoured, and it fails somewhere else later.
    let start = Instant::now();
    let (_o, err, code) = run_pty(
        &root,
        &[
            "send", "delayed", "--with-delay", "0.3", "--seq", "a", "--seq", "b", "--seq", "c",
        ],
    );
    assert_eq!(code, 0, "send failed: {err}");
    assert!(
        start.elapsed() >= Duration::from_millis(600),
        "--with-delay was not honoured: took {:?}",
        start.elapsed()
    );

    let (_o, err, code) = run_pty(&root, &["send", "delayed", "--with-delay", "nope", "--seq", "a"]);
    assert_eq!(code, 2, "a bad delay was accepted: {err}");

    let _ = run_pty(&root, &["kill", "delayed"]);
    let _ = std::fs::remove_dir_all(&root);
}
