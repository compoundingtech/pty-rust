//! Terminal query answers: the exact bytes (actor level) and the round trip
//! through a real child that echoes them (handle level).
//!
//! Port of the pty project's `tests/terminal-queries.test.ts:93-149`.

use std::time::Duration;

use pty_terminal::{Range, SpawnOptions, TerminalActor, TerminalHandle};

fn actor() -> TerminalActor {
    TerminalActor::new(24, 80, 100)
}

// ── byte-exact answers ──

/// node: src/server.ts:397-405 (DA1), tests/terminal-queries.test.ts:94-105
#[test]
fn da1_answer_is_nodes_bytes_and_stripped_from_data() {
    let mut a = actor();
    for q in [&b"\x1b[c"[..], b"\x1b[0c"] {
        let data = a.write(q);
        assert_eq!(data, b"", "DA1 must not reach DATA");
        assert_eq!(a.take_pty_replies(), b"\x1b[?62;22c");
    }
}

/// node: src/server.ts:491-498 (DA2), tests/terminal-queries.test.ts:140-149
#[test]
fn da2_answer_is_nodes_bytes() {
    let mut a = actor();
    let data = a.write(b"\x1b[>c");
    assert_eq!(data, b"");
    assert_eq!(a.take_pty_replies(), b"\x1b[>0;382;0c");
    assert_eq!(a.write(b"\x1b[>0c"), b"");
    assert_eq!(a.take_pty_replies(), b"\x1b[>0;382;0c");
}

/// node: src/server.ts:499-508 (DSR), tests/terminal-queries.test.ts:127-138
#[test]
fn dsr_reports_one_based_cursor() {
    let mut a = actor();
    assert_eq!(a.write(b"\x1b[6n"), b"");
    assert_eq!(a.take_pty_replies(), b"\x1b[1;1R");
    // Cursor at row 10, column 20 (1-based) → ESC[10;20R.
    let data = a.write(b"\x1b[10;20H\x1b[6n");
    assert_eq!(data, b"\x1b[10;20H");
    assert_eq!(a.take_pty_replies(), b"\x1b[10;20R");
}

/// node: src/server.ts:509-516 (XTVERSION)
#[test]
fn xtversion_answer_is_nodes_bytes() {
    let mut a = actor();
    assert_eq!(a.write(b"\x1b[>0q"), b"");
    assert_eq!(a.take_pty_replies(), b"\x1bP>|pty(0.8)\x1b\\");
    assert_eq!(a.write(b"\x1b[>q"), b"");
    assert_eq!(a.take_pty_replies(), b"\x1bP>|pty(0.8)\x1b\\");
}

/// node: src/server.ts:459-490 (OSC 10/11/4), tests/terminal-queries.test.ts:107-126
#[test]
fn color_queries_answer_with_st_whatever_the_query_terminator() {
    let mut a = actor();
    for (q, reply) in [
        (&b"\x1b]10;?\x1b\\"[..], &b"\x1b]10;rgb:c0c0/c0c0/c0c0\x1b\\"[..]),
        (b"\x1b]10;?\x07", b"\x1b]10;rgb:c0c0/c0c0/c0c0\x1b\\"),
        (b"\x1b]11;?\x1b\\", b"\x1b]11;rgb:0000/0000/0000\x1b\\"),
        (b"\x1b]11;?\x07", b"\x1b]11;rgb:0000/0000/0000\x1b\\"),
        (b"\x1b]4;7;?\x07", b"\x1b]4;7;rgb:0000/0000/0000\x1b\\"),
        (b"\x1b]4;255;?\x1b\\", b"\x1b]4;255;rgb:0000/0000/0000\x1b\\"),
        (b"\x1b]4;0;?\x1b\\", b"\x1b]4;0;rgb:0000/0000/0000\x1b\\"),
    ] {
        let data = a.write(q);
        assert_eq!(data, b"", "{q:?} must not reach DATA");
        assert_eq!(a.take_pty_replies(), reply, "{q:?}");
    }
    // Node answers only the first index of a multi-query and consumes it.
    assert_eq!(a.write(b"\x1b]4;1;?;2;?\x07"), b"");
    assert_eq!(a.take_pty_replies(), b"\x1b]4;1;rgb:0000/0000/0000\x1b\\");
    // A non-query OSC 10 (a set) passes through and is not answered.
    let set = b"\x1b]10;rgb:ffff/0000/0000\x07";
    assert_eq!(a.write(set), set);
    assert_eq!(a.take_pty_replies(), b"");
}

/// Replies come out in stream order even when a colour query sits between
/// two device queries in one chunk.
#[test]
fn replies_keep_stream_order() {
    let mut a = actor();
    let data = a.write(b"a\x1b[c\x1b]11;?\x07b\x1b[>c c");
    assert_eq!(data, b"ab c");
    assert_eq!(
        a.take_pty_replies(),
        b"\x1b[?62;22c\x1b]11;rgb:0000/0000/0000\x1b\\\x1b[>0;382;0c"
    );
    assert_eq!(a.plain(Range::Viewport), "ab c");
}

/// A query split across two PTY reads is still answered once and never
/// reaches DATA.
#[test]
fn split_query_is_answered_once() {
    let mut a = actor();
    let mut data = a.write(b"x\x1b]1");
    data.extend(a.write(b"1;?\x1b"));
    data.extend(a.write(b"\\y\x1b["));
    data.extend(a.write(b"cz"));
    assert_eq!(data, b"xyz");
    assert_eq!(
        a.take_pty_replies(),
        b"\x1b]11;rgb:0000/0000/0000\x1b\\\x1b[?62;22c"
    );
}

// ── through a real child that echoes the answer ──

fn spawn_echo(script: &str) -> TerminalHandle {
    TerminalHandle::spawn("sh", &["-c", script], SpawnOptions::default()).expect("spawn")
}

fn wait_text(h: &TerminalHandle, needle: &str) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let text = h.plain(Range::Full);
        if text.contains(needle) {
            return text;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {needle:?}; screen:\n{text}"
        );
        h.wait_rev(h.rev(), Duration::from_millis(200));
    }
}

/// node: tests/terminal-queries.test.ts:94-105
#[test]
fn child_sees_da1_answer() {
    let h = spawn_echo("printf '\\033[c'; exec cat");
    wait_text(&h, "62;22");
    h.kill();
}

/// node: tests/terminal-queries.test.ts:107-116
#[test]
fn child_sees_osc11_answer() {
    let h = spawn_echo("printf '\\033]11;?\\033\\\\'; exec cat");
    wait_text(&h, "0000/0000/0000");
    h.kill();
}

/// node: tests/terminal-queries.test.ts:118-126
#[test]
fn child_sees_osc10_answer() {
    let h = spawn_echo("printf '\\033]10;?\\033\\\\'; exec cat");
    wait_text(&h, "c0c0/c0c0/c0c0");
    h.kill();
}

/// node: tests/terminal-queries.test.ts:128-138
#[test]
fn child_sees_dsr_answer() {
    let h = spawn_echo("printf '\\033[6n'; exec cat");
    let text = wait_text(&h, "R");
    let has_cpr = text.lines().any(|l| {
        let Some(r) = l.find('R') else { return false };
        let before = &l[..r];
        let Some(semi) = before.rfind(';') else { return false };
        let col = &before[semi + 1..];
        let row: String = before[..semi]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        !row.is_empty() && !col.is_empty() && col.chars().all(|c| c.is_ascii_digit())
    });
    assert!(has_cpr, "expected a \\d+;\\d+R report:\n{text}");
    h.kill();
}

/// node: tests/terminal-queries.test.ts:140-149
#[test]
fn child_sees_da2_answer() {
    let h = spawn_echo("printf '\\033[>c'; exec cat");
    wait_text(&h, "382");
    h.kill();
}
