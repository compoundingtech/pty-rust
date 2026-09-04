//! Core libghostty-backed harness tests: spawn real processes in a PTY, feed
//! their output through libghostty, and assert on screenshots.
//!
//! These are the Rust analogue of the pty project's `screenshot.test.ts` /
//! `shells.test.ts` — but where the TS suite drives `@xterm/headless`, here the
//! terminal emulator is **libghostty**.

use std::fs;
use pty_testkit::{Session, SpawnOptions};

fn opts(rows: u16, cols: u16) -> SpawnOptions {
    SpawnOptions {
        rows: Some(rows),
        cols: Some(cols),
        ..Default::default()
    }
}

/// Hold a process open after it emits output so we can screenshot it.
fn keep_alive(script: &str) -> String {
    format!("{script}; sleep 30")
}

#[test]
fn echo_output_appears_in_screenshot() {
    let mut s = Session::spawn("sh", &["-c", &keep_alive("echo hello-world")], opts(24, 80))
        .expect("spawn");
    // wait_for_text already asserts the content appears (it errors on timeout).
    s.wait_for_text("hello-world", 5000).expect("wait");
    s.close();
}

#[test]
fn ls_shows_filenames() {
    let dir = tempdir("pty-ls");
    fs::write(dir.join("alpha.txt"), "").unwrap();
    fs::write(dir.join("beta.log"), "").unwrap();
    fs::create_dir(dir.join("gamma")).unwrap();

    let script = format!("ls {}", dir.display());
    let mut s = Session::spawn("sh", &["-c", &keep_alive(&script)], opts(24, 80)).expect("spawn");
    // wait_for_text covers "alpha.txt"; assert the other two entries too.
    let ss = s.wait_for_text("alpha.txt", 5000).expect("wait");
    assert!(ss.text.contains("beta.log"), "screen:\n{}", ss.text);
    assert!(ss.text.contains("gamma"), "screen:\n{}", ss.text);
    s.close();
}

#[test]
fn ls_la_shows_permissions_and_structure() {
    let dir = tempdir("pty-lsla");
    fs::write(dir.join("readme.md"), "hello").unwrap();
    fs::create_dir(dir.join("src")).unwrap();

    let script = format!("ls -la {}", dir.display());
    let mut s = Session::spawn("sh", &["-c", &keep_alive(&script)], opts(24, 100)).expect("spawn");
    let ss = s.wait_for_text("readme.md", 5000).expect("wait");
    assert!(ss.text.contains("readme.md"));
    assert!(ss.text.contains("src"));
    // permission bits like `-rw-r--r--` / `drwxr-xr-x`
    assert!(
        ss.lines.iter().any(|l| {
            let bytes = l.as_bytes();
            bytes.windows(10).any(|w| {
                w.iter()
                    .all(|&c| matches!(c, b'd' | b'r' | b'w' | b'x' | b'-' | b'l' | b's' | b't'))
                    && w.iter().filter(|&&c| c == b'r' || c == b'-').count() >= 3
            })
        }),
        "expected a permission-bits column in:\n{}",
        ss.text
    );
    assert!(ss.text.contains("total "), "expected `total N` header");
    s.close();
}

#[test]
fn preserves_ansi_colors() {
    // Red "RED", reset. Plain text keeps the word; the VT serialization keeps
    // an SGR color introducer that plain text does not.
    let script = "printf '\\033[31mRED\\033[0m done\\n'";
    let mut s = Session::spawn("sh", &["-c", &keep_alive(script)], opts(24, 80)).expect("spawn");
    let ss = s.wait_for_text("RED", 5000).expect("wait");
    assert!(ss.text.contains("RED"));
    // The ANSI capture must carry escape sequences the plain text lacks.
    assert!(ss.ansi.contains('\u{1b}'), "ansi had no escapes:\n{:?}", ss.ansi);
    assert!(!ss.text.contains('\u{1b}'), "plain text leaked escapes");
    // A red foreground shows up as SGR 31 or its 256-color form 38;5;1.
    assert!(
        ss.ansi.contains("31") || ss.ansi.contains("38;5;1"),
        "expected a red SGR in ansi:\n{:?}",
        ss.ansi
    );
    s.close();
}

#[test]
fn cursor_positioning_places_text_at_row_col() {
    // Move cursor to row 3, col 5 then write XY. (CUP is 1-based.)
    let script = "printf '\\033[3;5HXY'";
    let mut s = Session::spawn("sh", &["-c", &keep_alive(script)], opts(24, 80)).expect("spawn");
    let ss = s.wait_for_text("XY", 5000).expect("wait");
    // Row 3 is index 2; XY should sit at column 5 (four leading spaces).
    let row = &ss.lines[2];
    assert!(row.contains("XY"), "row3 was {row:?}");
    assert_eq!(row.find("XY"), Some(4), "XY not at column 5: {row:?}");
    s.close();
}

#[test]
fn wide_cjk_characters_render() {
    let script = "printf '你好 world\\n'";
    let mut s = Session::spawn("sh", &["-c", &keep_alive(script)], opts(24, 80)).expect("spawn");
    let ss = s.wait_for_text("你好", 5000).expect("wait");
    assert!(ss.text.contains("你好 world"), "screen:\n{}", ss.text);
    s.close();
}

#[test]
fn clear_screen_removes_prior_content() {
    // Print BEFORE, wait, then clear screen + home. The text must disappear.
    let script = "printf 'BEFORE\\n'; sleep 0.3; printf '\\033[2J\\033[H'";
    let mut s = Session::spawn("sh", &["-c", &keep_alive(script)], opts(24, 80)).expect("spawn");
    s.wait_for_text("BEFORE", 5000).expect("appear");
    s.wait_for_absent("BEFORE", 5000).expect("clear");
    s.close();
}

#[test]
fn bash_accepts_input_and_echoes() {
    let mut s = Session::spawn("bash", &["--norc", "--noprofile"], opts(24, 80)).expect("spawn");
    s.wait_for_text("$", 8000).expect("prompt");
    s.type_str("echo hello-bash\r");
    s.wait_for_text("hello-bash", 8000).expect("output");
    let _ = s.press("ctrl+d");
    s.close();
}

#[test]
fn ctrl_c_interrupts_a_running_program() {
    // `sleep 60` then echo DONE. Ctrl-C interrupts the sleep; with `set -e`
    // off, sh continues, so DONE prints promptly instead of after 60s.
    let mut s = Session::spawn("bash", &["--norc", "--noprofile"], opts(24, 80)).expect("spawn");
    s.wait_for_text("$", 8000).expect("prompt");
    s.type_str("sleep 60; echo DONE\r");
    // Give the sleep a moment to start, then interrupt.
    std::thread::sleep(std::time::Duration::from_millis(300));
    s.press("ctrl+c").expect("ctrl+c");
    s.wait_for_text("DONE", 8000).expect("interrupted");
    let _ = s.press("ctrl+d");
    s.close();
}

#[test]
fn resize_propagates_to_the_pty() {
    let mut s = Session::spawn("bash", &["--norc", "--noprofile"], opts(24, 80)).expect("spawn");
    s.wait_for_text("$", 8000).expect("prompt");
    s.resize(40, 100);
    assert_eq!(s.rows(), 40);
    assert_eq!(s.cols(), 100);
    // The child sees the new size via SIGWINCH; `stty size` prints "rows cols".
    s.type_str("stty size\r");
    s.wait_for_text("40 100", 8000).expect("stty size");
    let _ = s.press("ctrl+d");
    s.close();
}

#[test]
fn osc_sets_window_title() {
    // OSC 0 sets both icon name and window title; libghostty tracks it out of
    // band from the grid. BEL-terminated form.
    let script = "printf '\\033]0;My Terminal Title\\007'; printf 'body\\n'";
    let mut s = Session::spawn("sh", &["-c", &keep_alive(script)], opts(24, 80)).expect("spawn");
    s.wait_for_text("body", 5000).expect("wait");
    assert_eq!(s.title(), "My Terminal Title");
    s.close();
}

// ── helpers ──

fn tempdir(prefix: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir();
    // Unique-ish without pulling in a rng crate: pid + a monotonic counter.
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = base.join(format!("{prefix}-{}-{}", std::process::id(), n));
    fs::create_dir_all(&dir).unwrap();
    dir
}
