//! Parity tests: pin pty-rust behavior to the node pty reference for the
//! divergences the nesting verification found. Node is the reference; these
//! assert the values node emits so the two projects share one behavioral spec.

use pty_testkit::client::{resolve_seq_delay_ms, DEFAULT_SEQ_DELAY_MS};
use pty_testkit::{Session, SpawnOptions};

// ── #3: send --seq pacing (port of node's resolveSeqDelayMs coverage) ──

#[test]
fn seq_delay_defaults_to_300ms() {
    assert_eq!(DEFAULT_SEQ_DELAY_MS, 300);
    assert_eq!(resolve_seq_delay_ms(None), 300);
}

#[test]
fn seq_delay_zero_is_straight_stream() {
    assert_eq!(resolve_seq_delay_ms(Some(0.0)), 0);
}

#[test]
fn seq_delay_n_resolves_to_n_seconds() {
    assert_eq!(resolve_seq_delay_ms(Some(0.1)), 100);
    assert_eq!(resolve_seq_delay_ms(Some(0.5)), 500);
    assert_eq!(resolve_seq_delay_ms(Some(2.0)), 2000);
}

// ── #2: plain screenshot keeps a trailing WRITTEN space (cursor cell) ──

#[test]
fn plain_capture_keeps_trailing_written_space() {
    // A program writes "PROMPT$ " with NO newline — the trailing space is real
    // written content (like a shell PS1 "$ "). Node's plain serialization keeps
    // it (translateToString(true) trims only never-written cells); pty-rust must
    // match, so the captured line ends with the written space, not trimmed.
    let mut s = Session::spawn(
        "sh",
        &["-c", "printf 'PROMPT$ '; sleep 30"],
        SpawnOptions {
            rows: Some(24),
            cols: Some(80),
            ..Default::default()
        },
    )
    .expect("spawn");
    s.wait_for_text("PROMPT$", 5000).expect("prompt written");
    let ss = s.screenshot();
    assert!(
        ss.lines.iter().any(|l| l == "PROMPT$ "),
        "expected a line exactly 'PROMPT$ ' (trailing written space kept); lines: {:?}",
        ss.lines
    );
    // And it must NOT have been trimmed to "PROMPT$".
    assert!(
        !ss.lines.iter().any(|l| l == "PROMPT$"),
        "trailing written space was trimmed (diverges from node): {:?}",
        ss.lines
    );
    s.close();
}

#[test]
fn plain_capture_still_drops_never_written_trailing_cells() {
    // "abc\n" — the rest of the row is never written; those cells are dropped
    // (no 80-col padding), matching node.
    let mut s = Session::spawn(
        "sh",
        &["-c", "printf 'abc\\n'; sleep 30"],
        SpawnOptions {
            rows: Some(24),
            cols: Some(80),
            ..Default::default()
        },
    )
    .expect("spawn");
    s.wait_for_text("abc", 5000).expect("written");
    let ss = s.screenshot();
    assert!(
        ss.lines.iter().any(|l| l == "abc"),
        "expected exactly 'abc' (never-written trailing cells dropped): {:?}",
        ss.lines
    );
    s.close();
}
