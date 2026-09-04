//! `TerminalHandle`: spawning, attaching to a real daemon, attach identity
//! across a replacement under the same id, and late-event rejection.
//!
//! The attach tests need the `pty` binary from this workspace: `PTY_TEST_BIN`
//! if set, else `target/<profile>/pty` (built on demand).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
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
        // A counter, not a clock: `Instant::now().elapsed()` is however long
        // those two calls took, which is nanoseconds and often the same
        // number twice. Every rig in this process was getting the same
        // directory, so tests running in parallel shared a registry and
        // fought over the session id they all use.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "pty-handle-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
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

    // The first daemon goes away. On an external kill the daemon destroys
    // its client sockets before the child dies (server.ts:1364-1372), so
    // the handle sees the socket close, not an EXIT.
    rig.kill("a");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.connected() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
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

/// The case a Node daemon cannot serve: the child draws a kitty image, and a
/// client attaches only afterwards. The image was in `DATA` that this client
/// never saw, so everything it knows comes from the daemon's `SCREEN` — which
/// carries the image because the session's own terminal holds it
/// (docs/decisions/0012-kitty-graphics-replay.md).
///
/// The bytes are OMP's (`packages/tui/src/terminal-capabilities.ts`
/// `encodeKittyTransmit`, `packages/tui/src/kitty-graphics.ts`
/// `encodeKittyVirtualPlacement` / `encodeKittyPlaceholderGrid`): a `f=100`
/// PNG transmission, a virtual placement, and a placeholder cell carrying the
/// image id in its foreground colour and the placement id in its underline
/// colour.
#[test]
fn a_late_attach_gets_the_image_the_child_drew_before_it_connected() {
    let Some(rig) = Rig::new() else {
        eprintln!("skipping: no pty binary");
        return;
    };
    // A 1x1 red PNG (the kitty protocol's own example image), image id 4242,
    // placement id 7, one placeholder cell at image row 0, column 0.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
    let script = format!(
        "printf 'drawn\\n'; \
         printf '\\033_Ga=t,f=100,q=2,i=4242;{png}\\033\\\\'; \
         printf '\\033_Ga=p,U=1,q=2,i=4242,p=7,c=1,r=1\\033\\\\'; \
         printf '\\033[38;2;0;16;146m\\033[58:2::0:0:7m\u{10eeee}\u{305}\u{305}\\033[39;59m'; \
         exec cat"
    );
    rig.run("g", &script);
    // Let the child finish drawing before anyone attaches: this client must
    // learn the image from the replay, not from live DATA.
    std::thread::sleep(Duration::from_millis(400));

    let h = TerminalHandle::attach(
        rig.session("g"),
        AttachOptions {
            graphics: Some(pty_terminal::GraphicsOptions::DEFAULT),
            ..Default::default()
        },
    )
    .expect("attach");
    assert!(h.wait_ready(Duration::from_secs(5)), "first SCREEN");
    assert!(h.plain(Range::Full).contains("drawn"));

    let deadline = Instant::now() + Duration::from_secs(5);
    let state = loop {
        let state = h.graphics(0);
        if !state.placements.is_empty() {
            break state;
        }
        assert!(
            Instant::now() < deadline,
            "the replay carried no placement; screen:\n{}",
            h.plain(Range::Full)
        );
        h.wait_rev(h.rev(), Duration::from_millis(100));
    };

    let image = state.image(4242).expect("the image came with the replay");
    assert_eq!((image.width, image.height), (1, 1));
    assert_eq!(
        h.image_bytes(4242).map(|b| b.data),
        Some(vec![255, 0, 0, 255]),
        "the pixels themselves, decoded from the PNG"
    );
    let p = &state.placements[0];
    assert_eq!((p.image_id, p.placement_id), (4242, 7));
    assert!(p.is_virtual);
    assert!(
        matches!(p.position, pty_terminal::PlacementPosition::Placeholder(_)),
        "located by its placeholder cell, at {:?}",
        p.position
    );

    // A reconnect resets the terminal and replays a fresh SCREEN. The image
    // has to come back with it: the reset must not take the storage away, or
    // the replay's own transmission would be rejected.
    let position = p.position;
    h.reconnect().expect("reconnect");
    assert!(h.wait_ready(Duration::from_secs(5)), "SCREEN after reconnect");
    let deadline = Instant::now() + Duration::from_secs(5);
    let after = loop {
        let state = h.graphics(0);
        if !state.placements.is_empty() {
            break state;
        }
        assert!(Instant::now() < deadline, "reconnect lost the image");
        h.wait_rev(h.rev(), Duration::from_millis(100));
    };
    assert_eq!(
        h.image_bytes(4242).map(|b| b.data),
        Some(vec![255, 0, 0, 255])
    );
    assert_eq!(after.placements[0].placement_id, 7);
    assert_eq!(after.placements[0].position, position, "same cell");

    h.kill();
    rig.kill("g");
}

/// A daemon-side detach is a state a consumer has to be able to enter, so it
/// is an event, not something to poll `connected()` for. A reconnect is the
/// symmetric one: `Connected` means the new socket is up, and the attempt's
/// first SCREEN follows.
#[test]
fn a_lost_socket_and_a_reconnect_are_both_announced() {
    let Some(rig) = Rig::new() else {
        eprintln!("skipping: no pty binary");
        return;
    };
    rig.run("d", "printf 'first\\n'; exec sleep 60");
    let h = TerminalHandle::attach(rig.session("d"), AttachOptions::default()).expect("attach");
    assert!(h.wait_ready(Duration::from_secs(5)));
    let events = h.subscribe();

    rig.kill("d");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.connected() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!h.connected());
    let mut saw_disconnected = false;
    while let Ok(ev) = events.try_recv() {
        if ev == HandleEvent::Disconnected {
            saw_disconnected = true;
        }
    }
    assert!(saw_disconnected, "the lost socket was announced");

    rig.run("d", "printf 'second\\n'; exec sleep 60");
    h.reconnect().expect("reconnect");
    assert!(h.wait_ready(Duration::from_secs(5)));
    let mut saw_connected = false;
    while let Ok(ev) = events.try_recv() {
        if ev == HandleEvent::Connected(h.attempt()) {
            saw_connected = true;
        }
    }
    assert!(saw_connected, "the new socket was announced");
    h.kill();
    rig.kill("d");
}

/// Cell metrics live on the client's host — its font — but the session's
/// terminal is what answers geometry for a placement that left `c=`/`r=`
/// implicit, including in the replay every other client gets. So the client
/// declares them on ATTACH and RESIZE, and the daemon adopts them.
#[test]
fn a_client_declares_its_cell_size_and_the_session_geometry_follows() {
    let Some(rig) = Rig::new() else {
        eprintln!("skipping: no pty binary");
        return;
    };
    // A 16x16 RGBA image (f=32, 1024 pixel bytes) placed with no c=/r=, so
    // its cell extent is purely derived from the cell size.
    let px = format!("{}==", "A".repeat(1366));
    let script = format!(
        "printf 'drawn\\n'; \
         printf '\\033_Ga=t,q=2,i=77,f=32,s=16,v=16;{px}\\033\\\\'; \
         printf '\\033_Ga=p,q=2,i=77,p=1\\033\\\\'; \
         exec cat"
    );
    rig.run("c", &script);
    std::thread::sleep(Duration::from_millis(400));

    let graphics = pty_terminal::GraphicsOptions {
        cell: pty_terminal::CellSize {
            width: 16,
            height: 16,
        },
        ..pty_terminal::GraphicsOptions::DEFAULT
    };
    let h = TerminalHandle::attach(
        rig.session("c"),
        AttachOptions {
            graphics: Some(graphics),
            ..Default::default()
        },
    )
    .expect("attach");
    assert!(h.wait_ready(Duration::from_secs(5)));

    // 16x16 pixels at a declared 16x16 cell is 1x1 cells, on both sides: the
    // client's own terminal and the daemon's, which was told on ATTACH.
    let deadline = Instant::now() + Duration::from_secs(5);
    let state = loop {
        let state = h.graphics(0);
        if !state.placements.is_empty() {
            break state;
        }
        assert!(
            Instant::now() < deadline,
            "no placement; screen:\n{}",
            h.plain(Range::Full)
        );
        h.wait_rev(h.rev(), Duration::from_millis(100));
    };
    assert!(state.cell_declared, "the client declared its cell size");
    assert_eq!(state.placements[0].cell_size, (1, 1));
    assert_eq!(state.placements[0].requested_cells, (0, 0));

    // The daemon's own answer, via a PEEK-based read-only attach that never
    // declares anything: the replay it gets carries the image, and the
    // session's terminal is what resolved the geometry.
    let (code, out, err) = rig.pty(&["peek", "c"]);
    assert_eq!(code, 0, "peek failed: {out}{err}");
    assert!(
        out.contains("i=77"),
        "the session's replay carries the image: {out:?}"
    );

    // A later declaration travels on RESIZE and moves the derived extent.
    h.set_cell_size(8, 16);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = h.graphics(0);
        if state.placements.first().map(|p| p.cell_size) == Some((2, 1)) {
            assert_eq!(state.cell, pty_terminal::CellSize { width: 8, height: 16 });
            break;
        }
        assert!(Instant::now() < deadline, "the new cell size never took");
        h.wait_rev(h.rev(), Duration::from_millis(100));
    }

    h.kill();
    rig.kill("c");
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
