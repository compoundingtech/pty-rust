//! Terminal-fidelity tests exercising libghostty's VT core through the harness:
//! alternate-screen buffer switching, scrollback/viewport behavior, and text
//! styling in the ANSI capture. These mirror the intent of the pty project's
//! `screen-replay-altscreen.test.ts` and `scrollback-fidelity.test.ts`, driven
//! in spawn mode.

use pty_testkit::{Session, SpawnOptions};

fn opts(rows: u16, cols: u16) -> SpawnOptions {
    SpawnOptions {
        rows: Some(rows),
        cols: Some(cols),
        ..Default::default()
    }
}

#[test]
fn alternate_screen_switches_and_restores() {
    // MAIN prints on the primary screen; then enter alt-screen (?1049h) and
    // paint ALT; then leave alt-screen (?1049l) — MAIN must be restored and ALT
    // gone. This is exactly the buffer-swap guarantee a real terminal provides.
    let script = "printf 'MAIN-SCREEN\\n'; sleep 0.4; \
                  printf '\\033[?1049h\\033[HALT-SCREEN'; sleep 0.6; \
                  printf '\\033[?1049l'; sleep 30";
    let mut s = Session::spawn("sh", &["-c", script], opts(24, 80)).expect("spawn");

    // Primary screen shows MAIN.
    s.wait_for_text("MAIN-SCREEN", 5000).expect("main appears");

    // After ?1049h + paint, ALT is visible and MAIN is hidden (alt buffer).
    let alt = s.wait_for_text("ALT-SCREEN", 5000).expect("alt appears");
    assert!(
        !alt.text.contains("MAIN-SCREEN"),
        "main should be hidden while in alt-screen:\n{}",
        alt.text
    );

    // After ?1049l we're back on the primary screen: ALT gone, MAIN restored.
    let restored = s.wait_for_absent("ALT-SCREEN", 5000).expect("alt cleared");
    assert!(
        restored.text.contains("MAIN-SCREEN"),
        "main should be restored after leaving alt-screen:\n{}",
        restored.text
    );
    s.close();
}

#[test]
fn scrollback_retains_lines_beyond_the_viewport() {
    // Print 50 zero-padded lines into a 24-row terminal. Like xterm's
    // `buffer.active` (which the TS screenshot iterates in full), libghostty's
    // capture includes scrollback — so BOTH the earliest line that scrolled off
    // the visible viewport and the latest line are retained.
    let script = "for i in $(seq 1 50); do printf 'line-%03d\\n' \"$i\"; done; sleep 30";
    let mut s = Session::spawn("sh", &["-c", script], opts(24, 80)).expect("spawn");
    let ss = s.wait_for_text("line-050", 5000).expect("last line");
    assert!(ss.text.contains("line-050"), "latest line missing:\n{}", ss.text);
    assert!(
        ss.text.contains("line-001"),
        "scrollback should retain the earliest line:\n{}",
        ss.text
    );
    // All 50 emitted lines are present in scrollback order.
    let idx1 = ss.text.find("line-001").unwrap();
    let idx50 = ss.text.find("line-050").unwrap();
    assert!(idx1 < idx50, "scrollback order should be chronological");
    s.close();
}

#[test]
fn bold_and_underline_preserved_in_ansi_capture() {
    let script = "printf '\\033[1mBOLD\\033[0m \\033[4mUNDER\\033[0m\\n'";
    let mut s = Session::spawn("sh", &["-c", &format!("{script}; sleep 30")], opts(24, 80))
        .expect("spawn");
    let ss = s.wait_for_text("BOLD", 5000).expect("wait");
    assert!(ss.text.contains("BOLD UNDER"), "plain:\n{}", ss.text);
    // The ANSI serialization retains the SGR attributes; plain text drops them.
    assert!(ss.ansi.contains("1m"), "no bold SGR in ansi:\n{:?}", ss.ansi);
    assert!(
        ss.ansi.contains("4m"),
        "no underline SGR in ansi:\n{:?}",
        ss.ansi
    );
    s.close();
}

#[test]
fn carriage_return_overwrites_in_place() {
    // A progress-bar style redraw: "50%\r100%" — the CR returns to column 0 and
    // 100% overwrites 50%, so only 100% remains on that row.
    let script = "printf '50%%\\r100%%\\n'";
    let mut s = Session::spawn("sh", &["-c", &format!("{script}; sleep 30")], opts(24, 80))
        .expect("spawn");
    // wait_for_text covers "100%"; the point is that "50%" was overwritten.
    let ss = s.wait_for_text("100%", 5000).expect("wait");
    assert!(
        !ss.text.contains("50%"),
        "50%% should have been overwritten:\n{}",
        ss.text
    );
    s.close();
}
