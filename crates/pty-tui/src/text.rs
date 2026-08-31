//! Width-aware text helpers ported from `src/tui/colors.ts:35-200`:
//! Node's `charWidth` table, `visibleLength`, `truncate` with an ellipsis,
//! `wrapText` with code-point offsets, and `pad`.

/// Display width of one character (`charWidth`, `colors.ts:35-79`).
pub fn char_width(ch: char) -> usize {
    let code = ch as u32;
    if code < 0x20 {
        return 0;
    }
    if code < 0x2500 {
        return 1;
    }
    if (0x2500..=0x25ff).contains(&code) {
        return 1;
    }
    if (0x2600..=0x26ff).contains(&code) {
        return match code {
            0x2614..=0x2615
            | 0x2648..=0x2653
            | 0x267f
            | 0x2693
            | 0x26a0..=0x26a1
            | 0x26aa..=0x26ab
            | 0x26bd..=0x26be
            | 0x26c4..=0x26c5
            | 0x26d4
            | 0x26ea
            | 0x26f2..=0x26f3
            | 0x26f5
            | 0x26fa
            | 0x26fd => 2,
            _ => 1,
        };
    }
    if (0x2700..=0x27bf).contains(&code) {
        return 1;
    }
    if (0x2e80..=0x9fff).contains(&code)
        || (0xac00..=0xd7af).contains(&code)
        || (0xf900..=0xfaff).contains(&code)
        || (0xfe10..=0xfe6f).contains(&code)
        || (0xff01..=0xff60).contains(&code)
        || (0xffe0..=0xffe6).contains(&code)
        || (0x1f000..=0x1fbff).contains(&code)
        || (0x20000..=0x3ffff).contains(&code)
    {
        return 2;
    }
    1
}

/// Strip CSI sequences (`stripAnsi`).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Display width of a string, ANSI stripped (`visibleLength`).
pub fn visible_width(s: &str) -> usize {
    strip_ansi(s).chars().map(char_width).sum()
}

/// Display width of a plain string (`textWidth`).
pub fn text_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// Truncate to `max_width` cells with a trailing `…` (`truncate`,
/// `colors.ts:91-104`).
pub fn truncate(text: &str, max_width: usize) -> String {
    if visible_width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 1 {
        return text.chars().take(max_width).collect();
    }
    let mut w = 0;
    let mut out = String::new();
    for ch in text.chars() {
        let cw = char_width(ch);
        if w + cw + 1 > max_width {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('\u{2026}');
    out
}

/// Soft-wrap into lines of at most `max_width` cells, breaking before a
/// space or after a wide character, falling back to a character break;
/// returns the lines and the code-point offset each starts at (`wrapText`,
/// `colors.ts:121-183`). Concatenating the lines reproduces the text.
pub fn wrap_text(text: &str, max_width: usize) -> (Vec<String>, Vec<usize>) {
    if max_width == 0 {
        return (vec![text.to_string()], vec![0]);
    }
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut lines = Vec::new();
    let mut offsets = Vec::new();
    let mut pos = 0;
    while pos < len {
        let mut line_width = 0;
        let mut line_end = pos;
        let mut last_break: Option<usize> = None;
        while line_end < len {
            let ch = chars[line_end];
            let w = char_width(ch);
            if line_width + w > max_width {
                break;
            }
            if ch == ' ' && line_end > pos {
                last_break = Some(line_end);
            }
            line_width += w;
            line_end += 1;
            if w >= 2 {
                last_break = Some(line_end);
            }
        }
        if line_end >= len {
            lines.push(chars[pos..len].iter().collect());
            offsets.push(pos);
            break;
        }
        match last_break {
            Some(b) if b > pos => {
                lines.push(chars[pos..b].iter().collect());
                offsets.push(pos);
                pos = b;
            }
            _ => {
                if line_end == pos {
                    line_end += 1;
                }
                lines.push(chars[pos..line_end].iter().collect());
                offsets.push(pos);
                pos = line_end;
            }
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
        offsets.push(0);
    }
    (lines, offsets)
}

/// Text alignment for [`pad`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Right,
    Center,
}

/// Pad to `width` cells (`pad`, `colors.ts:185-192`).
pub fn pad(text: &str, width: usize, align: Align) -> String {
    let len = visible_width(text);
    if len >= width {
        return text.to_string();
    }
    let p = width - len;
    match align {
        Align::Right => format!("{}{text}", " ".repeat(p)),
        Align::Center => format!("{}{text}{}", " ".repeat(p / 2), " ".repeat(p.div_ceil(2))),
        Align::Left => format!("{text}{}", " ".repeat(p)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/tui-framework.test.ts (textWidth / wrapText cases)
    #[test]
    fn widths() {
        assert_eq!(text_width("abc"), 3);
        assert_eq!(text_width("日本"), 4);
        assert_eq!(text_width("╭─╮"), 3);
        assert_eq!(text_width("⚡"), 2);
        assert_eq!(visible_width("\x1b[31mred\x1b[0m"), 3);
    }

    #[test]
    fn truncate_with_ellipsis() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello w…");
        assert_eq!(truncate("hello", 1), "h");
        assert_eq!(truncate("hello", 0), "");
    }

    #[test]
    fn wrap_breaks_before_spaces_and_after_wide() {
        let (lines, offsets) = wrap_text("hello world foo", 7);
        assert_eq!(lines, vec!["hello", " world", " foo"]);
        assert_eq!(offsets, vec![0, 5, 11]);
        let (lines, _) = wrap_text("abcdefghij", 4);
        assert_eq!(lines, vec!["abcd", "efgh", "ij"]);
        let (lines, _) = wrap_text("日本語", 4);
        assert_eq!(lines, vec!["日本", "語"]);
        assert_eq!(wrap_text("", 5).0, vec![""]);
        assert_eq!(wrap_text("x", 0).0, vec!["x"]);
    }

    #[test]
    fn padding() {
        assert_eq!(pad("ab", 5, Align::Left), "ab   ");
        assert_eq!(pad("ab", 5, Align::Right), "   ab");
        assert_eq!(pad("ab", 5, Align::Center), " ab  ");
        assert_eq!(pad("abcdef", 3, Align::Left), "abcdef");
    }
}
