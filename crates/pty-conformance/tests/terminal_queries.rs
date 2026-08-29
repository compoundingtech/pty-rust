//! Port of tests/terminal-queries.test.ts, the half that is observable over
//! the socket: the daemon answers terminal queries (DA1, DA2, DSR, OSC 10/11
//! colour queries) on the child's behalf, and strips the queries themselves
//! from the bytes it forwards to attached clients.
//!
//! The daemon writes each answer into the pty; the child is `cat`, so the
//! answer comes back out as output (through tty echo and/or cat) and reaches
//! the attached client as DATA.
//!
//! Left out: the `stripTerminalQueries` pure-function cases (lines 20-89) —
//! those are a pty-core unit test. Two of them (the embedded query at :70
//! and the OSC 10 *set* at :85) are re-expressed here as end-to-end checks
//! on what an attached client receives.

use pty_conformance::*;
use pty_core::protocol::MessageType;
use std::time::Instant;

/// Text of every SCREEN and DATA payload received so far, in order.
fn seen_text(packets: &[pty_core::protocol::Packet]) -> String {
    let mut s = String::new();
    for p in packets {
        if matches!(p.type_, MessageType::Screen | MessageType::Data) {
            s.push_str(&String::from_utf8_lossy(&p.payload));
        }
    }
    s
}

/// Attach to `id` and read until the received text satisfies `pred`.
fn attach_until(rig: &Rig, id: &str, pred: impl Fn(&str) -> bool) -> String {
    let mut conn = rig.connect(id);
    conn.attach(24, 80);
    let mut packets = Vec::new();
    let start = Instant::now();
    let timeout = deadline();
    loop {
        let text = seen_text(&packets);
        if pred(&text) {
            return text;
        }
        let remaining = timeout.saturating_sub(start.elapsed());
        match conn.next_packet(remaining) {
            Some(p) => packets.push(p),
            None => panic!("timed out waiting for the query response; saw: {text:?}"),
        }
    }
}

fn query_session(rig: &Rig, id: &str, printf_arg: &str) {
    let script = format!("printf '{printf_arg}'; cat");
    rig.daemon(id, &["sh", "-c", &script], DaemonOpts::no_display_name());
}

/// node: tests/terminal-queries.test.ts:93
#[test]
fn responds_to_da1() {
    let rig = Rig::new();
    query_session(&rig, "da1", "\\033[c");
    attach_until(&rig, "da1", |t| t.contains("62;22"));
}

/// node: tests/terminal-queries.test.ts:105
#[test]
fn responds_to_osc11_background_query() {
    let rig = Rig::new();
    query_session(&rig, "bg", "\\033]11;?\\033\\\\");
    attach_until(&rig, "bg", |t| t.contains("0000/0000/0000"));
}

/// node: tests/terminal-queries.test.ts:117
#[test]
fn responds_to_osc10_foreground_query() {
    let rig = Rig::new();
    query_session(&rig, "fg", "\\033]10;?\\033\\\\");
    attach_until(&rig, "fg", |t| t.contains("c0c0/c0c0/c0c0"));
}

/// node: tests/terminal-queries.test.ts:127
#[test]
fn responds_to_dsr_cursor_position() {
    let rig = Rig::new();
    query_session(&rig, "dsr", "\\033[6n");
    let text = attach_until(&rig, "dsr", |t| regex::Regex::new(r"\d+;\d+R").unwrap().is_match(t));
    expect_regex(&text, r"\d+;\d+R");
}

/// node: tests/terminal-queries.test.ts:140
#[test]
fn responds_to_da2() {
    let rig = Rig::new();
    query_session(&rig, "da2", "\\033[>c");
    attach_until(&rig, "da2", |t| t.contains("382"));
}

/// node: tests/terminal-queries.test.ts:70
#[test]
fn query_embedded_in_output_is_stripped_for_clients() {
    let rig = Rig::new();
    // Delay so the client is attached before the chunk is produced and sees
    // it as live DATA rather than only in the SCREEN replay.
    rig.daemon(
        "emb",
        &["sh", "-c", "sleep 0.3; printf 'before\\033]11;?\\007after\\n'; cat"],
        DaemonOpts::no_display_name(),
    );
    let text = attach_until(&rig, "emb", |t| t.contains("beforeafter"));
    expect_not_contains(&text, "]11;?");
}

/// node: tests/terminal-queries.test.ts:85
#[test]
fn osc10_set_is_not_stripped() {
    let rig = Rig::new();
    rig.daemon(
        "set",
        &["sh", "-c", "sleep 0.3; printf '\\033]10;rgb:ffff/0000/0000\\007SET-DONE\\n'; cat"],
        DaemonOpts::no_display_name(),
    );
    let text = attach_until(&rig, "set", |t| t.contains("SET-DONE"));
    expect_contains(&text, "\x1b]10;rgb:ffff/0000/0000\x07");
}

/// The same answers are stripped from DATA: after the child echoes them the
/// client sees the response text but never the raw query bytes.
/// node: tests/terminal-queries.test.ts:74
#[test]
fn multiple_queries_in_one_chunk_are_all_stripped() {
    let rig = Rig::new();
    rig.daemon(
        "multi",
        &["sh", "-c", "sleep 0.3; printf 'A\\033]10;?\\007\\033]11;?\\007\\033[cB\\n'; cat"],
        DaemonOpts::no_display_name(),
    );
    let text = attach_until(&rig, "multi", |t| t.contains("AB"));
    expect_not_contains(&text, "]10;?");
    expect_not_contains(&text, "]11;?");
    expect_not_contains(&text, "\x1b[c");
}
