//! Port of the pty project's `tests/mouse-parse.test.ts`.

use pty_testkit::input::{is_mouse_event, parse_input, InputEvent, MouseAction, MouseButton, MouseEvent};

fn parse(s: &str) -> Vec<InputEvent> {
    parse_input(s.as_bytes())
}

fn mouse(s: &str) -> MouseEvent {
    let events = parse(s);
    assert_eq!(events.len(), 1, "expected exactly one event");
    assert!(is_mouse_event(&events[0]));
    match &events[0] {
        InputEvent::Mouse(m) => *m,
        _ => unreachable!(),
    }
}

#[test]
fn left_press_0_based() {
    let e = mouse("\x1b[<0;10;5M");
    assert_eq!(e.action, MouseAction::Press);
    assert_eq!(e.button, MouseButton::Left);
    assert_eq!(e.x, 9);
    assert_eq!(e.y, 4);
    assert!(!e.ctrl);
    assert!(!e.alt);
    assert!(!e.shift);
}

#[test]
fn left_release_lowercase_m() {
    let e = mouse("\x1b[<0;10;5m");
    assert_eq!(e.action, MouseAction::Release);
    assert_eq!(e.button, MouseButton::Left);
}

#[test]
fn middle_and_right_buttons() {
    assert_eq!(mouse("\x1b[<1;1;1M").button, MouseButton::Middle);
    assert_eq!(mouse("\x1b[<2;1;1M").button, MouseButton::Right);
}

#[test]
fn drag_motion_flag() {
    let e = mouse("\x1b[<32;5;5M");
    assert_eq!(e.action, MouseAction::Drag);
    assert_eq!(e.button, MouseButton::Left);
}

#[test]
fn hover_move() {
    let e = mouse("\x1b[<35;5;5M");
    assert_eq!(e.action, MouseAction::Move);
    assert_eq!(e.button, MouseButton::None);
}

#[test]
fn scroll_wheel() {
    let up = mouse("\x1b[<64;10;10M");
    assert_eq!(up.action, MouseAction::ScrollUp);
    assert_eq!(up.button, MouseButton::None);
    let down = mouse("\x1b[<65;10;10M");
    assert_eq!(down.action, MouseAction::ScrollDown);
}

#[test]
fn modifiers_from_bits() {
    let e = mouse("\x1b[<28;1;1M");
    assert!(e.shift);
    assert!(e.alt);
    assert!(e.ctrl);
}

#[test]
fn interleaves_mouse_with_key_events() {
    let events = parse("a\x1b[<0;3;4Mb");
    assert_eq!(events.len(), 3);
    match &events[0] {
        InputEvent::Key(k) => assert_eq!(k.name, "a"),
        _ => panic!("events[0] should be key 'a'"),
    }
    assert!(is_mouse_event(&events[1]));
    match &events[2] {
        InputEvent::Key(k) => assert_eq!(k.name, "b"),
        _ => panic!("events[2] should be key 'b'"),
    }
}

#[test]
fn parse_key_filters_out_mouse_events() {
    let keys = pty_testkit::input::parse_key("a\x1b[<0;3;4Mb".as_bytes());
    let names: Vec<&str> = keys.iter().map(|k| k.name.as_str()).collect();
    assert_eq!(names, vec!["a", "b"]);
}
