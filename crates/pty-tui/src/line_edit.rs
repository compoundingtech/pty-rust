//! Readline-style single-line editing, ported from
//! `src/tui/widgets/form.ts:15-155`: a text with a cursor (in characters),
//! `apply_text_key` for the editing keys, word motion, and a renderer that
//! paints the cursor as an inverse cell on top of the character under it.
//!
//! Keys: backspace, delete, left/right (alt = word), alt+b / alt+f, home /
//! ctrl+a, end / ctrl+e, ctrl+u (clear to start), ctrl+w (delete word
//! behind), ctrl+k (kill to end), printable (no ctrl/alt) inserts.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::input::KeyEvent;

/// `TextFieldState` (`form.ts:15-18`). `cursor` counts characters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextFieldState {
    pub text: String,
    pub cursor: usize,
}

impl TextFieldState {
    /// A field with the cursor at the end.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        TextFieldState { text, cursor }
    }

    /// The empty field.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Number of characters.
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn chars(&self) -> Vec<char> {
        self.text.chars().collect()
    }

    fn from_chars(chars: &[char], cursor: usize) -> Self {
        TextFieldState {
            text: chars.iter().collect(),
            cursor,
        }
    }
}

/// `\p{L}\p{N}_` (`form.ts:23-25`).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The start of the previous word from `pos` (`prevWordBoundary`,
/// `form.ts:30-37`): skips non-word characters, then the word.
pub fn prev_word_boundary(text: &str, pos: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut i = pos.min(chars.len());
    while i > 0 && !is_word_char(chars[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word_char(chars[i - 1]) {
        i -= 1;
    }
    i
}

/// One past the end of the next word from `pos` (`nextWordBoundary`,
/// `form.ts:40-45`).
pub fn next_word_boundary(text: &str, pos: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut i = pos.min(chars.len());
    while i < chars.len() && !is_word_char(chars[i]) {
        i += 1;
    }
    while i < chars.len() && is_word_char(chars[i]) {
        i += 1;
    }
    i
}

/// Apply a key to a field. `None` when the key is not an editing key, so
/// the caller can dispatch it elsewhere (`applyTextKey`, `form.ts:50-118`).
pub fn apply_text_key(state: &TextFieldState, key: &KeyEvent) -> Option<TextFieldState> {
    let chars = state.chars();
    let len = chars.len();
    let cursor = state.cursor.min(len);
    let name = key.name.as_str();
    match name {
        "backspace" => {
            if cursor == 0 {
                return Some(state.clone());
            }
            let mut c = chars;
            c.remove(cursor - 1);
            return Some(TextFieldState::from_chars(&c, cursor - 1));
        }
        "delete" => {
            if cursor >= len {
                return Some(state.clone());
            }
            let mut c = chars;
            c.remove(cursor);
            return Some(TextFieldState::from_chars(&c, cursor));
        }
        "left" => {
            if key.alt {
                return Some(TextFieldState {
                    cursor: prev_word_boundary(&state.text, cursor),
                    ..state.clone()
                });
            }
            if cursor == 0 {
                return Some(state.clone());
            }
            return Some(TextFieldState {
                cursor: cursor - 1,
                ..state.clone()
            });
        }
        "right" => {
            if key.alt {
                return Some(TextFieldState {
                    cursor: next_word_boundary(&state.text, cursor),
                    ..state.clone()
                });
            }
            if cursor >= len {
                return Some(state.clone());
            }
            return Some(TextFieldState {
                cursor: cursor + 1,
                ..state.clone()
            });
        }
        _ => {}
    }
    let ch = key.ch.as_deref();
    if key.alt && ch == Some("b") {
        return Some(TextFieldState {
            cursor: prev_word_boundary(&state.text, cursor),
            ..state.clone()
        });
    }
    if key.alt && ch == Some("f") {
        return Some(TextFieldState {
            cursor: next_word_boundary(&state.text, cursor),
            ..state.clone()
        });
    }
    if name == "home" || (name == "a" && key.ctrl) {
        return Some(TextFieldState {
            cursor: 0,
            ..state.clone()
        });
    }
    if name == "end" || (name == "e" && key.ctrl) {
        return Some(TextFieldState {
            cursor: len,
            ..state.clone()
        });
    }
    if name == "u" && key.ctrl {
        return Some(TextFieldState::from_chars(&chars[cursor..], 0));
    }
    if name == "w" && key.ctrl {
        let start = prev_word_boundary(&state.text, cursor);
        let mut c = chars[..start].to_vec();
        c.extend_from_slice(&chars[cursor..]);
        return Some(TextFieldState::from_chars(&c, start));
    }
    if name == "k" && key.ctrl {
        return Some(TextFieldState::from_chars(&chars[..cursor], cursor));
    }
    if let Some(ch) = ch
        && !ch.is_empty()
        && !key.ctrl
        && !key.alt
    {
        let mut c = chars[..cursor].to_vec();
        let inserted: Vec<char> = ch.chars().collect();
        c.extend_from_slice(&inserted);
        c.extend_from_slice(&chars[cursor..]);
        return Some(TextFieldState::from_chars(&c, cursor + inserted.len()));
    }
    None
}

/// The legacy string form: a `█` inserted at the cursor when active
/// (`renderFieldText`, `form.ts:126-131`).
pub fn render_field_text(text: &str, cursor: usize, active: bool) -> String {
    if !active {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let before: String = chars[..cursor].iter().collect();
    let after: String = chars[cursor..].iter().collect();
    format!("{before}\u{2588}{after}")
}

/// The field as spans: inactive = one span; active = before / the character
/// under the cursor inverted (a space at the end) / after
/// (`renderFieldNodes`, `form.ts:136-155`).
pub fn render_field_spans(
    text: &str,
    cursor: usize,
    active: bool,
    style: Style,
) -> Vec<Span<'static>> {
    if !active {
        return vec![Span::styled(text.to_string(), style)];
    }
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let before: String = chars[..cursor].iter().collect();
    let under: String = chars
        .get(cursor)
        .map(|c| c.to_string())
        .unwrap_or_else(|| " ".to_string());
    let after: String = chars[(cursor + 1).min(chars.len())..].iter().collect();
    vec![
        Span::styled(before, style),
        Span::styled(under, style.add_modifier(Modifier::REVERSED)),
        Span::styled(after, style),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(name: &str) -> KeyEvent {
        KeyEvent::named(name)
    }
    fn p(ch: &str) -> KeyEvent {
        KeyEvent::printable(ch)
    }
    fn ctrl(ch: &str) -> KeyEvent {
        KeyEvent::ctrl(ch)
    }
    fn alt(ch: &str) -> KeyEvent {
        KeyEvent::alt(ch)
    }
    fn st(text: &str, cursor: usize) -> TextFieldState {
        TextFieldState {
            text: text.into(),
            cursor,
        }
    }

    /// node: tests/widgets-form.test.ts:14-108
    #[test]
    fn apply_text_key_edits() {
        let empty = st("", 0);
        let hello = st("hello", 5);
        assert_eq!(apply_text_key(&hello, &p("!")), Some(st("hello!", 6)));
        assert_eq!(apply_text_key(&hello, &k("backspace")), Some(st("hell", 4)));
        assert_eq!(apply_text_key(&st("hello", 2), &k("delete")), Some(st("helo", 2)));
        assert_eq!(apply_text_key(&hello, &k("left")), Some(st("hello", 4)));
        assert_eq!(apply_text_key(&empty, &k("left")), Some(empty.clone()));
        assert_eq!(apply_text_key(&empty, &k("right")), Some(empty.clone()));
        assert_eq!(apply_text_key(&hello, &k("home")), Some(st("hello", 0)));
        assert_eq!(apply_text_key(&st("hello", 0), &k("end")), Some(st("hello", 5)));
        assert_eq!(apply_text_key(&st("hello", 3), &ctrl("u")), Some(st("lo", 0)));
        assert_eq!(apply_text_key(&hello, &ctrl("a")), Some(st("hello", 0)));
        assert_eq!(apply_text_key(&st("hello", 1), &ctrl("e")), Some(st("hello", 5)));
        assert_eq!(apply_text_key(&st("git status", 10), &ctrl("w")), Some(st("git ", 4)));
        assert_eq!(
            apply_text_key(&st("hello worldxxx", 11), &ctrl("w")),
            Some(st("hello xxx", 6))
        );
        assert_eq!(apply_text_key(&st("foo   bar", 9), &ctrl("w")), Some(st("foo   ", 6)));
        assert_eq!(apply_text_key(&empty, &ctrl("w")), Some(st("", 0)));
        assert_eq!(apply_text_key(&st("hello world", 5), &ctrl("k")), Some(st("hello", 5)));
        assert_eq!(apply_text_key(&hello, &ctrl("k")), Some(st("hello", 5)));
        assert_eq!(apply_text_key(&hello, &ctrl("s")), None);
        assert_eq!(apply_text_key(&hello, &alt("q")), None);
    }

    /// node: tests/widgets-form.test.ts:110-137
    #[test]
    fn render_field() {
        assert_eq!(render_field_text("foo", 1, false), "foo");
        assert_eq!(render_field_text("foo", 1, true), "f\u{2588}oo");
        assert_eq!(render_field_text("foo", 3, true), "foo\u{2588}");
        let s = render_field_spans("hello", 2, false, Style::default());
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].content, "hello");
        let s = render_field_spans("hello", 2, true, Style::default());
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].content, "he");
        assert_eq!(s[1].content, "l");
        assert!(s[1].style.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(s[2].content, "lo");
        let s = render_field_spans("hi", 2, true, Style::default());
        assert_eq!(s[1].content, " ");
        assert!(s[1].style.add_modifier.contains(Modifier::REVERSED));
    }

    /// node: tests/widgets-form.test.ts:139-157
    #[test]
    fn word_boundaries() {
        assert_eq!(prev_word_boundary("hello  world", 12), 7);
        assert_eq!(prev_word_boundary("hello world", 6), 0);
        assert_eq!(next_word_boundary("hello world", 0), 5);
        assert_eq!(next_word_boundary("hello world", 5), 11);
        assert_eq!(prev_word_boundary("   abc", 6), 3);
        assert_eq!(next_word_boundary("abc   ", 0), 3);
        assert_eq!(next_word_boundary("abc   ", 3), 6);
    }

    /// node: tests/widgets-form.test.ts:159-177
    #[test]
    fn word_motion() {
        let state = st("hello there friend", 11);
        assert_eq!(apply_text_key(&state, &KeyEvent::named("left").with_alt()).unwrap().cursor, 6);
        assert_eq!(apply_text_key(&state, &KeyEvent::named("right").with_alt()).unwrap().cursor, 18);
        assert_eq!(apply_text_key(&state, &alt("b")).unwrap().cursor, 6);
        assert_eq!(apply_text_key(&state, &alt("f")).unwrap().cursor, 18);
    }

    #[test]
    fn multibyte_text_counts_characters() {
        let s = TextFieldState::new("héllo");
        assert_eq!(s.cursor, 5);
        assert_eq!(apply_text_key(&s, &k("backspace")), Some(st("héll", 4)));
        assert_eq!(apply_text_key(&st("héllo", 1), &k("delete")), Some(st("hllo", 1)));
    }
}
