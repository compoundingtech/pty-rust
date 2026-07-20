//! Named-key resolution, ported from the pty project's `src/keys.ts`.
//!
//! Turns key specs like `ctrl+c`, `return`, `alt+x`, `shift+tab` into the byte
//! sequence a real terminal would send to the PTY. Modifier combinations use
//! the xterm modifier-parameter encoding; control chars use CSI-u.

use std::collections::HashSet;

/// Error returned when a key spec references an unknown key or modifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyError(pub String);

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for KeyError {}

/// Base named keys → their unmodified byte sequence.
fn key_map(name: &str) -> Option<&'static str> {
    Some(match name {
        "return" | "enter" => "\r",
        "tab" => "\t",
        "escape" | "esc" => "\x1b",
        "space" => " ",
        "backspace" => "\x7f",
        "delete" => "\x1b[3~",
        "up" => "\x1b[A",
        "down" => "\x1b[B",
        "right" => "\x1b[C",
        "left" => "\x1b[D",
        "home" => "\x1b[H",
        "end" => "\x1b[F",
        "pageup" => "\x1b[5~",
        "pagedown" => "\x1b[6~",
        _ => return None,
    })
}

/// CSI-u keycodes for control-char keys under modifiers.
fn csi_u_keycode(name: &str) -> Option<u32> {
    Some(match name {
        "return" | "enter" => 13,
        "tab" => 9,
        "escape" | "esc" => 27,
        "space" => 32,
        "backspace" => 127,
        _ => return None,
    })
}

fn is_modifier(m: &str) -> bool {
    matches!(m, "ctrl" | "alt" | "shift")
}

/// xterm modifier parameter: `1 + bitmask(shift=1, alt=2, ctrl=4)`.
fn modifier_param(mods: &HashSet<String>) -> u32 {
    1 + if mods.contains("shift") { 1 } else { 0 }
        + if mods.contains("alt") { 2 } else { 0 }
        + if mods.contains("ctrl") { 4 } else { 0 }
}

/// Parse a key spec like `ctrl+c`, `return`, `alt+x` into the bytes a terminal
/// would send. Returns `Err` for unknown modifiers or keys.
pub fn resolve_key(spec: &str) -> Result<String, KeyError> {
    let lower = spec.to_lowercase();
    let mut parts: Vec<&str> = lower.split('+').collect();
    // `pop()` on the last element as the base key.
    let base = parts.pop().unwrap_or("").to_string();
    let mods: HashSet<String> = parts.iter().map(|s| s.to_string()).collect();

    for m in &mods {
        if !is_modifier(m) {
            return Err(KeyError(format!(
                "Unknown modifier: \"{m}\" in key spec \"{spec}\""
            )));
        }
    }

    let is_letter = base.len() == 1 && base.as_bytes()[0].is_ascii_lowercase();
    let has_modifiers = !mods.is_empty();
    let mapped = key_map(&base);

    if mapped.is_none() && !is_letter {
        return Err(KeyError(format!(
            "Unknown key: \"{base}\" in key spec \"{spec}\""
        )));
    }

    // Single-letter keys.
    if is_letter {
        let mut result = base.clone();
        if mods.contains("shift") {
            result = result.to_uppercase();
        }
        if mods.contains("ctrl") {
            let code = result.to_lowercase().as_bytes()[0] as u32;
            result = char::from_u32(code - 96).unwrap().to_string();
        }
        if mods.contains("alt") {
            result = format!("\x1b{result}");
        }
        return Ok(result);
    }

    let mapped = mapped.unwrap();

    // Named keys without modifiers: return the mapped value directly.
    if !has_modifiers {
        return Ok(mapped.to_string());
    }

    let modp = modifier_param(&mods);

    // Special case: shift+tab produces the legacy backtab sequence.
    if base == "tab" && modp == 2 {
        return Ok("\x1b[Z".to_string());
    }

    // CSI sequences of form ESC [ N ~  (e.g. delete, pageup).
    if let Some(digits) = mapped
        .strip_prefix("\x1b[")
        .and_then(|s| s.strip_suffix('~'))
    {
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            return Ok(format!("\x1b[{digits};{modp}~"));
        }
    }

    // CSI sequences of form ESC [ X  (single uppercase letter: arrows, home, end).
    if let Some(letter) = mapped.strip_prefix("\x1b[") {
        if letter.len() == 1 && letter.as_bytes()[0].is_ascii_uppercase() {
            return Ok(format!("\x1b[1;{modp}{letter}"));
        }
    }

    // Control-char keys (return, tab, escape, space, backspace): CSI-u encoding.
    if let Some(keycode) = csi_u_keycode(&base) {
        return Ok(format!("\x1b[{keycode};{modp}u"));
    }

    Ok(mapped.to_string())
}

/// If `value` starts with `key:`, resolve the key name; otherwise return the
/// literal string.
pub fn parse_seq_value(value: &str) -> Result<String, KeyError> {
    if let Some(rest) = value.strip_prefix("key:") {
        resolve_key(rest)
    } else {
        Ok(value.to_string())
    }
}
