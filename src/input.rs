//! Raw stdin input parsing — keyboard + mouse. Port of the pty project's
//! `src/tui/input.ts`.
//!
//! Parses a byte chunk from a terminal into an ordered list of key and mouse
//! events, handling legacy xterm encodings, modified arrows, SGR mouse
//! reporting, and the Kitty keyboard protocol (CSI-u).

/// A decoded keyboard event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub name: String,
    /// The literal character, when the key produced printable text.
    pub char: Option<String>,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Mouse button identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    None,
}

/// Mouse action kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Drag,
    Move,
    ScrollUp,
    ScrollDown,
}

/// A decoded mouse event. `x`/`y` are 0-based cell coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub action: MouseAction,
    pub button: MouseButton,
    pub x: i32,
    pub y: i32,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// A key or mouse event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
}

/// Type guard: is this a mouse event?
pub fn is_mouse_event(e: &InputEvent) -> bool {
    matches!(e, InputEvent::Mouse(_))
}

/// ANSI sequences to enable SGR mouse reporting.
pub const MOUSE_ENABLE_SGR: &str = "\x1b[?1002h\x1b[?1006h";
/// ANSI sequences to disable SGR mouse reporting.
pub const MOUSE_DISABLE_SGR: &str = "\x1b[?1006l\x1b[?1002l";

/// Kitty keyboard-protocol codepoints that decode to a NAMED key event rather
/// than the raw control char, mirroring the legacy bare-key parsing.
fn kitty_codepoint_name(cp: u32) -> Option<&'static str> {
    Some(match cp {
        27 => "escape",
        13 => "return",
        9 => "tab",
        127 => "backspace",
        _ => return None,
    })
}

fn named(name: &str, ctrl: bool, alt: bool, shift: bool) -> InputEvent {
    InputEvent::Key(KeyEvent {
        name: name.to_string(),
        char: None,
        ctrl,
        alt,
        shift,
    })
}

fn printable(ch: &str, ctrl: bool, alt: bool, shift: bool) -> InputEvent {
    InputEvent::Key(KeyEvent {
        name: ch.to_string(),
        char: Some(ch.to_string()),
        ctrl,
        alt,
        shift,
    })
}

fn decode_mouse(button_code: u32, x: i32, y: i32, is_release: bool) -> Option<MouseEvent> {
    let shift = button_code & 0x04 != 0;
    let alt = button_code & 0x08 != 0;
    let ctrl = button_code & 0x10 != 0;
    let motion = button_code & 0x20 != 0;
    let wheel = button_code & 0x40 != 0;
    let low = button_code & 0x03;

    let cx = (x - 1).max(0);
    let cy = (y - 1).max(0);

    if wheel {
        return Some(MouseEvent {
            action: if low == 0 {
                MouseAction::ScrollUp
            } else {
                MouseAction::ScrollDown
            },
            button: MouseButton::None,
            x: cx,
            y: cy,
            ctrl,
            alt,
            shift,
        });
    }

    let button = match low {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::None,
    };

    let action = if is_release {
        MouseAction::Release
    } else if motion {
        if button == MouseButton::None {
            MouseAction::Move
        } else {
            MouseAction::Drag
        }
    } else {
        MouseAction::Press
    };

    Some(MouseEvent {
        action,
        button,
        x: cx,
        y: cy,
        ctrl,
        alt,
        shift,
    })
}

/// Legacy entry point — keyboard events only.
pub fn parse_key(data: &[u8]) -> Vec<KeyEvent> {
    parse_input(data)
        .into_iter()
        .filter_map(|e| match e {
            InputEvent::Key(k) => Some(k),
            InputEvent::Mouse(_) => None,
        })
        .collect()
}

/// Consume a run of ASCII digits from `chars` starting at `j`; return the
/// parsed value (if any digits) and the index past them.
fn take_digits(chars: &[char], j: usize) -> (Option<u32>, usize) {
    let start = j;
    let mut k = j;
    while k < chars.len() && chars[k].is_ascii_digit() {
        k += 1;
    }
    if k == start {
        return (None, j);
    }
    let s: String = chars[start..k].iter().collect();
    (s.parse::<u32>().ok(), k)
}

/// Parse a stdin chunk into an ordered list of keyboard + mouse events.
pub fn parse_input(data: &[u8]) -> Vec<InputEvent> {
    let s = String::from_utf8_lossy(data);
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut events: Vec<InputEvent> = Vec::new();
    let mut i = 0usize;

    while i < len {
        // ESC sequences.
        if chars[i] == '\x1b' {
            // CSI: ESC [ ...
            if i + 1 < len && chars[i + 1] == '[' {
                let p = i + 2; // start of `rest`

                // SGR mouse: ESC [ < b ; x ; y (M|m)
                if p < len && chars[p] == '<' {
                    let mut q = p + 1;
                    let (b, q1) = take_digits(&chars, q);
                    if let Some(b) = b {
                        q = q1;
                        if q < len && chars[q] == ';' {
                            q += 1;
                            let (x, q2) = take_digits(&chars, q);
                            if let Some(x) = x {
                                q = q2;
                                if q < len && chars[q] == ';' {
                                    q += 1;
                                    let (y, q3) = take_digits(&chars, q);
                                    if let Some(y) = y {
                                        q = q3;
                                        if q < len && (chars[q] == 'M' || chars[q] == 'm') {
                                            let release = chars[q] == 'm';
                                            q += 1;
                                            if let Some(ev) =
                                                decode_mouse(b, x as i32, y as i32, release)
                                            {
                                                events.push(InputEvent::Mouse(ev));
                                            }
                                            i = q;
                                            continue;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Arrow keys, home, end (bare).
                if p < len
                    && let Some(name) = match chars[p] {
                        'A' => Some("up"),
                        'B' => Some("down"),
                        'C' => Some("right"),
                        'D' => Some("left"),
                        'H' => Some("home"),
                        'F' => Some("end"),
                        _ => None,
                    } {
                        events.push(named(name, false, false, false));
                        i = p + 1;
                        continue;
                    }

                // Modified arrows: ESC [ 1 ; <mods> <letter>.
                if p + 1 < len && chars[p] == '1' && chars[p + 1] == ';' {
                    let (mods_raw, q1) = take_digits(&chars, p + 2);
                    if let Some(mods_raw) = mods_raw
                        && q1 < len
                            && let Some(name) = match chars[q1] {
                                'A' => Some("up"),
                                'B' => Some("down"),
                                'C' => Some("right"),
                                'D' => Some("left"),
                                'H' => Some("home"),
                                'F' => Some("end"),
                                _ => None,
                            } {
                                let mods = mods_raw.saturating_sub(1);
                                let shift = mods & 0x01 != 0;
                                let alt = mods & 0x02 != 0;
                                let ctrl = mods & 0x04 != 0;
                                events.push(named(name, ctrl, alt, shift));
                                i = q1 + 1;
                                continue;
                            }
                }

                // Shift+Tab (legacy): ESC [ Z
                if p < len && chars[p] == 'Z' {
                    events.push(named("backtab", false, false, true));
                    i = p + 1;
                    continue;
                }

                // ESC [ 3~ / 5~ / 6~
                if p + 1 < len && chars[p + 1] == '~'
                    && let Some(name) = match chars[p] {
                        '3' => Some("delete"),
                        '5' => Some("pageup"),
                        '6' => Some("pagedown"),
                        _ => None,
                    } {
                        events.push(named(name, false, false, false));
                        i = p + 2;
                        continue;
                    }

                // Kitty keyboard protocol: ESC [ <code> [; <mods>] u
                if p < len && chars[p].is_ascii_digit() {
                    let (code, q1) = take_digits(&chars, p);
                    if let Some(code) = code {
                        // Optional `; mods`.
                        let (mods_wire, q2) = if q1 < len && chars[q1] == ';' {
                            let (m, q) = take_digits(&chars, q1 + 1);
                            (m, q)
                        } else {
                            (Some(1), q1)
                        };
                        if let Some(mods_wire) = mods_wire
                            && q2 < len && chars[q2] == 'u' {
                                let mods = mods_wire.saturating_sub(1);
                                let shift = mods & 0x01 != 0;
                                let alt = mods & 0x02 != 0;
                                let ctrl = mods & 0x04 != 0;
                                if code == 0x09 && shift {
                                    events.push(named("backtab", ctrl, alt, shift));
                                } else if let Some(name) = kitty_codepoint_name(code) {
                                    events.push(named(name, ctrl, alt, shift));
                                } else if let Some(ch) = char::from_u32(code) {
                                    events.push(printable(&ch.to_string(), ctrl, alt, shift));
                                }
                                i = q2 + 1;
                                continue;
                            }
                    }
                }

                // Unknown CSI — skip to the final byte (@..~).
                let mut k = p;
                while k < len && !('@'..='~').contains(&chars[k]) {
                    k += 1;
                }
                i = k + 1;
                continue;
            }

            // Alt+<char>: ESC followed by a printable character.
            if i + 1 < len && chars[i + 1] >= ' ' {
                let ch = chars[i + 1].to_string();
                events.push(printable(&ch, false, true, false));
                i += 2;
                continue;
            }

            // Bare ESC.
            events.push(named("escape", false, false, false));
            i += 1;
            continue;
        }

        // Control characters.
        let code = chars[i] as u32;

        if code == 0x0d {
            events.push(named("return", false, false, false));
            i += 1;
            continue;
        }
        if code == 0x09 {
            events.push(named("tab", false, false, false));
            i += 1;
            continue;
        }
        if code == 0x7f {
            events.push(named("backspace", false, false, false));
            i += 1;
            continue;
        }
        if code == 0x1c {
            events.push(named("\\", true, false, false));
            i += 1;
            continue;
        }

        // Ctrl+A..Ctrl+Z (0x01–0x1a).
        if (0x01..=0x1a).contains(&code) {
            let letter = char::from_u32(code + 0x60).unwrap().to_string();
            events.push(named(&letter, true, false, false));
            i += 1;
            continue;
        }

        // Regular printable character.
        if code >= 0x20 {
            let ch = chars[i].to_string();
            events.push(printable(&ch, false, false, false));
            i += 1;
            continue;
        }

        // Unknown — skip.
        i += 1;
    }

    events
}
