//! Port of tests/screen-replay-altscreen.test.ts: the SCREEN an ATTACH
//! receives starts at byte 0 with `ESC[?1049h` when the child is in the
//! alternate screen buffer (so the client's terminal switches buffers
//! before anything is painted), never when it is on the main buffer, and
//! the legacy `?1047h` form is tracked and normalized to `?1049h`. Node's
//! testing `Session.server` becomes `pty run -d`; the SCREEN is captured on
//! a fresh raw socket.
//!
//! Left out: nothing. (Node deliberately does not pin PEEK's prefix.)

use pty_conformance::*;
use pty_core::protocol::MessageType;
use std::time::Duration;

fn capture_attach_screen(rig: &Rig, id: &str) -> Vec<u8> {
    let mut conn = rig.connect(id);
    conn.attach(24, 80);
    conn.wait_for(MessageType::Screen, Duration::from_secs(3))
        .expect("SCREEN")
        .payload
}

fn start(rig: &Rig, id: &str, printf: &str) {
    let script = format!("printf '{printf}'; sleep 60");
    rig.daemon(id, &["sh", "-c", &script], DaemonOpts::no_display_name());
    std::thread::sleep(Duration::from_millis(300));
}

const ALT_ENTER: &[u8] = b"\x1b[?1049h";

/// node: tests/screen-replay-altscreen.test.ts:62
#[test]
fn attach_prefixes_1049h_at_position_0_when_child_is_in_alt_screen() {
    let rig = Rig::new();
    start(&rig, "alt1", "\\033[?1049h\\033[Halt-marker");
    let screen = capture_attach_screen(&rig, "alt1");
    assert!(screen.starts_with(ALT_ENTER), "{:?}", String::from_utf8_lossy(&screen));
    assert!(
        screen.windows(b"alt-marker".len()).any(|w| w == b"alt-marker"),
        "{:?}",
        String::from_utf8_lossy(&screen)
    );
}

/// node: tests/screen-replay-altscreen.test.ts:85
#[test]
fn attach_does_not_prefix_1049h_on_the_main_screen() {
    let rig = Rig::new();
    start(&rig, "alt2", "main-only");
    let screen = capture_attach_screen(&rig, "alt2");
    assert!(!screen.starts_with(ALT_ENTER), "{:?}", String::from_utf8_lossy(&screen));
}

/// node: tests/screen-replay-altscreen.test.ts:102
#[test]
fn attach_stops_prefixing_1049h_after_child_leaves_alt_screen() {
    let rig = Rig::new();
    start(&rig, "alt3", "\\033[?1049h\\033[?1049lmain-again");
    let screen = capture_attach_screen(&rig, "alt3");
    assert!(!screen.starts_with(ALT_ENTER), "{:?}", String::from_utf8_lossy(&screen));
}

/// node: tests/screen-replay-altscreen.test.ts:127
#[test]
fn tracks_1047_legacy_variant_as_alt_screen_too() {
    let rig = Rig::new();
    start(&rig, "alt4", "\\033[?1047h\\033[Halt-1047");
    let screen = capture_attach_screen(&rig, "alt4");
    assert!(screen.starts_with(ALT_ENTER), "{:?}", String::from_utf8_lossy(&screen));
}
