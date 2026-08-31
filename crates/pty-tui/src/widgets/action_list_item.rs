//! Action list item (`src/tui/widgets/action-list-item.ts`): a 3-cell icon
//! chip (accent fill when focused, border fill otherwise) then the label
//! (bold when focused) and optional right-aligned text.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::text::text_width;
use crate::theme::{Color, Theme};

/// `ActionListItemOptions`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActionListItemOptions {
    pub icon: Option<String>,
    pub focused: bool,
    pub right: Option<String>,
}

/// The left spans (chip + label) and the optional right span.
pub fn action_list_item_spans(theme: &Theme, label: &str, opts: &ActionListItemOptions) -> (Vec<Span<'static>>, Option<Span<'static>>) {
    let chip = Span::styled(
        format!(" {} ", opts.icon.clone().unwrap_or_else(|| " ".into())),
        Style::default()
            .bg(theme.color(if opts.focused { Color::Accent } else { Color::Border }))
            .fg(theme.color(if opts.focused { Color::Primary } else { Color::Muted })),
    );
    let mut label_style = Style::default().fg(theme.color(Color::Primary));
    if opts.focused {
        label_style = label_style.add_modifier(Modifier::BOLD);
    }
    let label = Span::styled(format!(" {label}"), label_style);
    let right = opts
        .right
        .clone()
        .map(|r| Span::styled(r, Style::default().fg(theme.color(Color::Muted))));
    (vec![chip, label], right)
}

/// The row as a line for `width` columns (right text pushed to the end).
pub fn action_list_item(theme: &Theme, label: &str, opts: &ActionListItemOptions, width: usize) -> Line<'static> {
    let (mut spans, right) = action_list_item_spans(theme, label, opts);
    if let Some(r) = right {
        let used: usize = spans.iter().map(|s| text_width(&s.content)).sum::<usize>() + text_width(&r.content);
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
        spans.push(r);
    }
    Line::from(spans)
}

/// The row as a widget.
pub struct ActionListItem<'a> {
    pub theme: Theme,
    pub label: &'a str,
    pub opts: ActionListItemOptions,
}

impl Widget for ActionListItem<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let line = action_list_item(&self.theme, self.label, &self.opts, area.width as usize);
        buf.set_line(area.x, area.y, &line, area.width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/action-list-item.test.ts:6-31
    #[test]
    fn node_contract() {
        let t = crate::theme::COOL_BLUE;
        let (spans, right) = action_list_item_spans(
            &t,
            "Deploy",
            &ActionListItemOptions {
                icon: Some("▶".into()),
                ..Default::default()
            },
        );
        assert_eq!(spans[0].content, " ▶ ");
        assert_eq!(spans[0].style.bg, Some(t.color(Color::Border)));
        assert_eq!(spans[1].content, " Deploy");
        assert!(right.is_none());
        let (spans, _) = action_list_item_spans(&t, "x", &ActionListItemOptions::default());
        assert_eq!(spans[0].content, "   ");
        let (spans, _) = action_list_item_spans(
            &t,
            "Deploy",
            &ActionListItemOptions {
                icon: Some("▶".into()),
                focused: true,
                right: None,
            },
        );
        assert_eq!(spans[0].style.bg, Some(t.color(Color::Accent)));
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
        let line = action_list_item(
            &t,
            "Session",
            &ActionListItemOptions {
                right: Some("3m".into()),
                ..Default::default()
            },
            20,
        );
        assert_eq!(line.spans.len(), 4);
        assert_eq!(line.spans[3].content, "3m");
        assert_eq!(line.to_string().chars().count(), 20);
        assert!(line.to_string().ends_with("3m"));
    }

    /// node: tests/action-list-item.test.ts:34-45
    #[test]
    fn rendered() {
        let t = crate::theme::COOL_BLUE;
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 1));
        ActionListItem {
            theme: t,
            label: "Deploy",
            opts: ActionListItemOptions {
                icon: Some("▶".into()),
                focused: true,
                right: None,
            },
        }
        .render(buf.area, &mut buf);
        assert_eq!(buf[(1, 0)].symbol(), "▶");
        assert_eq!(buf[(1, 0)].bg, ratatui::style::Color::Rgb(100, 160, 255));
        let row: String = (0..30).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(row.contains("Deploy"));
    }
}
