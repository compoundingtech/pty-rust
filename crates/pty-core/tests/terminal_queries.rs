//! Port of the pty project's `tests/terminal-queries.test.ts`, first half:
//! `strip_terminal_queries` — pure port of the `stripTerminalQueries` util.
//!
//! The second half (the terminal query *responses* libghostty generates) needs
//! a live `Session`, so it lives in `pty-testkit/tests/terminal_queries.rs`.

use pty_core::queries::strip_terminal_queries;

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
