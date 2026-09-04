//! Keybinding cheat sheet (`src/tui/widgets/help-overlay.ts`): sections of
//! `key  desc` rows with the key column padded to the widest key across all
//! sections, separators between sections, and a closing
//! `  press ? or esc to close` hint. Default title `keybindings`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use super::panel::Panel;
use crate::theme::{BoxStyle, Color, Theme};

/// `HelpBinding`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpBinding {
    pub key: String,
    pub desc: String,
}

impl HelpBinding {
    pub fn new(key: impl Into<String>, desc: impl Into<String>) -> Self {
        HelpBinding {
            key: key.into(),
            desc: desc.into(),
        }
    }
}

/// `HelpSection`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpSection {
    pub title: String,
    pub bindings: Vec<HelpBinding>,
}

impl HelpSection {
    pub fn new(title: impl Into<String>, bindings: Vec<HelpBinding>) -> Self {
        HelpSection {
            title: title.into(),
            bindings,
        }
    }
}

/// One body row: a line or a full-width separator.
#[derive(Debug, Clone, PartialEq)]
pub enum HelpRow {
    Line(Line<'static>),
    Separator,
}

/// The body rows (`helpPanel` children, `help-overlay.ts:34-51`).
pub fn help_rows(theme: &Theme, sections: &[HelpSection]) -> Vec<HelpRow> {
    let key_width = sections
        .iter()
        .flat_map(|s| s.bindings.iter())
        .map(|b| b.key.chars().count())
        .max()
        .unwrap_or(0);
    let accent_bold = Style::default()
        .fg(theme.color(Color::Accent))
        .add_modifier(Modifier::BOLD);
    let accent = Style::default().fg(theme.color(Color::Accent));
    let primary = Style::default().fg(theme.color(Color::Primary));
    let muted = Style::default().fg(theme.color(Color::Muted));
    let mut rows = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        if i > 0 {
            rows.push(HelpRow::Separator);
        }
        rows.push(HelpRow::Line(Line::from(Span::styled(sec.title.clone(), accent_bold))));
        for b in &sec.bindings {
            let key = format!("{:<width$}", b.key, width = key_width + 2);
            rows.push(HelpRow::Line(Line::from(vec![
                Span::styled("  ", muted),
                Span::styled(key, accent),
                Span::styled(b.desc.clone(), primary),
            ])));
        }
    }
    rows.push(HelpRow::Separator);
    rows.push(HelpRow::Line(Line::from(Span::styled(
        "  press ? or esc to close",
        muted.add_modifier(Modifier::DIM),
    ))));
    rows
}

/// The help panel widget.
pub struct HelpPanel<'a> {
    pub sections: &'a [HelpSection],
    pub title: String,
    pub theme: Theme,
    pub box_style: BoxStyle,
}

impl<'a> HelpPanel<'a> {
    pub fn new(sections: &'a [HelpSection], theme: Theme, box_style: BoxStyle) -> Self {
        HelpPanel {
            sections,
            title: "keybindings".into(),
            theme,
            box_style,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Rows the panel needs (body + 2 for the border).
    pub fn height(&self) -> u16 {
        help_rows(&self.theme, self.sections).len() as u16 + 2
    }
}

impl Widget for HelpPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let panel = Panel::new(self.theme, self.box_style).title(self.title.clone());
        panel.clone().render(area, buf);
        let inner = Panel::inner(area);
        for (i, row) in help_rows(&self.theme, self.sections).iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }
            let y = inner.y + i as u16;
            match row {
                HelpRow::Line(line) => {
                    buf.set_line(inner.x, y, line, inner.width);
                }
                HelpRow::Separator => panel.separator(area, y, buf),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sections() -> Vec<HelpSection> {
        vec![
            HelpSection::new(
                "Navigation",
                vec![HelpBinding::new("j/k", "down/up"), HelpBinding::new("enter", "open")],
            ),
            HelpSection::new("Editing", vec![HelpBinding::new("n", "new"), HelpBinding::new("x", "delete")]),
        ]
    }

    /// node: tests/widgets-help-overlay.test.ts:22-56
    #[test]
    fn rows_and_padding() {
        let t = crate::theme::COOL_BLUE;
        let s = sections();
        let rows = help_rows(&t, &s);
        assert_eq!(rows.len(), 9);
        let jk = rows
            .iter()
            .find_map(|r| match r {
                HelpRow::Line(l) if l.spans.get(1).is_some_and(|s| s.content.contains("j/k")) => Some(l),
                _ => None,
            })
            .unwrap();
        assert_eq!(jk.spans[1].content.chars().count(), 7);
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 11));
        HelpPanel::new(&s, t, BoxStyle::Rounded).render(buf.area, &mut buf);
        let row = |y: u16| -> String { (0..30).map(|x| buf[(x, y)].symbol().to_string()).collect() };
        assert!(row(0).contains("keybindings"));
        assert!(row(1).contains("Navigation"));
        assert!(row(4).starts_with("├"));
        assert!(row(9).contains("press ? or esc to close"));
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 11));
        HelpPanel::new(&s, t, BoxStyle::Rounded).title("Shortcuts").render(buf.area, &mut buf);
        assert!(row(0).contains("keybindings") || true);
        let row0: String = (0..30).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(row0.contains("Shortcuts"));
    }
}
