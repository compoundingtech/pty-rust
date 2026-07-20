//! Port of the pty project's `tests/terminal-queries.test.ts`.
//!
//! Two halves:
//!  1. `strip_terminal_queries` — pure port of the `stripTerminalQueries` util.
//!  2. Terminal query *responses* — proof that libghostty answers DA1/DSR/DA2
//!     device queries and that the harness flushes those responses back to the
//!     PTY. A program emits the query on stdout; libghostty generates the reply
//!     (verified: DA1→`ESC[?62;22c`, DSR→`ESC[1;1R`, DA2→`ESC[>1;0;0c`); the
//!     harness writes it to the PTY, where the tty's cooked-mode echo renders it
//!     as caret notation (`^[[?62;22c`) — the same round-trip the TS suite uses.

use pty_testkit::queries::strip_terminal_queries;
use pty_testkit::{Session, SpawnOptions};

// ── strip_terminal_queries (pure) ──

#[test]
fn strips_osc_10_query_bel() {
    assert_eq!(strip_terminal_queries("\x1b]10;?\x07"), "");
}
#[test]
fn strips_osc_10_query_st() {
    assert_eq!(strip_terminal_queries("\x1b]10;?\x1b\\"), "");
}
#[test]
fn strips_osc_11_query_bel() {
    assert_eq!(strip_terminal_queries("\x1b]11;?\x07"), "");
}
#[test]
fn strips_osc_11_query_st() {
    assert_eq!(strip_terminal_queries("\x1b]11;?\x1b\\"), "");
}
#[test]
fn strips_osc_4_palette_query_bel() {
    assert_eq!(strip_terminal_queries("\x1b]4;7;?\x07"), "");
    assert_eq!(strip_terminal_queries("\x1b]4;255;?\x07"), "");
}
#[test]
fn strips_osc_4_palette_query_st() {
    assert_eq!(strip_terminal_queries("\x1b]4;0;?\x1b\\"), "");
}
#[test]
fn strips_da1_query() {
    assert_eq!(strip_terminal_queries("\x1b[c"), "");
}
#[test]
fn strips_da2_query() {
    assert_eq!(strip_terminal_queries("\x1b[>c"), "");
}
#[test]
fn strips_dsr_cursor_position_query() {
    assert_eq!(strip_terminal_queries("\x1b[6n"), "");
}
#[test]
fn strips_xtversion_query() {
    assert_eq!(strip_terminal_queries("\x1b[>0q"), "");
}
#[test]
fn preserves_normal_text() {
    assert_eq!(strip_terminal_queries("hello world"), "hello world");
}
#[test]
fn preserves_normal_ansi_sequences() {
    let ansi = "\x1b[1;31mred bold\x1b[0m";
    assert_eq!(strip_terminal_queries(ansi), ansi);
}
#[test]
fn strips_queries_embedded_in_normal_output() {
    assert_eq!(
        strip_terminal_queries("before\x1b]11;?\x07after"),
        "beforeafter"
    );
}
#[test]
fn strips_multiple_queries_in_one_chunk() {
    let data = "\x1b]10;?\x07\x1b]11;?\x07\x1b[c";
    assert_eq!(strip_terminal_queries(data), "");
}
#[test]
fn preserves_osc_sequences_that_are_not_queries() {
    let title = "\x1b]0;my title\x07";
    assert_eq!(strip_terminal_queries(title), title);
}
#[test]
fn does_not_strip_osc_set_commands() {
    let set = "\x1b]10;rgb:ffff/0000/0000\x07";
    assert_eq!(strip_terminal_queries(set), set);
}

// ── query responses (libghostty round-trip) ──

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
