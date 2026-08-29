//! Message bubble (`src/tui/widgets/message.ts`): each content line padded
//! on a fill — incoming on the border fill, left-aligned; outgoing on the
//! accent fill, right-aligned — with an optional bold muted sender label
//! above.

use ratatui::layout::Alignment;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{Color, Theme};

/// `MessageOptions`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MessageOptions {
    pub outgoing: bool,
    pub from: Option<String>,
}

/// `message` as lines (right-aligned when outgoing).
pub fn message(theme: &Theme, content: &str, opts: &MessageOptions) -> Vec<Line<'static>> {
    let bubble_bg = if opts.outgoing { Color::Accent } else { Color::Border };
    let align = if opts.outgoing { Alignment::Right } else { Alignment::Left };
    let mut lines = Vec::new();
    if let Some(from) = &opts.from {
        lines.push(
            Line::from(Span::styled(
                from.clone(),
                Style::default()
                    .fg(theme.color(Color::Muted))
                    .add_modifier(Modifier::BOLD),
            ))
            .alignment(align),
        );
    }
    for line in content.split('\n') {
        lines.push(
            Line::from(Span::styled(
                format!(" {line} "),
                Style::default()
                    .fg(theme.color(Color::Primary))
                    .bg(theme.color(bubble_bg)),
            ))
            .alignment(align),
        );
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: src/tui/widgets/message.ts:27-41
    #[test]
    fn bubbles() {
        let t = crate::theme::COOL_BLUE;
        let l = message(&t, "hi\nthere", &MessageOptions::default());
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].spans[0].content, " hi ");
        assert_eq!(l[0].spans[0].style.bg, Some(t.color(Color::Border)));
        assert_eq!(l[0].alignment, Some(Alignment::Left));
        let l = message(
            &t,
            "yo",
            &MessageOptions {
                outgoing: true,
                from: Some("me".into()),
            },
        );
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].spans[0].content, "me");
        assert_eq!(l[1].spans[0].style.bg, Some(t.color(Color::Accent)));
        assert_eq!(l[1].alignment, Some(Alignment::Right));
        let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 10, 2));
        ratatui::widgets::Widget::render(ratatui::widgets::Paragraph::new(l), buf.area, &mut buf);
        let row1: String = (0..10).map(|x| buf[(x, 1)].symbol().to_string()).collect();
        assert_eq!(row1, "       yo ");
    }
}
