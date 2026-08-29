//! Port of the pty project's `tests/terminal-queries.test.ts`, second half:
//! terminal query *responses* — proof that libghostty answers DA1/DSR/DA2
//! device queries and that the harness flushes those responses back to the
//! PTY. A program emits the query on stdout; libghostty generates the reply
//! (verified: DA1→`ESC[?62;22c`, DSR→`ESC[1;1R`, DA2→`ESC[>1;0;0c`); the
//! harness writes it to the PTY, where the tty's cooked-mode echo renders it
//! as caret notation (`^[[?62;22c`) — the same round-trip the TS suite uses.
//!
//! The pure `strip_terminal_queries` half lives in
//! `pty-core/tests/terminal_queries.rs`.

use pty_testkit::{Session, SpawnOptions};

fn opts() -> SpawnOptions {
    SpawnOptions {
        rows: Some(24),
        cols: Some(80),
        ..Default::default()
    }
}

#[test]
fn responds_to_da1() {
    // Program writes the DA1 query to stdout; libghostty replies ESC[?62;22c,
    // the harness injects it, and the tty echo renders "?62;22c".
    let mut s = Session::spawn("sh", &["-c", "printf '\\033[c'; exec cat"], opts()).unwrap();
    let ss = s.wait_for_text("62;22", 5000).expect("DA1 response");
    assert!(ss.text.contains("62;22"), "screen:\n{}", ss.text);
    s.close();
}

#[test]
fn responds_to_dsr_cursor_position() {
    // DSR (ESC[6n) → ESC[<row>;<col>R. With nothing printed, the cursor is at
    // 1;1, so the echoed response contains "1;1R" (and always matches \d+;\d+R).
    let mut s = Session::spawn("sh", &["-c", "printf '\\033[6n'; exec cat"], opts()).unwrap();
    let ss = s.wait_for_text(";1R", 5000).expect("DSR response");
    let has_cpr = ss.lines.iter().any(|l| {
        // crude \d+;\d+R matcher
        if let Some(rpos) = l.find('R') {
            let before = &l[..rpos];
            if let Some(semi) = before.rfind(';') {
                let (a, b) = (&before[..semi], &before[semi + 1..]);
                return !a.is_empty()
                    && a.chars().rev().take_while(|c| c.is_ascii_digit()).count() > 0
                    && !b.is_empty()
                    && b.chars().all(|c| c.is_ascii_digit());
            }
        }
        false
    });
    assert!(has_cpr, "expected a \\d+;\\d+R cursor report:\n{}", ss.text);
    s.close();
}

#[test]
fn responds_to_da2() {
    // DA2 (ESC[>c) → ESC[>1;0;0c under libghostty.
    let mut s = Session::spawn("sh", &["-c", "printf '\\033[>c'; exec cat"], opts()).unwrap();
    let ss = s.wait_for_text(">1;0;0", 5000).expect("DA2 response");
    assert!(ss.text.contains(">1;0;0"), "screen:\n{}", ss.text);
    s.close();
}
