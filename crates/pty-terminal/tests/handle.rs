//! `TerminalHandle`: spawning, attaching to a real daemon, attach identity
//! across a replacement under the same id, and late-event rejection.
//!
//! The attach tests need the `pty` binary from this workspace: `PTY_TEST_BIN`
//! if set, else `target/<profile>/pty` (built on demand).

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use pty_terminal::{AttachOptions, HandleEvent, Range, SessionRef, SpawnOptions, TerminalHandle};

fn wait_text(h: &TerminalHandle, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = h.plain(Range::Full);
        if text.contains(needle) {
            return text;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {needle:?}; screen:\n{text}");
        h.wait_rev(h.rev(), Duration::from_millis(200));
    }
}

/// node: tests/pty-handle.test.ts (createPty), issue #1
#[test]
fn spawn_cat_write_and_snapshot() {
    let h = TerminalHandle::spawn("cat", &[], SpawnOptions::default()).expect("spawn");
    assert!(h.wait_ready(Duration::from_secs(1)));
    let events = h.subscribe();
    h.write(b"hello\r");
    let text = wait_text(&h, "hello");
    assert!(text.starts_with("hello"), "{text:?}");
    let g = h.snapshot(0);
    assert_eq!(g.rows[0][..5].iter().map(|c| c.text.as_str()).collect::<String>(), "hello");
    assert!(matches!(events.try_recv(), Ok(HandleEvent::Dirty(_))));
    assert!(!h.exited());
    h.kill();
    assert!(h.exited() || !h.connected());
}

#[test]
fn spawn_reports_exit_code() {
    let h = TerminalHandle::spawn("sh", &["-c", "printf done; exit 3"], SpawnOptions::default()).expect("spawn");
    let events = h.subscribe();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !h.exited() && Instant::now() < deadline {
        h.wait_rev(h.rev(), Duration::from_millis(100));
    }
    assert_eq!(h.exit_code(), Some(3));
    assert_eq!(h.plain(Range::Viewport), "done");
    let mut saw_exit = false;
    while let Ok(ev) = events.try_recv() {
        if ev == HandleEvent::Exited(3) {
            saw_exit = true;
        }
    }
    assert!(saw_exit);
    h.kill();
}

#[test]
fn spawn_resize_changes_the_grid_and_emits_geometry() {
    let h = TerminalHandle::spawn("cat", &[], SpawnOptions::default()).expect("spawn");
    let events = h.subscribe();
    h.resize(40, 10);
    let deadline = Instant::now() + Duration::from_secs(2);
    while (h.cols(), h.rows()) != (40, 10) && Instant::now() < deadline {
        h.wait_rev(h.rev(), Duration::from_millis(50));
    }
    assert_eq!((h.cols(), h.rows()), (40, 10));
    let g = h.snapshot(0);
    assert_eq!((g.cols, g.rows_n), (40, 10));
    let mut saw = false;
    while let Ok(ev) = events.try_recv() {
        if ev == HandleEvent::Geometry(10, 40) {
            saw = true;
        }
    }
    assert!(saw);
    h.kill();
}

// ── against the Rust daemon ──

fn pty_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PTY_TEST_BIN") {
        return Some(PathBuf::from(p));
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.join("../..");
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace.join("target"));
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let bin = target.join(profile).join("pty");
    if !bin.exists() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let ok = Command::new(cargo)
            .args(["build", "-p", "pty"])
            .current_dir(&workspace)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return None;
        }
    }
    bin.exists().then_some(bin)
}

struct Rig {
    bin: PathBuf,
    root: PathBuf,
}

impl Rig {
    fn new() -> Option<Rig> {
        let bin = pty_bin()?;
        let root = std::env::temp_dir().join(format!("pty-handle-{}-{}", std::process::id(), Instant::now().elapsed().as_nanos()));
        std::fs::create_dir_all(&root).ok()?;
        Some(Rig { bin, root })
    }

    fn pty(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(&self.bin)
            .args(args)
            .env("PTY_ROOT", &self.root)
            .env_remove("PTY_SESSION")
            .env_remove("PTY_SESSION_DIR")
            .env_remove("PTY_SERVER_CONFIG")
            .env("PTY_REAP_ON_EXIT", "false")
            .output()
            .expect("run pty");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn run(&self, id: &str, script: &str) {
        let (code, out, err) = self.pty(&["run", "-d", "--id", id, "--", "sh", "-c", script]);
        assert_eq!(code, 0, "pty run failed: {out}{err}");
        // `pty run -d` returns as soon as the session looks up; when a
        // preserved session under the same id exists, that can be before the
        // replacement's socket is listening.
        let sock = self.root.join(format!("{id}.sock"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !sock.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(sock.exists(), "socket for {id} never appeared");
    }

    fn session(&self, id: &str) -> SessionRef {
        SessionRef {
            root: self.root.clone(),
            id: id.to_string(),
        }
    }

    fn kill(&self, id: &str) {
        let _ = self.pty(&["kill", id]);
        let sock = self.root.join(format!("{id}.sock"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while sock.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        for entry in std::fs::read_dir(&self.root).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".pid") {
                let _ = self.pty(&["kill", id]);
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// node: src/tui/builders.ts:600-779 (attachPty): ATTACH, SCREEN replay,
/// DATA, DETACH; readiness is the first SCREEN.
#[test]
fn attach_replays_screen_streams_data_and_detaches() {
    let Some(rig) = Rig::new() else {
        eprintln!("skipping: no pty binary");
        return;
    };
    rig.run("a", "printf 'first\\n'; exec cat");
    let h = TerminalHandle::attach(rig.session("a"), AttachOptions::default()).expect("attach");
    assert!(h.wait_ready(Duration::from_secs(5)), "first SCREEN");
    assert!(h.is_ready());
    let text = h.plain(Range::Full);
    assert!(text.contains("first"), "{text:?}");
    h.write(b"typed\r");
    wait_text(&h, "typed");
    h.kill();
    assert!(!h.connected());
    // The daemon is still running after DETACH.
    assert!(rig.root.join("a.sock").exists());
    rig.kill("a");
}

/// A read-only attach never sends input.
#[test]
fn readonly_attach_drops_input() {
    let Some(rig) = Rig::new() else {
        eprintln!("skipping: no pty binary");
        return;
    };
    rig.run("r", "printf 'ro\\n'; exec cat");
    let h = TerminalHandle::attach(
        rig.session("r"),
        AttachOptions {
            readonly: true,
            ..Default::default()
        },
    )
    .expect("attach");
    assert!(h.wait_ready(Duration::from_secs(5)));
    h.write(b"nope\r");
    std::thread::sleep(Duration::from_millis(300));
    assert!(!h.plain(Range::Full).contains("nope"));
    h.kill();
    rig.kill("r");
}

/// node-daemon-protocol-disk.md §1.12 / conformance fixture "attach identity
/// with a replacement under the same id": `--id a`, exit, `--id a` again — a
/// reconnect reaches the replacement, and nothing from the old daemon (its
/// EXIT, its screen) survives into the new attempt.
#[test]
fn attach_identity_reconnect_reaches_the_replacement() {
    let Some(rig) = Rig::new() else {
        eprintln!("skipping: no pty binary");
        return;
    };
    rig.run("a", "printf 'first\\n'; exec sleep 60");
    let h = TerminalHandle::attach(rig.session("a"), AttachOptions::default()).expect("attach");
    assert!(h.wait_ready(Duration::from_secs(5)));
    assert!(h.plain(Range::Full).contains("first"));
    let first_attempt = h.attempt();

    // The first daemon goes away: the handle sees EXIT and the socket close.
    rig.kill("a");
    let deadline = Instant::now() + Duration::from_secs(5);
    while (!h.exited() || h.connected()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(h.exited(), "EXIT from the first daemon");
    assert!(!h.connected());

    // A replacement under the same id.
    rig.run("a", "printf 'second\\n'; exec sleep 60");
    h.reconnect().expect("reconnect");
    assert!(h.attempt() > first_attempt);
    assert!(!h.exited(), "the old EXIT does not belong to the new attempt");
    assert!(h.wait_ready(Duration::from_secs(5)), "replacement SCREEN");
    let text = h.plain(Range::Full);
    assert!(text.contains("second"), "{text:?}");
    assert!(!text.contains("first"), "old screen must be gone: {text:?}");
    h.kill();
    rig.kill("a");
}

#[test]
fn attach_to_missing_session_is_an_error() {
    let root = std::env::temp_dir();
    let r = TerminalHandle::attach(
        SessionRef {
            root,
            id: "definitely-not-a-session".into(),
        },
        AttachOptions::default(),
    );
    assert!(r.is_err());
}
