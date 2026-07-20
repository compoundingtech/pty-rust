//! Port of the pty project's `tests/input-parse.test.ts`.

use pty_testkit::input::{parse_key, KeyEvent};

fn pk(s: &str) -> Vec<KeyEvent> {
    parse_key(s.as_bytes())
}

fn named(name: &str, ctrl: bool, alt: bool, shift: bool) -> KeyEvent {
    KeyEvent {
        name: name.to_string(),
        char: None,
        ctrl,
        alt,
        shift,
    }
}

fn ch(name: &str, c: &str, ctrl: bool, alt: bool, shift: bool) -> KeyEvent {
    KeyEvent {
        name: name.to_string(),
        char: Some(c.to_string()),
        ctrl,
        alt,
        shift,
    }
}

// ── basics ──

#[test]
fn plain_printable_character() {
    assert_eq!(pk("a"), vec![ch("a", "a", false, false, false)]);
}

#[test]
fn return_tab_backspace_named() {
    assert_eq!(pk("\r"), vec![named("return", false, false, false)]);
    assert_eq!(pk("\t"), vec![named("tab", false, false, false)]);
    assert_eq!(pk("\x7f"), vec![named("backspace", false, false, false)]);
}

#[test]
fn bare_esc_is_escape() {
    assert_eq!(pk("\x1b"), vec![named("escape", false, false, false)]);
}

#[test]
fn arrow_keys() {
    assert_eq!(pk("\x1b[A"), vec![named("up", false, false, false)]);
    assert_eq!(pk("\x1b[B"), vec![named("down", false, false, false)]);
    assert_eq!(pk("\x1b[C"), vec![named("right", false, false, false)]);
    assert_eq!(pk("\x1b[D"), vec![named("left", false, false, false)]);
}

#[test]
fn ctrl_letter() {
    assert_eq!(pk("\x01"), vec![named("a", true, false, false)]);
}

#[test]
fn alt_letter() {
    assert_eq!(pk("\x1ba"), vec![ch("a", "a", false, true, false)]);
}

// ── shift+tab (backtab) ──

#[test]
fn esc_bracket_z_is_backtab() {
    assert_eq!(pk("\x1b[Z"), vec![named("backtab", false, false, true)]);
}

#[test]
fn kitty_backtab() {
    assert_eq!(pk("\x1b[9;2u"), vec![named("backtab", false, false, true)]);
}

#[test]
fn kitty_shift_ctrl_tab() {
    assert_eq!(pk("\x1b[9;6u"), vec![named("backtab", true, false, true)]);
}

#[test]
fn kitty_plain_tab_is_named_tab() {
    assert_eq!(pk("\x1b[9;1u"), vec![named("tab", false, false, false)]);
}

// ── kitty CSI-u named special keys ──

#[test]
fn kitty_esc_mods_omitted() {
    assert_eq!(pk("\x1b[27u"), vec![named("escape", false, false, false)]);
}

#[test]
fn kitty_esc_explicit_no_mods() {
    assert_eq!(pk("\x1b[27;1u"), vec![named("escape", false, false, false)]);
}

#[test]
fn kitty_return_and_backspace_mods_omitted() {
    assert_eq!(pk("\x1b[13u"), vec![named("return", false, false, false)]);
    assert_eq!(pk("\x1b[127u"), vec![named("backspace", false, false, false)]);
}

#[test]
fn kitty_ctrl_escape() {
    assert_eq!(pk("\x1b[27;5u"), vec![named("escape", true, false, false)]);
}

#[test]
fn kitty_non_special_codepoint_decodes_to_char() {
    assert_eq!(pk("\x1b[97u"), vec![ch("a", "a", false, false, false)]);
}

// ── modified arrow keys ──

#[test]
fn option_left() {
    assert_eq!(pk("\x1b[1;3D"), vec![named("left", false, true, false)]);
}

#[test]
fn option_right() {
    assert_eq!(pk("\x1b[1;3C"), vec![named("right", false, true, false)]);
}

#[test]
fn shift_up() {
    assert_eq!(pk("\x1b[1;2A"), vec![named("up", false, false, true)]);
}

#[test]
fn ctrl_shift_alt_end() {
    assert_eq!(pk("\x1b[1;8F"), vec![named("end", true, true, true)]);
}

// ── kitty modifier extraction ──

#[test]
fn kitty_extracts_shift() {
    assert_eq!(pk("\x1b[97;2u"), vec![ch("a", "a", false, false, true)]);
}

#[test]
fn kitty_extracts_all_three_modifiers() {
    assert_eq!(pk("\x1b[97;8u"), vec![ch("a", "a", true, true, true)]);
}
