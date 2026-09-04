//! Multi-line composer (`src/tui/widgets/text-area.ts`): logical lines with
//! a row/col cursor. Keys: `return` splits the line, `backspace`/`delete`
//! merge at line edges, `left`/`right` wrap across lines (alt = word),
//! `alt+b`/`alt+f`, `up`/`down` (column clamped), `home`/`ctrl+a`,
//! `end`/`ctrl+e`, printable inserts. `tab`, `backtab`, `escape` and
//! `ctrl+return` are left to the caller (`None`).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::input::KeyEvent;
use crate::line_edit::{next_word_boundary, prev_word_boundary};
use crate::theme::{Color, Theme};

/// `TextAreaState` (`text-area.ts:20-26`); `col` counts characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextAreaState {
    pub lines: Vec<String>,
    pub row: usize,
    pub col: usize,
}

fn len(s: &str) -> usize {
    s.chars().count()
}

fn slice(s: &str, from: usize, to: Option<usize>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let from = from.min(chars.len());
    let to = to.unwrap_or(chars.len()).min(chars.len()).max(from);
    chars[from..to].iter().collect()
}

impl TextAreaState {
    /// `createTextArea`: split on newlines; empty text is one empty line.
    pub fn new(initial: &str) -> Self {
        let lines = if initial.is_empty() {
            vec![String::new()]
        } else {
            initial.split('\n').map(str::to_string).collect()
        };
        TextAreaState {
            lines,
            row: 0,
            col: 0,
        }
    }

    /// `textAreaToString`.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    fn with_line(&self, line: String) -> Self {
        let mut lines = self.lines.clone();
        lines[self.row] = line;
        TextAreaState {
            lines,
            ..self.clone()
        }
    }

    fn at(&self, row: usize, col: usize) -> Self {
        TextAreaState {
            row,
            col,
            ..self.clone()
        }
    }
}

impl Default for TextAreaState {
    fn default() -> Self {
        Self::new("")
    }
}

/// `applyTextAreaKey` (`text-area.ts:42-178`); `None` when the caller
/// should handle the key.
pub fn apply_text_area_key(state: &TextAreaState, key: &KeyEvent) -> Option<TextAreaState> {
    let name = key.name.as_str();
    if matches!(name, "tab" | "backtab" | "escape") {
        return None;
    }
    if name == "return" && key.ctrl {
        return None;
    }
    let cur = state.lines.get(state.row).cloned().unwrap_or_default();
    let cur_len = len(&cur);
    let col = state.col.min(cur_len);
    if name == "return" {
        let before = slice(&cur, 0, Some(col));
        let after = slice(&cur, col, None);
        let mut lines = state.lines[..state.row].to_vec();
        lines.push(before);
        lines.push(after);
        lines.extend_from_slice(&state.lines[state.row + 1..]);
        return Some(TextAreaState {
            lines,
            row: state.row + 1,
            col: 0,
        });
    }
    if name == "backspace" {
        if col == 0 {
            if state.row == 0 {
                return Some(state.clone());
            }
            let prev = state.lines[state.row - 1].clone();
            let merged_col = len(&prev);
            let mut lines = state.lines[..state.row - 1].to_vec();
            lines.push(format!("{prev}{cur}"));
            lines.extend_from_slice(&state.lines[state.row + 1..]);
            return Some(TextAreaState {
                lines,
                row: state.row - 1,
                col: merged_col,
            });
        }
        let next = format!("{}{}", slice(&cur, 0, Some(col - 1)), slice(&cur, col, None));
        return Some(state.with_line(next).at(state.row, col - 1));
    }
    if name == "delete" {
        if col >= cur_len {
            if state.row + 1 >= state.lines.len() {
                return Some(state.clone());
            }
            let mut lines = state.lines[..state.row].to_vec();
            lines.push(format!("{cur}{}", state.lines[state.row + 1]));
            lines.extend_from_slice(&state.lines[state.row + 2..]);
            return Some(TextAreaState {
                lines,
                row: state.row,
                col,
            });
        }
        let next = format!("{}{}", slice(&cur, 0, Some(col)), slice(&cur, col + 1, None));
        return Some(state.with_line(next).at(state.row, col));
    }
    if name == "left" {
        if key.alt {
            if col > 0 {
                return Some(state.at(state.row, prev_word_boundary(&cur, col)));
            }
            if state.row > 0 {
                return Some(state.at(state.row - 1, len(&state.lines[state.row - 1])));
            }
            return Some(state.clone());
        }
        if col > 0 {
            return Some(state.at(state.row, col - 1));
        }
        if state.row > 0 {
            return Some(state.at(state.row - 1, len(&state.lines[state.row - 1])));
        }
        return Some(state.clone());
    }
    if name == "right" {
        if key.alt {
            if col < cur_len {
                return Some(state.at(state.row, next_word_boundary(&cur, col)));
            }
            if state.row + 1 < state.lines.len() {
                return Some(state.at(state.row + 1, 0));
            }
            return Some(state.clone());
        }
        if col < cur_len {
            return Some(state.at(state.row, col + 1));
        }
        if state.row + 1 < state.lines.len() {
            return Some(state.at(state.row + 1, 0));
        }
        return Some(state.clone());
    }
    let ch = key.ch.as_deref();
    if key.alt && ch == Some("b") {
        return Some(state.at(state.row, prev_word_boundary(&cur, col)));
    }
    if key.alt && ch == Some("f") {
        return Some(state.at(state.row, next_word_boundary(&cur, col)));
    }
    if name == "up" {
        if state.row == 0 {
            return Some(state.clone());
        }
        let prev = len(&state.lines[state.row - 1]);
        return Some(state.at(state.row - 1, col.min(prev)));
    }
    if name == "down" {
        if state.row + 1 >= state.lines.len() {
            return Some(state.clone());
        }
        let next = len(&state.lines[state.row + 1]);
        return Some(state.at(state.row + 1, col.min(next)));
    }
    if name == "home" || (name == "a" && key.ctrl) {
        return Some(state.at(state.row, 0));
    }
    if name == "end" || (name == "e" && key.ctrl) {
        return Some(state.at(state.row, cur_len));
    }
    if let Some(ch) = ch
        && !ch.is_empty()
        && !key.ctrl
        && !key.alt
    {
        let next = format!("{}{ch}{}", slice(&cur, 0, Some(col)), slice(&cur, col, None));
        return Some(state.with_line(next).at(state.row, col + len(ch)));
    }
    None
}

/// One line per logical line; when `active` the cursor row paints the
/// character under the cursor inverted (`renderTextArea`).
pub fn render_text_area(theme: &Theme, state: &TextAreaState, active: bool) -> Vec<Line<'static>> {
    let primary = Style::default().fg(theme.color(Color::Primary));
    state
        .lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if !active || i != state.row {
                let text = if line.is_empty() { " ".to_string() } else { line.clone() };
                return Line::from(Span::styled(text, primary));
            }
            let col = state.col.min(len(line));
            let before = slice(line, 0, Some(col));
            let under = {
                let u = slice(line, col, Some(col + 1));
                if u.is_empty() { " ".to_string() } else { u }
            };
            let after = slice(line, col + 1, None);
            Line::from(vec![
                Span::styled(before, primary),
                Span::styled(under, primary.add_modifier(Modifier::REVERSED)),
                Span::styled(after, primary),
            ])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(n: &str) -> KeyEvent {
        KeyEvent::named(n)
    }

    fn type_in(initial: &TextAreaState, keys: &[KeyEvent]) -> TextAreaState {
        let mut s = initial.clone();
        for key in keys {
            if let Some(n) = apply_text_area_key(&s, key) {
                s = n;
            }
        }
        s
    }

    /// node: tests/widgets-text-area.test.ts:22-70
    #[test]
    fn basics() {
        let s = TextAreaState::new("");
        assert_eq!(s.lines, vec![""]);
        assert_eq!(s, TextAreaState { lines: vec![String::new()], row: 0, col: 0 });
        assert_eq!(s.text(), "");
        assert_eq!(TextAreaState::new("hello\nworld").lines, vec!["hello", "world"]);
        let s = type_in(&TextAreaState::new(""), &[KeyEvent::printable("h"), KeyEvent::printable("i")]);
        assert_eq!(s.text(), "hi");
        assert_eq!(s.col, 2);
        let s = type_in(
            &TextAreaState::new("hello world"),
            &[k("home"), k("right"), k("right"), k("right"), k("right"), k("right"), k("return")],
        );
        assert_eq!(s.lines, vec!["hello", " world"]);
        assert_eq!((s.row, s.col), (1, 0));
        let s = type_in(&TextAreaState::new("line1\nline2"), &[k("down"), k("backspace")]);
        assert_eq!(s.lines, vec!["line1line2"]);
        assert_eq!((s.row, s.col), (0, 5));
        let s = type_in(&TextAreaState::new("line1\nline2"), &[k("end"), k("delete")]);
        assert_eq!(s.lines, vec!["line1line2"]);
        assert_eq!((s.row, s.col), (0, 5));
    }

    /// node: tests/widgets-text-area.test.ts:72-102
    #[test]
    fn cursor_movement() {
        let s = type_in(&TextAreaState::new("ab\nxy"), &[k("down"), k("left")]);
        assert_eq!((s.row, s.col), (0, 2));
        let s = type_in(&TextAreaState::new("ab\nxy"), &[k("end"), k("right")]);
        assert_eq!((s.row, s.col), (1, 0));
        let s0 = type_in(&TextAreaState::new("longer\nshrt"), &[k("end")]);
        assert_eq!(s0.col, 6);
        let s1 = type_in(&s0, &[k("down")]);
        assert_eq!(s1.col, 4);
        assert_eq!(type_in(&s1, &[k("up")]).col, 4);
        let s = type_in(&TextAreaState::new("hello\nworld"), &[k("down"), k("end"), k("home")]);
        assert_eq!((s.row, s.col), (1, 0));
    }

    /// node: tests/widgets-text-area.test.ts:104-122
    #[test]
    fn passthrough_keys() {
        let s0 = TextAreaState::new("x");
        assert_eq!(apply_text_area_key(&s0, &k("tab")), None);
        assert_eq!(apply_text_area_key(&s0, &k("backtab")), None);
        assert_eq!(apply_text_area_key(&s0, &k("escape")), None);
        assert_eq!(apply_text_area_key(&s0, &k("return").with_ctrl()), None);
        assert_eq!(apply_text_area_key(&s0, &KeyEvent::printable("s").with_ctrl()), None);
        assert_eq!(apply_text_area_key(&s0, &KeyEvent::printable("q").with_alt()), None);
    }

    /// node: tests/widgets-text-area.test.ts:124-155
    #[test]
    fn rendering() {
        let t = crate::theme::COOL_BLUE;
        let s = TextAreaState::new("one\ntwo\nthree");
        assert_eq!(render_text_area(&t, &s, false).len(), 3);
        let s = TextAreaState {
            row: 1,
            col: 1,
            ..TextAreaState::new("abc\ndef")
        };
        let lines = render_text_area(&t, &s, true);
        assert_eq!(lines[0].spans.len(), 1);
        assert_eq!(lines[0].spans[0].content, "abc");
        assert_eq!(lines[1].spans.len(), 3);
        assert_eq!(lines[1].spans[0].content, "d");
        assert_eq!(lines[1].spans[1].content, "e");
        assert!(lines[1].spans[1].style.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(lines[1].spans[2].content, "f");
        let s = TextAreaState {
            row: 0,
            col: 3,
            ..TextAreaState::new("abc")
        };
        let lines = render_text_area(&t, &s, true);
        assert_eq!(lines[0].spans[0].content, "abc");
        assert_eq!(lines[0].spans[1].content, " ");
        assert_eq!(lines[0].spans[2].content, "");
    }
}
