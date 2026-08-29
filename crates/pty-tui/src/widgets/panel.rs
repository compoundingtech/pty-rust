//! The panel: a bordered, `bg2`-filled box with a bold accent title on the
//! top border and an optional caption on the bottom border
//! (`panel(...)`, `builders.ts:157-166`; render `screen.ts:594-620`; layout
//! `layout.ts:299-344`). Content is inset two columns left and right and
//! one row top and bottom.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;

use crate::pane::draw_box;
use crate::text::text_width;
use crate::theme::{BoxStyle, Theme, to_ratatui};

/// The panel chrome.
#[derive(Debug, Clone)]
pub struct Panel {
    pub title: Option<String>,
    pub footer_title: Option<String>,
    pub theme: Theme,
    pub box_style: BoxStyle,
}

impl Panel {
    pub fn new(theme: Theme, box_style: BoxStyle) -> Self {
        Panel {
            title: None,
            footer_title: None,
            theme,
            box_style,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn footer_title(mut self, title: impl Into<String>) -> Self {
        self.footer_title = Some(title.into());
        self
    }

    /// The content rect: inset 2 columns and 1 row (`layoutPanel`).
    pub fn inner(area: Rect) -> Rect {
        Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(4),
            height: area.height.saturating_sub(2),
        }
    }

    /// Draw a separator on row `y` spanning the full panel width, joining
    /// the side borders (`separator()`, `layout.ts:331-334`).
    pub fn separator(&self, area: Rect, y: u16, buf: &mut Buffer) {
        if area.width < 2 || y < area.y || y >= area.y + area.height {
            return;
        }
        let (lj, rj) = self.box_style.junctions();
        let line = format!(
            "{lj}{}{rj}",
            self.box_style.horizontal().repeat(area.width as usize - 2)
        );
        let style = Style::default()
            .fg(to_ratatui(self.theme.border))
            .bg(to_ratatui(self.theme.bg2));
        buf.set_stringn(area.x, y, &line, area.width as usize, style);
    }

    fn caption(&self, buf: &mut Buffer, x: u16, y: u16, text: &str, max: u16) {
        let bg = to_ratatui(self.theme.bg2);
        let border = Style::default().fg(to_ratatui(self.theme.border)).bg(bg);
        let accent = Style::default()
            .fg(to_ratatui(self.theme.fg_ac))
            .bg(bg)
            .add_modifier(Modifier::BOLD);
        let w = text_width(text) as u16;
        if max < 3 {
            return;
        }
        buf.set_stringn(x, y, " ", 1, border);
        buf.set_stringn(x + 1, y, text, (max - 2) as usize, accent);
        let end = x + 1 + w.min(max - 2);
        if end < x + max {
            buf.set_stringn(end, y, " ", 1, border);
        }
    }
}

impl Widget for Panel {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let bg = to_ratatui(self.theme.bg2);
        buf.set_style(area, Style::default().bg(bg));
        let border = self.theme.border.unwrap_or((0, 0, 0));
        draw_box(buf, area, self.box_style, None, border);
        if self.theme.border.is_none() {
            // draw_box needs a colour; without one the border is default.
            for y in area.y..area.y + area.height {
                for x in area.x..area.x + area.width {
                    if y == area.y
                        || y == area.y + area.height - 1
                        || x == area.x
                        || x == area.x + area.width - 1
                    {
                        buf[(x, y)].set_fg(ratatui::style::Color::Reset);
                    }
                }
            }
        }
        buf.set_style(
            Rect::new(area.x, area.y, area.width, 1),
            Style::default().bg(bg),
        );
        if area.width > 4 {
            if let Some(title) = &self.title {
                self.caption(buf, area.x + 2, area.y, title, area.width - 4);
            }
            if let Some(footer) = &self.footer_title
                && area.height >= 2
            {
                self.caption(buf, area.x + 2, area.y + area.height - 1, footer, area.width - 4);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::COOL_BLUE;

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect()
    }

    /// node: tests/panel-footer-title.test.ts
    #[test]
    fn title_and_footer_title_on_the_borders() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 5));
        Panel::new(COOL_BLUE, BoxStyle::Rounded)
            .title("pty")
            .footer_title("cap")
            .render(buf.area, &mut buf);
        assert_eq!(row(&buf, 0), "╭─ pty ────────────╮");
        assert_eq!(row(&buf, 4), "╰─ cap ────────────╯");
        assert_eq!(row(&buf, 1), "│                  │");
        assert!(buf[(3, 0)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(3, 0)].fg, ratatui::style::Color::Rgb(100, 160, 255));
        assert_eq!(buf[(5, 2)].bg, ratatui::style::Color::Rgb(22, 27, 42));
        assert_eq!(Panel::inner(buf.area), Rect::new(2, 1, 16, 3));
    }

    #[test]
    fn separator_joins_the_borders() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let p = Panel::new(COOL_BLUE, BoxStyle::Rounded);
        p.clone().render(buf.area, &mut buf);
        p.separator(buf.area, 2, &mut buf);
        assert_eq!(row(&buf, 2), "├────────┤");
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let p = Panel::new(COOL_BLUE, BoxStyle::Double);
        p.clone().render(buf.area, &mut buf);
        p.separator(buf.area, 2, &mut buf);
        assert_eq!(row(&buf, 2), "╠════════╣");
    }
}
