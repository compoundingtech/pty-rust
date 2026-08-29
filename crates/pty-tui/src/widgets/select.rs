//! Dropdown select (`src/tui/widgets/select.ts`): a caret + value button
//! and, while open, the option list with `› ` on the highlight. Keys:
//! closed — `return`/`down` open; open — `up`/`down` move the highlight,
//! `return` commits and closes, `escape` closes.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::input::KeyEvent;
use crate::theme::{Color, Theme};

/// `SelectState` (`select.ts:20-24`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectState {
    pub open: bool,
    /// The highlighted option while open.
    pub index: usize,
}

impl SelectState {
    /// `createSelectState`.
    pub fn new(index: usize) -> Self {
        SelectState { open: false, index }
    }
}

/// Render options (`SelectOptions`, `select.ts:30-39`).
#[derive(Debug, Clone, Default)]
pub struct SelectOptions {
    pub placeholder: Option<String>,
    pub focused: bool,
    pub open_caret: Option<String>,
    pub closed_caret: Option<String>,
}

/// The button line plus, when open, one line per option
/// (`renderSelect`, `select.ts:43-73`).
pub fn render_select(
    theme: &Theme,
    options: &[String],
    selected_index: Option<usize>,
    state: SelectState,
    opts: &SelectOptions,
) -> Vec<Line<'static>> {
    let value = selected_index
        .and_then(|i| options.get(i).cloned())
        .or_else(|| opts.placeholder.clone())
        .unwrap_or_else(|| "(none)".to_string());
    let caret = if state.open {
        opts.open_caret.clone().unwrap_or_else(|| "\u{25be}".into())
    } else {
        opts.closed_caret.clone().unwrap_or_else(|| "\u{25b8}".into())
    };
    let caret_style = Style::default().fg(theme.color(if opts.focused {
        Color::Accent
    } else {
        Color::Muted
    }));
    let mut value_style = Style::default().fg(theme.color(if opts.focused {
        Color::Accent
    } else {
        Color::Primary
    }));
    if opts.focused {
        value_style = value_style.add_modifier(Modifier::BOLD);
    }
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{caret} "), caret_style),
        Span::styled(value, value_style),
    ])];
    if state.open {
        for (i, opt) in options.iter().enumerate() {
            let active = i == state.index;
            let mut style = Style::default().fg(theme.color(if active {
                Color::Accent
            } else {
                Color::Secondary
            }));
            if active {
                style = style.add_modifier(Modifier::BOLD);
            }
            let label = if active {
                format!("\u{203a} {opt}")
            } else {
                format!("  {opt}")
            };
            lines.push(Line::from(vec![Span::raw("  "), Span::styled(label, style)]));
        }
    }
    lines
}

/// `handleSelectKey` (`select.ts:82-105`): the new state and, on commit,
/// the chosen index.
pub fn handle_select_key(state: SelectState, options_len: usize, key: &KeyEvent) -> (SelectState, Option<usize>) {
    if !state.open {
        if key.name == "return" || key.name == "down" {
            return (
                SelectState {
                    open: true,
                    index: state.index,
                },
                None,
            );
        }
        return (state, None);
    }
    match key.name.as_str() {
        "up" => (
            SelectState {
                index: state.index.saturating_sub(1),
                ..state
            },
            None,
        ),
        "down" => (
            SelectState {
                index: (state.index + 1).min(options_len.saturating_sub(1)),
                ..state
            },
            None,
        ),
        "return" => (
            SelectState {
                open: false,
                index: state.index,
            },
            Some(state.index),
        ),
        "escape" => (SelectState { open: false, ..state }, None),
        _ => (state, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> Vec<String> {
        ["alpha", "beta", "gamma"].map(String::from).to_vec()
    }
    fn k(n: &str) -> KeyEvent {
        KeyEvent::named(n)
    }

    /// node: tests/select.test.ts:11-41
    #[test]
    fn reducer() {
        assert_eq!(SelectState::new(1), SelectState { open: false, index: 1 });
        assert!(handle_select_key(SelectState::new(1), 3, &k("return")).0.open);
        assert!(handle_select_key(SelectState::new(0), 3, &k("down")).0.open);
        let mut s = SelectState { open: true, index: 0 };
        s = handle_select_key(s, 3, &k("down")).0;
        assert_eq!(s.index, 1);
        s = handle_select_key(s, 3, &k("down")).0;
        assert_eq!(s.index, 2);
        s = handle_select_key(s, 3, &k("down")).0;
        assert_eq!(s.index, 2);
        s = handle_select_key(s, 3, &k("up")).0;
        assert_eq!(s.index, 1);
        let (st, chosen) = handle_select_key(SelectState { open: true, index: 2 }, 3, &k("return"));
        assert_eq!(chosen, Some(2));
        assert!(!st.open);
        let (st, chosen) = handle_select_key(SelectState { open: true, index: 2 }, 3, &k("escape"));
        assert_eq!(chosen, None);
        assert!(!st.open);
    }

    /// node: tests/select.test.ts:43-76
    #[test]
    fn render() {
        let t = crate::theme::COOL_BLUE;
        let lines = render_select(&t, &opts(), Some(1), SelectState::new(1), &SelectOptions::default());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "▸ ");
        assert_eq!(lines[0].spans[1].content, "beta");
        let o = SelectOptions {
            placeholder: Some("pick…".into()),
            ..Default::default()
        };
        let lines = render_select(&t, &opts(), None, SelectState::new(0), &o);
        assert_eq!(lines[0].spans[1].content, "pick…");
        let lines = render_select(&t, &opts(), Some(0), SelectState { open: true, index: 1 }, &SelectOptions::default());
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[2].spans[1].content, "› beta");
        assert_eq!(lines[2].spans[1].style.fg, Some(t.color(Color::Accent)));
        assert_eq!(lines[0].to_string(), "▾ alpha");
        let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 24, 8));
        ratatui::widgets::Widget::render(ratatui::widgets::Paragraph::new(lines), buf.area, &mut buf);
        let row = |y: u16| -> String { (0..24).map(|x| buf[(x, y)].symbol().to_string()).collect() };
        assert!(row(0).contains("▾ alpha"));
        assert!(row(2).contains("› beta"));
    }
}
