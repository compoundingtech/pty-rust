//! Port of tests/ratatui-compat.test.ts re-expressed through the socket.
//! Node drives an in-process xterm from the testing library and inspects
//! its screenshot (`text`, `ansi`) before and after `reconnect()`. Here the
//! "before" is what a client attached from the start receives live
//! (SCREEN + DATA bytes) and the "after" is the SCREEN payload a fresh
//! ATTACH gets — the serialized replay the daemon produces. Every Node
//! `it` has a counterpart; the children are `sh` scripts printing the same
//! escape sequences (dimensions read with `stty size`, SIGWINCH via `trap`).

use pty_conformance::*;
use pty_core::protocol::MessageType;
use std::time::Duration;

/// A child that pauses (so a live attacher is in place), reads its own
/// terminal size into `$rows`/`$cols`, runs `body`, then idles.
fn child(body: &str) -> String {
    format!("sleep 0.3; set -- $(stty size); rows=$1; cols=$2; {body}; exec sleep 300")
}

/// `printf` a run of `n` spaces (shell arithmetic expression).
fn spaces(n: &str) -> String {
    format!("printf '%*s' \"$(({n}))\" ''")
}

fn start(rig: &Rig, id: &str, script: &str) {
    rig.daemon(id, &["sh", "-c", script], DaemonOpts::no_display_name());
}

/// Attach live and collect SCREEN+DATA bytes until `marker` shows up.
fn live_until(rig: &Rig, id: &str, rows: u16, cols: u16, marker: &str) -> (Conn, Vec<u8>) {
    let mut conn = rig.connect(id);
    conn.attach(rows, cols);
    let mut bytes = Vec::new();
    let start = std::time::Instant::now();
    loop {
        match conn.next_packet(Duration::from_millis(200)) {
            Some(p) if matches!(p.type_, MessageType::Screen | MessageType::Data) => {
                bytes.extend_from_slice(&p.payload);
                if contains(&bytes, marker) {
                    // Give the rest of the drawing a moment to land.
                    for p in conn.drain(Duration::from_millis(200)) {
                        if matches!(p.type_, MessageType::Screen | MessageType::Data) {
                            bytes.extend_from_slice(&p.payload);
                        }
                    }
                    return (conn, bytes);
                }
            }
            Some(_) => {}
            None => {}
        }
        assert!(
            start.elapsed() < deadline(),
            "timed out waiting for {marker:?} in the live stream; got {:?}",
            String::from_utf8_lossy(&bytes)
        );
    }
}

/// Node's `reconnect()`: drop the socket, wait 100 ms, attach afresh and
/// take the SCREEN payload.
fn reconnect(rig: &Rig, id: &str, conn: Conn, rows: u16, cols: u16) -> Vec<u8> {
    drop(conn);
    std::thread::sleep(Duration::from_millis(100));
    screen(rig, id, rows, cols)
}

fn screen(rig: &Rig, id: &str, rows: u16, cols: u16) -> Vec<u8> {
    let mut conn = rig.connect(id);
    conn.attach(rows, cols);
    conn.wait_for(MessageType::Screen, deadline()).expect("SCREEN").payload
}

fn contains(hay: &[u8], needle: &str) -> bool {
    let n = needle.as_bytes();
    !n.is_empty() && hay.windows(n.len()).any(|w| w == n)
}

#[track_caller]
fn expect_text(hay: &[u8], needle: &str) {
    assert!(contains(hay, needle), "expected {needle:?} in:\n{:?}", String::from_utf8_lossy(hay));
}

/// Node's `ansiContainsSGR`: the params appear inside some `CSI … m`.
fn contains_sgr(hay: &[u8], params: &str) -> bool {
    let s = String::from_utf8_lossy(hay);
    let re = regex::Regex::new(&format!(
        "\x1b\\[(?:[0-9;]*;)?{}(?:;[0-9;]*)?m",
        regex::escape(params)
    ))
    .unwrap();
    re.is_match(&s)
}

#[track_caller]
fn expect_sgr(hay: &[u8], params: &str) {
    assert!(contains_sgr(hay, params), "expected SGR {params} in:\n{:?}", String::from_utf8_lossy(hay));
}

#[track_caller]
fn expect_regex_bytes(hay: &[u8], re: &str) {
    expect_regex(&String::from_utf8_lossy(hay), re);
}

#[track_caller]
fn expect_not_regex_bytes(hay: &[u8], re: &str) {
    expect_not_regex(&String::from_utf8_lossy(hay), re);
}

// ── 1. ECH/CUF round-trip with background colors ──

/// node: tests/ratatui-compat.test.ts:138
#[test]
fn full_width_rgb_background_fill_survives_replay() {
    let rig = Rig::new();
    start(&rig, "bgfill", &child(&format!(
        "printf '\\033[48;2;71;76;86m'; {}; printf '\\033[0m\\n'; printf 'BG-FILL-DONE\\n'",
        spaces("cols")
    )));
    let (conn, live) = live_until(&rig, "bgfill", 24, 80, "BG-FILL-DONE");
    expect_regex_bytes(&live, "\x1b\\[48;2;71;76;86m");
    let replay = reconnect(&rig, "bgfill", conn, 24, 80);
    expect_text(&replay, "BG-FILL-DONE");
    expect_regex_bytes(&replay, "\x1b\\[48;2;71;76;86m");
}

/// node: tests/ratatui-compat.test.ts:179
#[test]
fn partial_background_fill_survives_replay() {
    let rig = Rig::new();
    start(&rig, "partbg", &child(&format!(
        "printf '\\033[48;2;0;100;200m'; {}; printf '\\033[0m'; {}; printf '\\n'; printf 'PARTIAL-BG-DONE\\n'",
        spaces("40"),
        spaces("40")
    )));
    let (conn, live) = live_until(&rig, "partbg", 24, 80, "PARTIAL-BG-DONE");
    expect_regex_bytes(&live, "\x1b\\[48;2;0;100;200m");
    let replay = reconnect(&rig, "partbg", conn, 24, 80);
    expect_text(&replay, "PARTIAL-BG-DONE");
    expect_regex_bytes(&replay, "\x1b\\[48;2;0;100;200m");
}

/// node: tests/ratatui-compat.test.ts:214
#[test]
fn ech_cuf_encoding_preserves_text_alongside_background_fill() {
    let rig = Rig::new();
    start(&rig, "textbg", &child(&format!(
        "printf '\\033[48;2;30;30;30m\\033[38;2;255;255;255mHello World'; {}; printf '\\033[0m\\n'; printf 'TEXT-BG-DONE\\n'",
        spaces("cols - 11")
    )));
    let (conn, live) = live_until(&rig, "textbg", 24, 80, "TEXT-BG-DONE");
    expect_text(&live, "Hello World");
    let replay = reconnect(&rig, "textbg", conn, 24, 80);
    expect_text(&replay, "Hello World");
    expect_text(&replay, "TEXT-BG-DONE");
    expect_sgr(&replay, "48;2;30;30;30");
    expect_sgr(&replay, "38;2;255;255;255");
}

// ── 2. Full-screen ratatui-style rendering ──

const FILL_ROWS: &str = "r=1; while [ $r -le $rows ]; do printf '\\033[%d;1H' $r; printf \"$bg\"; printf '\\033[K'; r=$((r+1)); done";

/// node: tests/ratatui-compat.test.ts:260
#[test]
fn alt_screen_with_per_row_background_erase_survives_replay() {
    let rig = Rig::new();
    let body = format!(
        "printf '\\033[?1049h'; bg='\\033[48;2;71;76;86m'; {FILL_ROWS}; \
         printf '\\033[1;1H'; printf \"$bg\"'\\033[1m Title Bar \\033[22m\\033[0m'; \
         printf '\\033[3;1H'; printf \"$bg\"' Content line here\\033[0m'; \
         printf '\\033[%d;1H' $rows; printf \"$bg\"' Status: RATATUI-SCREEN-OK\\033[0m'"
    );
    start(&rig, "ratscreen", &child(&body));
    let (conn, live) = live_until(&rig, "ratscreen", 24, 80, "RATATUI-SCREEN-OK");
    expect_text(&live, "Title Bar");
    expect_text(&live, "Content line here");
    expect_sgr(&live, "48;2;71;76;86");
    let replay = reconnect(&rig, "ratscreen", conn, 24, 80);
    expect_text(&replay, "Title Bar");
    expect_text(&replay, "Content line here");
    expect_text(&replay, "RATATUI-SCREEN-OK");
    expect_sgr(&replay, "48;2;71;76;86");
}

/// node: tests/ratatui-compat.test.ts:326
#[test]
fn cursor_addressed_drawing_with_multiple_colors_per_row() {
    let rig = Rig::new();
    let body = format!(
        "printf '\\033[?1049h\\033[H\\033[2J'; \
         printf '\\033[1;1H\\033[48;2;180;0;0m'; {}; printf '\\033[48;2;0;0;180m'; {}; printf '\\033[0m'; \
         printf '\\033[2;1H\\033[48;2;30;30;30m\\033[38;2;0;200;0mMULTI-COLOR-OK'; {}; printf '\\033[0m'",
        spaces("40"),
        spaces("40"),
        spaces("cols - 14")
    );
    start(&rig, "multicolor", &child(&body));
    let (conn, live) = live_until(&rig, "multicolor", 24, 80, "MULTI-COLOR-OK");
    expect_sgr(&live, "48;2;180;0;0");
    expect_sgr(&live, "48;2;0;0;180");
    expect_sgr(&live, "38;2;0;200;0");
    let replay = reconnect(&rig, "multicolor", conn, 24, 80);
    expect_text(&replay, "MULTI-COLOR-OK");
    expect_sgr(&replay, "48;2;180;0;0");
    expect_sgr(&replay, "48;2;0;0;180");
    expect_sgr(&replay, "38;2;0;200;0");
}

/// node: tests/ratatui-compat.test.ts:379
#[test]
fn full_screen_background_with_erase_line_keeps_background_after_replay() {
    let rig = Rig::new();
    let body = format!(
        "printf '\\033[?1049h'; bg='\\033[48;2;128;0;128m'; {FILL_ROWS}; \
         printf '\\033[1;1H\\033[48;2;128;0;128m\\033[38;2;255;255;255mFULL-BG-EL-OK\\033[0m'"
    );
    start(&rig, "fullbgel", &child(&body));
    // Node creates this session at 10×40; the first attacher sets that
    // geometry before the child reads `stty size`.
    let (conn, live) = live_until(&rig, "fullbgel", 10, 40, "FULL-BG-EL-OK");
    expect_sgr(&live, "48;2;128;0;128");
    let replay = reconnect(&rig, "fullbgel", conn, 10, 40);
    expect_text(&replay, "FULL-BG-EL-OK");
    expect_sgr(&replay, "48;2;128;0;128");
    let matches = count_regex(&String::from_utf8_lossy(&replay), "48;2;128;0;128");
    assert!(matches >= 1);
}

// ── 3. Kitty keyboard protocol stack ──

/// node: tests/ratatui-compat.test.ts:452
#[test]
fn kitty_keyboard_push_is_replayed_in_the_mode_prefix_on_reattach() {
    let rig = Rig::new();
    start(&rig, "kittykb", &child("printf '\\033[>7u'; printf 'KITTY-KB-ACTIVE\\n'"));
    let (conn, live) = live_until(&rig, "kittykb", 24, 80, "KITTY-KB-ACTIVE");
    expect_text(&live, "KITTY-KB-ACTIVE");
    let raw = screen(&rig, "kittykb", 24, 80);
    expect_regex_bytes(&raw, "\x1b\\[>7u");
    let replay = reconnect(&rig, "kittykb", conn, 24, 80);
    expect_text(&replay, "KITTY-KB-ACTIVE");
}

/// node: tests/ratatui-compat.test.ts:492
#[test]
fn multiple_kitty_push_pop_cycles_keep_the_stack_right() {
    let rig = Rig::new();
    start(&rig, "kittystack", &child("printf '\\033[>7u\\033[>3u\\033[<u'; printf 'KITTY-STACK-OK\\n'"));
    let (_conn, _live) = live_until(&rig, "kittystack", 24, 80, "KITTY-STACK-OK");
    let raw = screen(&rig, "kittystack", 24, 80);
    expect_regex_bytes(&raw, "\x1b\\[>7u");
    expect_not_regex_bytes(&raw, "\x1b\\[>3u");
}

/// node: tests/ratatui-compat.test.ts:531
#[test]
fn kitty_pop_on_empty_stack_does_not_break_the_session() {
    let rig = Rig::new();
    start(&rig, "kittypop", &child("printf '\\033[<u'; printf 'KITTY-EMPTY-POP-OK\\n'"));
    let (conn, live) = live_until(&rig, "kittypop", 24, 80, "KITTY-EMPTY-POP-OK");
    expect_text(&live, "KITTY-EMPTY-POP-OK");
    let replay = reconnect(&rig, "kittypop", conn, 24, 80);
    expect_text(&replay, "KITTY-EMPTY-POP-OK");
    expect_not_regex_bytes(&replay, "\x1b\\[>[0-9]+u");
}

/// node: tests/ratatui-compat.test.ts:566
#[test]
fn kitty_flags_combined_with_hidden_cursor_and_sgr_mouse() {
    let rig = Rig::new();
    start(&rig, "kittycombo", &child("printf '\\033[?1006h\\033[?25l\\033[>7u'; printf 'KITTY-COMBO-OK\\n'"));
    live_until(&rig, "kittycombo", 24, 80, "KITTY-COMBO-OK");
    let raw = screen(&rig, "kittycombo", 24, 80);
    expect_regex_bytes(&raw, "\x1b\\[\\?1006h");
    expect_regex_bytes(&raw, "\x1b\\[\\?25l");
    expect_regex_bytes(&raw, "\x1b\\[>7u");
}

/// node: tests/ratatui-compat.test.ts:603
#[test]
fn mouse_tracking_modes_are_replayed_on_reattach() {
    let rig = Rig::new();
    start(&rig, "mousetrack", &child("printf '\\033[?1002h\\033[?1006h'; printf 'MOUSE-TRACKING-OK\\n'"));
    live_until(&rig, "mousetrack", 24, 80, "MOUSE-TRACKING-OK");
    let raw = screen(&rig, "mousetrack", 24, 80);
    expect_regex_bytes(&raw, "\x1b\\[\\?1002h");
    expect_regex_bytes(&raw, "\x1b\\[\\?1006h");
}

/// node: tests/ratatui-compat.test.ts:638
#[test]
fn mouse_tracking_modes_are_cleared_when_the_child_disables_them() {
    let rig = Rig::new();
    start(&rig, "mouseoff", &child("printf '\\033[?1003h\\033[?1003l'; printf 'MOUSE-OFF-OK\\n'"));
    live_until(&rig, "mouseoff", 24, 80, "MOUSE-OFF-OK");
    let raw = screen(&rig, "mouseoff", 24, 80);
    expect_not_regex_bytes(&raw, "\x1b\\[\\?1003h");
}

/// node: tests/ratatui-compat.test.ts:668
#[test]
fn button_tracking_1000_is_tracked_and_replayed() {
    let rig = Rig::new();
    start(&rig, "mouse1000", &child("printf '\\033[?1000h'; printf 'MOUSE-1000-OK\\n'"));
    live_until(&rig, "mouse1000", 24, 80, "MOUSE-1000-OK");
    let raw = screen(&rig, "mouse1000", 24, 80);
    expect_regex_bytes(&raw, "\x1b\\[\\?1000h");
}

// ── 4. Resize timing with full-screen redraw ──

/// A child that draws a full-screen background plus `SIZE:<cols>x<rows>`
/// on row 1, and redraws `delay` seconds after every SIGWINCH.
fn resize_script(delay: &str) -> String {
    format!(
        "draw() {{ set -- $(stty size); rows=$1; cols=$2; printf '\\033[?1049h'; \
         r=1; while [ $r -le $rows ]; do printf '\\033[%d;1H\\033[48;2;40;40;40m\\033[K' $r; r=$((r+1)); done; \
         printf '\\033[1;1H\\033[48;2;40;40;40m\\033[38;2;255;255;255mSIZE:%sx%s\\033[0m' $cols $rows; }}; \
         trap '{}draw' WINCH; sleep 0.3; draw; while :; do sleep 0.02; done",
        if delay == "0" { String::new() } else { format!("sleep {delay}; ") }
    )
}

fn wait_live_text(conn: &mut Conn, marker: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let start = std::time::Instant::now();
    loop {
        if let Some(p) = conn.next_packet(Duration::from_millis(200))
            && matches!(p.type_, MessageType::Screen | MessageType::Data)
        {
            bytes.extend_from_slice(&p.payload);
            if contains(&bytes, marker) {
                return bytes;
            }
        }
        assert!(start.elapsed() < deadline(), "no {marker:?} in the live stream: {:?}", String::from_utf8_lossy(&bytes));
    }
}

/// node: tests/ratatui-compat.test.ts:744
#[test]
fn instant_redraw_resize_then_reconnect_shows_the_new_size() {
    let rig = Rig::new();
    start(&rig, "rsinstant", &resize_script("0"));
    let (mut conn, _) = live_until(&rig, "rsinstant", 24, 80, "SIZE:80x24");
    conn.resize(30, 100);
    wait_live_text(&mut conn, "SIZE:100x30");
    let replay = reconnect(&rig, "rsinstant", conn, 30, 100);
    expect_text(&replay, "SIZE:100x30");
}

/// node: tests/ratatui-compat.test.ts:772
#[test]
fn slow_redraw_resize_then_reconnect_after_the_redraw() {
    let rig = Rig::new();
    start(&rig, "rsslow", &resize_script("0.1"));
    let (mut conn, _) = live_until(&rig, "rsslow", 24, 80, "SIZE:80x24");
    conn.resize(30, 100);
    wait_live_text(&mut conn, "SIZE:100x30");
    let replay = reconnect(&rig, "rsslow", conn, 30, 100);
    expect_text(&replay, "SIZE:100x30");
}

/// node: tests/ratatui-compat.test.ts:802
#[test]
fn reconnect_at_a_different_size_after_a_slow_redraw() {
    let rig = Rig::new();
    start(&rig, "rsreslow", &resize_script("0.1"));
    let (mut conn, _) = live_until(&rig, "rsreslow", 24, 80, "SIZE:80x24");
    conn.resize(20, 60);
    wait_live_text(&mut conn, "SIZE:60x20");
    let replay = reconnect(&rig, "rsreslow", conn, 20, 60);
    expect_text(&replay, "SIZE:60x20");
}

/// node: tests/ratatui-compat.test.ts:833
#[test]
fn reconnect_right_after_a_resize_waits_for_the_redraw_to_settle() {
    let rig = Rig::new();
    start(&rig, "rsrace", &resize_script("0.05"));
    let (mut conn, _) = live_until(&rig, "rsrace", 24, 80, "SIZE:80x24");
    conn.resize(20, 60);
    // Reconnect immediately, without waiting for the app to redraw. Node's
    // client then waits for the text on the terminal fed by the SCREEN and
    // whatever DATA follows it on the same attach.
    drop(conn);
    std::thread::sleep(Duration::from_millis(100));
    let mut conn = rig.connect("rsrace");
    conn.attach(20, 60);
    let bytes = wait_live_text(&mut conn, "SIZE:60x20");
    expect_text(&bytes, "SIZE:60x20");
}

/// node: tests/ratatui-compat.test.ts:866
#[test]
fn immediate_reconnect_at_a_different_size_with_an_instant_redraw_app() {
    let rig = Rig::new();
    start(&rig, "rsfast", &resize_script("0"));
    let (mut conn, _) = live_until(&rig, "rsfast", 24, 80, "SIZE:80x24");
    conn.resize(15, 50);
    drop(conn);
    std::thread::sleep(Duration::from_millis(100));
    let mut conn = rig.connect("rsfast");
    conn.attach(15, 50);
    let bytes = wait_live_text(&mut conn, "SIZE:50x15");
    expect_text(&bytes, "SIZE:50x15");
}

// ── 5. Mixed content layout (codex-style UI) ──

/// node: tests/ratatui-compat.test.ts:897
#[test]
fn box_drawing_with_styled_content_survives_reconnect() {
    let rig = Rig::new();
    let body = format!(
        "printf '\\033[?1049h\\033[H\\033[2J'; \
         dim='\\033[2m'; bold='\\033[1m'; reset='\\033[0m'; darkbg='\\033[48;2;71;76;86m'; white='\\033[38;2;255;255;255m'; \
         bw=40; [ $cols -lt 40 ] && bw=$cols; \
         hline() {{ i=0; while [ $i -lt $1 ]; do printf '─'; i=$((i+1)); done; }}; \
         printf '\\033[1;1H'; printf \"$dim\"'╭'; hline $((bw-2)); printf '╮'\"$reset\"; \
         printf '\\033[2;1H'; printf \"$dim\"'│'\"$reset$bold$white\"' Codex CLI '\"$reset$dim\"; {}; printf '│'\"$reset\"; \
         printf '\\033[3;1H'; printf \"$dim\"'╰'; hline $((bw-2)); printf '╯'\"$reset\"; \
         printf '\\033[5;3H\\033[38;2;100;200;100mSome green content text'\"$reset\"; \
         printf '\\033[7;3H\\033[38;2;200;100;100mSome red content text'\"$reset\"; \
         printf '\\033[9;3HPlain text line here'; \
         r=$((rows-3)); while [ $r -le $rows ]; do printf '\\033[%d;1H' $r; printf \"$darkbg\"; {}; printf \"$reset\"; r=$((r+1)); done; \
         printf '\\033[%d;2H' $((rows-2)); printf \"$darkbg$white\"'> BOX-LAYOUT-DONE'\"$reset\"",
        spaces("bw - 2 - 11"),
        spaces("cols")
    );
    start(&rig, "boxlayout", &child(&body));
    let (conn, live) = live_until(&rig, "boxlayout", 24, 80, "BOX-LAYOUT-DONE");
    for t in ["╭", "╮", "╰", "╯", "Codex CLI", "Some green content text", "Some red content text", "Plain text line here"] {
        expect_text(&live, t);
    }
    expect_sgr(&live, "48;2;71;76;86");
    expect_sgr(&live, "38;2;100;200;100");
    expect_sgr(&live, "38;2;200;100;100");
    let replay = reconnect(&rig, "boxlayout", conn, 24, 80);
    for t in ["╭", "╮", "╰", "╯", "Codex CLI", "Some green content text", "Some red content text", "Plain text line here", "BOX-LAYOUT-DONE"] {
        expect_text(&replay, t);
    }
    expect_sgr(&replay, "48;2;71;76;86");
    expect_sgr(&replay, "38;2;100;200;100");
    expect_sgr(&replay, "38;2;200;100;100");
}

/// node: tests/ratatui-compat.test.ts:998
#[test]
fn horizontal_line_drawing_chars_are_preserved_exactly() {
    let rig = Rig::new();
    let body = "printf '\\033[?1049h\\033[H\\033[2J'; \
         printf '\\033[1;1H┌────┐'; printf '\\033[2;1H│ OK │'; printf '\\033[3;1H└────┘'; \
         printf '\\033[5;1H╭────╮'; printf '\\033[6;1H│ OK │'; printf '\\033[7;1H╰────╯'; \
         printf '\\033[9;1H╔════╗'; printf '\\033[10;1H║ OK ║'; printf '\\033[11;1H╚════╝'; \
         printf '\\033[13;1HBOX-CHARS-DONE'";
    start(&rig, "boxchars", &child(body));
    let chars = ["┌", "┐", "└", "┘", "╭", "╮", "╰", "╯", "╔", "╗", "╚", "╝"];
    let (conn, live) = live_until(&rig, "boxchars", 24, 80, "BOX-CHARS-DONE");
    for c in chars {
        expect_text(&live, c);
    }
    let replay = reconnect(&rig, "boxchars", conn, 24, 80);
    for c in chars {
        expect_text(&replay, c);
    }
}

/// node: tests/ratatui-compat.test.ts:1071
#[test]
fn input_area_with_cursor_at_the_bottom_survives_reconnect() {
    let rig = Rig::new();
    let body = format!(
        "printf '\\033[?1049h\\033[H\\033[2J'; \
         printf '\\033[1;1HHeader text here'; printf '\\033[12;1HMiddle content'; \
         inbg='\\033[48;2;50;50;60m'; infg='\\033[38;2;200;200;220m'; \
         r=$((rows-1)); while [ $r -le $rows ]; do printf '\\033[%d;1H' $r; printf \"$inbg\"; {}; printf '\\033[0m'; r=$((r+1)); done; \
         printf '\\033[%d;1H' $rows; printf \"$inbg$infg\"'> CURSOR-POS-OK\\033[0m'; \
         printf '\\033[%d;17H' $rows",
        spaces("cols")
    );
    start(&rig, "cursorpos", &child(&body));
    let (conn, live) = live_until(&rig, "cursorpos", 24, 80, "CURSOR-POS-OK");
    expect_text(&live, "Header text here");
    expect_text(&live, "Middle content");
    expect_text(&live, "CURSOR-POS-OK");
    let replay = reconnect(&rig, "cursorpos", conn, 24, 80);
    expect_text(&replay, "Header text here");
    expect_text(&replay, "Middle content");
    expect_text(&replay, "CURSOR-POS-OK");
    expect_regex_bytes(&replay, "\x1b\\[48;2;50;50;60m");
}

/// node: tests/ratatui-compat.test.ts:1130
#[test]
fn dense_ratatui_layout_survives_reconnect() {
    let rig = Rig::new();
    let body = format!(
        "printf '\\033[?1049h\\033[H\\033[2J'; \
         hbg='\\033[48;2;60;60;90m'; sbg='\\033[48;2;40;40;60m'; cbg='\\033[48;2;30;30;30m'; \
         white='\\033[38;2;255;255;255m'; yellow='\\033[38;2;255;200;0m'; cyan='\\033[38;2;0;200;200m'; reset='\\033[0m'; \
         fill() {{ r=$1; while [ $r -le $2 ]; do printf '\\033[%d;1H' $r; printf \"$3\"; {}; printf \"$reset\"; r=$((r+1)); done; }}; \
         fill 1 2 \"$hbg\"; \
         printf '\\033[1;2H'\"$hbg$white\"'\\033[1m Codex \\033[22m'\"$reset\"; \
         printf '\\033[2;2H'\"$hbg$yellow\"'Model: o4-mini'\"$reset\"; \
         fill 3 $((rows-2)) \"$cbg\"; \
         printf '\\033[4;3H'\"$cbg$cyan\"'user>'\"$reset$cbg$white\"' What is 2+2?'\"$reset\"; \
         printf '\\033[6;3H'\"$cbg$cyan\"'assistant>'\"$reset$cbg$white\"' The answer is 4.'\"$reset\"; \
         fill $((rows-1)) $rows \"$sbg\"; \
         printf '\\033[%d;2H' $((rows-1)); printf \"$sbg$white\"'DENSE-LAYOUT-OK'\"$reset\"; \
         printf '\\033[%d;2H' $rows; printf \"$sbg$yellow\"'Tokens: 42'\"$reset\"",
        spaces("cols")
    );
    start(&rig, "dense", &child(&body));
    let texts = ["Codex", "Model: o4-mini", "user>", "What is 2+2?", "assistant>", "The answer is 4.", "Tokens: 42"];
    let (conn, live) = live_until(&rig, "dense", 24, 80, "DENSE-LAYOUT-OK");
    for t in texts {
        expect_text(&live, t);
    }
    let replay = reconnect(&rig, "dense", conn, 24, 80);
    for t in texts {
        expect_text(&replay, t);
    }
    expect_text(&replay, "DENSE-LAYOUT-OK");
    expect_sgr(&replay, "48;2;60;60;90");
    expect_sgr(&replay, "48;2;40;40;60");
    expect_sgr(&replay, "48;2;30;30;30");
    expect_sgr(&replay, "38;2;255;200;0");
    expect_sgr(&replay, "38;2;0;200;200");
}
