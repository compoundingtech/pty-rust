//! Input events with Node's names, ported from `src/tui/input.ts`.
//!
//! [`KeyEvent`] carries a `name` (`up down left right home end pageup
//! pagedown delete tab backtab return escape backspace`, a letter for
//! ctrl+letter, or the character itself), the printable `ch`, and the
//! modifier flags. [`MouseEvent`] is an SGR mouse report with 0-based
//! coordinates. Two decoders produce them: [`parse_input`] reads raw bytes
//! exactly the way Node's `parseInput` does (legacy escapes, modified
//! arrows, kitty CSI-u with an optional modifier parameter, SGR mouse), and
//! [`from_crossterm_key`] / [`from_crossterm_mouse`] map crossterm's events
//! for hosts that use crossterm's reader.

use crossterm::event::{
    KeyCode, KeyEvent as CtKey, KeyModifiers, MouseButton as CtButton, MouseEvent as CtMouse,
    MouseEventKind,
};

/// A key press (`KeyEvent`, `input.ts:3-12`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyEvent {
    /// The key name; the character itself for printable keys.
    pub name: String,
    /// The printable text, when the key types something.
    pub ch: Option<String>,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl KeyEvent {
    /// A named key with no modifiers (`{ name }`).
    pub fn named(name: &str) -> Self {
        KeyEvent {
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// A printable key (`{ name: ch, char: ch }`).
    pub fn printable(ch: &str) -> Self {
        KeyEvent {
            name: ch.to_string(),
            ch: Some(ch.to_string()),
            ..Default::default()
        }
    }

    /// `ctrl+<letter>` (`{ name: letter, ctrl: true }`, no char).
    pub fn ctrl(letter: &str) -> Self {
        KeyEvent {
            name: letter.to_string(),
            ctrl: true,
            ..Default::default()
        }
    }

    /// `alt+<char>` (`{ name: ch, char: ch, alt: true }`).
    pub fn alt(ch: &str) -> Self {
        KeyEvent {
            name: ch.to_string(),
            ch: Some(ch.to_string()),
            alt: true,
            ..Default::default()
        }
    }

    pub fn with_ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    pub fn with_alt(mut self) -> Self {
        self.alt = true;
        self
    }

    pub fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    /// Is this `name` with no modifiers at all?
    pub fn is_plain(&self, name: &str) -> bool {
        self.name == name && !self.ctrl && !self.alt && !self.shift
    }

    /// Is this `ctrl+<name>` (alt and shift clear)?
    pub fn is_ctrl(&self, name: &str) -> bool {
        self.name == name && self.ctrl && !self.alt
    }

    /// The printable character when the key types one (no ctrl/alt).
    pub fn typed(&self) -> Option<&str> {
        if self.ctrl || self.alt {
            return None;
        }
        self.ch.as_deref()
    }
}

/// `MouseButton` (`input.ts:14`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    None,
}

/// `MouseAction` (`input.ts:15`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Drag,
    Move,
    ScrollUp,
    ScrollDown,
}

/// An SGR mouse report with 0-based coordinates (`MouseEvent`,
/// `input.ts:17-28`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub action: MouseAction,
    pub button: MouseButton,
    /// 0-based column.
    pub x: u16,
    /// 0-based row.
    pub y: u16,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// A key or mouse event (`InputEvent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
}

/// Enable SGR mouse reporting (button-event tracking, `input.ts:54`).
pub const MOUSE_ENABLE_SGR: &str = "\x1b[?1002h\x1b[?1006h";
/// Disable it (`input.ts:55`).
pub const MOUSE_DISABLE_SGR: &str = "\x1b[?1006l\x1b[?1002l";

/// `decodeMouse` (`input.ts:57-95`).
fn decode_mouse(code: u32, x: u32, y: u32, release: bool) -> MouseEvent {
    let shift = code & 0x04 != 0;
    let alt = code & 0x08 != 0;
    let ctrl = code & 0x10 != 0;
    let motion = code & 0x20 != 0;
    let wheel = code & 0x40 != 0;
    let low = code & 0x03;
    let x = x.saturating_sub(1).min(u16::MAX as u32) as u16;
    let y = y.saturating_sub(1).min(u16::MAX as u32) as u16;
    let mods = |action, button| MouseEvent {
        action,
        button,
        x,
        y,
        ctrl,
        alt,
        shift,
    };
    if wheel {
        let action = if low == 0 {
            MouseAction::ScrollUp
        } else {
            MouseAction::ScrollDown
        };
        return mods(action, MouseButton::None);
    }
    let button = match low {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::None,
    };
    if release {
        return mods(MouseAction::Release, button);
    }
    if motion {
        let action = if button == MouseButton::None {
            MouseAction::Move
        } else {
            MouseAction::Drag
        };
        return mods(action, button);
    }
    mods(MouseAction::Press, button)
}

fn kitty_mods(wire: Option<u32>) -> (bool, bool, bool) {
    let mods = wire.unwrap_or(1).saturating_sub(1);
    (mods & 1 != 0, mods & 2 != 0, mods & 4 != 0)
}

/// Read decimal digits at `i`; returns the value and the index after.
fn digits(s: &[char], mut i: usize) -> Option<(u32, usize)> {
    let start = i;
    let mut v: u32 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = v.saturating_mul(10).saturating_add(s[i] as u32 - '0' as u32);
        i += 1;
    }
    (i > start).then_some((v, i))
}

fn letter_to_name(c: char) -> Option<&'static str> {
    Some(match c {
        'A' => "up",
        'B' => "down",
        'C' => "right",
        'D' => "left",
        'H' => "home",
        'F' => "end",
        _ => return None,
    })
}

/// Parse a stdin chunk into key and mouse events (`parseInput`,
/// `input.ts:104-249`).
pub fn parse_input(data: &[u8]) -> Vec<InputEvent> {
    let text = String::from_utf8_lossy(data);
    let s: Vec<char> = text.chars().collect();
    let mut events = Vec::new();
    let mut i = 0;
    let push_key = |events: &mut Vec<InputEvent>, k: KeyEvent| events.push(InputEvent::Key(k));
    while i < s.len() {
        if s[i] == '\x1b' {
            if i + 1 < s.len() && s[i + 1] == '[' {
                let r = i + 2; // start of the CSI body
                // SGR mouse: ESC [ < b ; x ; y (M|m)
                if s.get(r) == Some(&'<')
                    && let Some((b, j)) = digits(&s, r + 1)
                    && s.get(j) == Some(&';')
                    && let Some((x, j)) = digits(&s, j + 1)
                    && s.get(j) == Some(&';')
                    && let Some((y, j)) = digits(&s, j + 1)
                    && let Some(&fin) = s.get(j)
                    && (fin == 'M' || fin == 'm')
                {
                    events.push(InputEvent::Mouse(decode_mouse(b, x, y, fin == 'm')));
                    i = j + 1;
                    continue;
                }
                // Plain arrows, home, end.
                if let Some(&c) = s.get(r)
                    && let Some(name) = letter_to_name(c)
                {
                    push_key(&mut events, KeyEvent::named(name));
                    i = r + 1;
                    continue;
                }
                // Modified arrows: ESC [ 1 ; mods X
                if s.get(r) == Some(&'1')
                    && s.get(r + 1) == Some(&';')
                    && let Some((mods, j)) = digits(&s, r + 2)
                    && let Some(&c) = s.get(j)
                    && let Some(name) = letter_to_name(c)
                {
                    let (shift, alt, ctrl) = kitty_mods(Some(mods));
                    push_key(
                        &mut events,
                        KeyEvent {
                            name: name.to_string(),
                            ch: None,
                            ctrl,
                            alt,
                            shift,
                        },
                    );
                    i = j + 1;
                    continue;
                }
                // Shift+Tab (legacy): ESC [ Z
                if s.get(r) == Some(&'Z') {
                    push_key(&mut events, KeyEvent::named("backtab").with_shift());
                    i = r + 1;
                    continue;
                }
                // Delete / page up / page down.
                if s.get(r + 1) == Some(&'~') {
                    let name = match s.get(r) {
                        Some('3') => Some("delete"),
                        Some('5') => Some("pageup"),
                        Some('6') => Some("pagedown"),
                        _ => None,
                    };
                    if let Some(name) = name {
                        push_key(&mut events, KeyEvent::named(name));
                        i = r + 2;
                        continue;
                    }
                }
                // Kitty CSI-u: ESC [ code [; mods] u
                if let Some((code, j)) = digits(&s, r) {
                    let (mods, j) = if s.get(j) == Some(&';') {
                        match digits(&s, j + 1) {
                            Some((m, j2)) => (Some(m), j2),
                            None => (None, j),
                        }
                    } else {
                        (None, j)
                    };
                    if s.get(j) == Some(&'u') {
                        let (shift, alt, ctrl) = kitty_mods(mods);
                        let key = if code == 9 && shift {
                            KeyEvent {
                                name: "backtab".into(),
                                ch: None,
                                ctrl,
                                alt,
                                shift,
                            }
                        } else if let Some(name) = match code {
                            27 => Some("escape"),
                            13 => Some("return"),
                            9 => Some("tab"),
                            127 => Some("backspace"),
                            _ => None,
                        } {
                            KeyEvent {
                                name: name.into(),
                                ch: None,
                                ctrl,
                                alt,
                                shift,
                            }
                        } else {
                            let ch = char::from_u32(code)
                                .map(|c| c.to_string())
                                .unwrap_or_default();
                            KeyEvent {
                                name: ch.clone(),
                                ch: Some(ch),
                                ctrl,
                                alt,
                                shift,
                            }
                        };
                        push_key(&mut events, key);
                        i = j + 1;
                        continue;
                    }
                }
                // Unknown CSI: skip to the final byte.
                let mut j = r;
                while j < s.len() && !('@'..='~').contains(&s[j]) {
                    j += 1;
                }
                i = j + 1;
                continue;
            }
            // Alt+<char>: ESC followed by a printable character.
            if i + 1 < s.len() && s[i + 1] >= ' ' {
                push_key(&mut events, KeyEvent::alt(&s[i + 1].to_string()));
                i += 2;
                continue;
            }
            push_key(&mut events, KeyEvent::named("escape"));
            i += 1;
            continue;
        }
        let code = s[i] as u32;
        match code {
            0x0d => push_key(&mut events, KeyEvent::named("return")),
            0x09 => push_key(&mut events, KeyEvent::named("tab")),
            0x7f => push_key(&mut events, KeyEvent::named("backspace")),
            0x1c => push_key(&mut events, KeyEvent::ctrl("\\")),
            0x01..=0x1a => {
                let letter = char::from_u32(code + 0x60).unwrap().to_string();
                push_key(&mut events, KeyEvent::ctrl(&letter));
            }
            c if c >= 0x20 => push_key(&mut events, KeyEvent::printable(&s[i].to_string())),
            _ => {}
        }
        i += 1;
    }
    events
}

/// Keyboard events only (`parseKey`).
pub fn parse_key(data: &[u8]) -> Vec<KeyEvent> {
    parse_input(data)
        .into_iter()
        .filter_map(|e| match e {
            InputEvent::Key(k) => Some(k),
            InputEvent::Mouse(_) => None,
        })
        .collect()
}

/// Map a crossterm key event to Node's shape. Release/repeat events (kitty
/// report-all-keys) map the same way; filter them upstream if unwanted.
pub fn from_crossterm_key(ev: &CtKey) -> Option<KeyEvent> {
    let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
    let alt = ev.modifiers.contains(KeyModifiers::ALT);
    let shift = ev.modifiers.contains(KeyModifiers::SHIFT);
    let named = |name: &str| KeyEvent {
        name: name.to_string(),
        ch: None,
        ctrl,
        alt,
        shift,
    };
    Some(match ev.code {
        KeyCode::Up => named("up"),
        KeyCode::Down => named("down"),
        KeyCode::Left => named("left"),
        KeyCode::Right => named("right"),
        KeyCode::Home => named("home"),
        KeyCode::End => named("end"),
        KeyCode::PageUp => named("pageup"),
        KeyCode::PageDown => named("pagedown"),
        KeyCode::Delete => named("delete"),
        KeyCode::Tab => named("tab"),
        KeyCode::BackTab => KeyEvent {
            shift: true,
            ..named("backtab")
        },
        KeyCode::Enter => named("return"),
        KeyCode::Esc => named("escape"),
        KeyCode::Backspace => named("backspace"),
        KeyCode::Char(c) => {
            if ctrl {
                // Node reports ctrl+letter as the lowercase letter, no char.
                let letter = c.to_ascii_lowercase().to_string();
                KeyEvent {
                    name: letter,
                    ch: None,
                    ctrl,
                    alt,
                    shift,
                }
            } else {
                let ch = c.to_string();
                KeyEvent {
                    name: ch.clone(),
                    ch: Some(ch),
                    ctrl,
                    alt,
                    shift,
                }
            }
        }
        _ => return None,
    })
}

/// Map a crossterm mouse event to Node's shape (`None` for horizontal
/// scrolling, which Node has no name for).
pub fn from_crossterm_mouse(ev: &CtMouse) -> Option<MouseEvent> {
    let button = |b: CtButton| match b {
        CtButton::Left => MouseButton::Left,
        CtButton::Middle => MouseButton::Middle,
        CtButton::Right => MouseButton::Right,
    };
    let (action, button) = match ev.kind {
        MouseEventKind::Down(b) => (MouseAction::Press, button(b)),
        MouseEventKind::Up(b) => (MouseAction::Release, button(b)),
        MouseEventKind::Drag(b) => (MouseAction::Drag, button(b)),
        MouseEventKind::Moved => (MouseAction::Move, MouseButton::None),
        MouseEventKind::ScrollUp => (MouseAction::ScrollUp, MouseButton::None),
        MouseEventKind::ScrollDown => (MouseAction::ScrollDown, MouseButton::None),
        MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => return None,
    };
    Some(MouseEvent {
        action,
        button,
        x: ev.column,
        y: ev.row,
        ctrl: ev.modifiers.contains(KeyModifiers::CONTROL),
        alt: ev.modifiers.contains(KeyModifiers::ALT),
        shift: ev.modifiers.contains(KeyModifiers::SHIFT),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(s: &str) -> Vec<KeyEvent> {
        parse_key(s.as_bytes())
    }
    fn k(name: &str) -> KeyEvent {
        KeyEvent::named(name)
    }
    fn mouse(s: &str) -> MouseEvent {
        let ev = parse_input(s.as_bytes());
        assert_eq!(ev.len(), 1);
        match &ev[0] {
            InputEvent::Mouse(m) => *m,
            other => panic!("not a mouse event: {other:?}"),
        }
    }

    /// node: tests/input-parse.test.ts:8-40
    #[test]
    fn basics() {
        assert_eq!(keys("a"), vec![KeyEvent::printable("a")]);
        assert_eq!(keys("\r"), vec![k("return")]);
        assert_eq!(keys("\t"), vec![k("tab")]);
        assert_eq!(keys("\x7f"), vec![k("backspace")]);
        assert_eq!(keys("\x1b"), vec![k("escape")]);
        assert_eq!(keys("\x1b[A"), vec![k("up")]);
        assert_eq!(keys("\x1b[B"), vec![k("down")]);
        assert_eq!(keys("\x1b[C"), vec![k("right")]);
        assert_eq!(keys("\x1b[D"), vec![k("left")]);
        assert_eq!(keys("\x01"), vec![KeyEvent::ctrl("a")]);
        assert_eq!(keys("\x1ba"), vec![KeyEvent::alt("a")]);
        assert_eq!(keys("\x1c"), vec![KeyEvent::ctrl("\\")]);
    }

    /// node: tests/input-parse.test.ts:42-77
    #[test]
    fn backtab_encodings() {
        assert_eq!(keys("\x1b[Z"), vec![k("backtab").with_shift()]);
        assert_eq!(keys("\x1b[9;2u"), vec![k("backtab").with_shift()]);
        assert_eq!(keys("\x1b[9;6u"), vec![k("backtab").with_shift().with_ctrl()]);
        assert_eq!(keys("\x1b[9;1u"), vec![k("tab")]);
    }

    /// node: tests/input-parse.test.ts:79-119
    #[test]
    fn kitty_named_keys() {
        assert_eq!(keys("\x1b[27u"), vec![k("escape")]);
        assert_eq!(keys("\x1b[27;1u"), vec![k("escape")]);
        assert_eq!(keys("\x1b[13u"), vec![k("return")]);
        assert_eq!(keys("\x1b[127u"), vec![k("backspace")]);
        assert_eq!(keys("\x1b[27;5u"), vec![k("escape").with_ctrl()]);
        assert_eq!(keys("\x1b[97u"), vec![KeyEvent::printable("a")]);
    }

    /// node: tests/input-parse.test.ts:121-142
    #[test]
    fn modified_arrows() {
        assert_eq!(keys("\x1b[1;3D"), vec![k("left").with_alt()]);
        assert_eq!(keys("\x1b[1;3C"), vec![k("right").with_alt()]);
        assert_eq!(keys("\x1b[1;2A"), vec![k("up").with_shift()]);
        assert_eq!(keys("\x1b[1;8F"), vec![k("end").with_ctrl().with_alt().with_shift()]);
    }

    /// node: tests/input-parse.test.ts:144-158
    #[test]
    fn kitty_modifiers() {
        assert_eq!(keys("\x1b[97;2u"), vec![KeyEvent::printable("a").with_shift()]);
        assert_eq!(
            keys("\x1b[97;8u"),
            vec![KeyEvent::printable("a").with_ctrl().with_alt().with_shift()]
        );
    }

    #[test]
    fn other_named_and_unknown_csi() {
        assert_eq!(keys("\x1b[3~"), vec![k("delete")]);
        assert_eq!(keys("\x1b[5~"), vec![k("pageup")]);
        assert_eq!(keys("\x1b[6~"), vec![k("pagedown")]);
        assert_eq!(keys("\x1b[H\x1b[F"), vec![k("home"), k("end")]);
        // Unknown CSI is skipped to its final byte.
        assert_eq!(keys("\x1b[?1;2cX"), vec![KeyEvent::printable("X")]);
    }

    /// node: tests/mouse-parse.test.ts:15-71
    #[test]
    fn sgr_mouse() {
        let e = mouse("\x1b[<0;10;5M");
        assert_eq!(e.action, MouseAction::Press);
        assert_eq!(e.button, MouseButton::Left);
        assert_eq!((e.x, e.y), (9, 4));
        assert!(!e.ctrl && !e.alt && !e.shift);
        let e = mouse("\x1b[<0;10;5m");
        assert_eq!(e.action, MouseAction::Release);
        assert_eq!(e.button, MouseButton::Left);
        assert_eq!(mouse("\x1b[<1;1;1M").button, MouseButton::Middle);
        assert_eq!(mouse("\x1b[<2;1;1M").button, MouseButton::Right);
        let e = mouse("\x1b[<32;5;5M");
        assert_eq!(e.action, MouseAction::Drag);
        assert_eq!(e.button, MouseButton::Left);
        let e = mouse("\x1b[<35;5;5M");
        assert_eq!(e.action, MouseAction::Move);
        assert_eq!(e.button, MouseButton::None);
        let up = mouse("\x1b[<64;10;10M");
        assert_eq!(up.action, MouseAction::ScrollUp);
        assert_eq!(up.button, MouseButton::None);
        assert_eq!(mouse("\x1b[<65;10;10M").action, MouseAction::ScrollDown);
        let e = mouse("\x1b[<28;1;1M");
        assert!(e.shift && e.alt && e.ctrl);
    }

    /// node: tests/mouse-parse.test.ts:73-80
    #[test]
    fn interleaves_mouse_and_keys() {
        let ev = parse_input(b"a\x1b[<0;3;4Mb");
        assert_eq!(ev.len(), 3);
        assert_eq!(ev[0], InputEvent::Key(KeyEvent::printable("a")));
        assert!(matches!(ev[1], InputEvent::Mouse(_)));
        assert_eq!(ev[2], InputEvent::Key(KeyEvent::printable("b")));
        let names: Vec<String> = parse_key(b"a\x1b[<0;3;4Mb")
            .into_iter()
            .map(|k| k.name)
            .collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn crossterm_mapping() {
        let ev = CtKey::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(from_crossterm_key(&ev), Some(KeyEvent::ctrl("c")));
        let ev = CtKey::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(from_crossterm_key(&ev), Some(k("backtab").with_shift()));
        let ev = CtKey::new(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(from_crossterm_key(&ev), Some(KeyEvent::alt("x")));
        let ev = CtKey::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(from_crossterm_key(&ev), Some(k("escape")));
        let m = CtMouse {
            kind: MouseEventKind::ScrollDown,
            column: 3,
            row: 4,
            modifiers: KeyModifiers::NONE,
        };
        let e = from_crossterm_mouse(&m).unwrap();
        assert_eq!(e.action, MouseAction::ScrollDown);
        assert_eq!((e.x, e.y), (3, 4));
    }
}
