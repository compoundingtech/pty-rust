//! A centred modal (`overlay(...)`, `screen.ts:168-275`): a shadow one row
//! down and two columns right in `rgb(8,10,16)`, then a [`Panel`] with the
//! title. The host clears the area and renders it after the base screen,
//! the way `app()` composited overlays by bounding box (`app.ts:123-153`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color as RColor, Style};
use ratatui::widgets::{Clear, Widget};

use super::panel::Panel;
use crate::theme::{BoxStyle, Theme};

/// The centred `width x height` rect inside `area` (`screen.ts:187-190`).
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// The overlay chrome. Render it at the centred rect; draw content into
/// [`Overlay::inner`].
#[derive(Debug, Clone)]
pub struct Overlay {
    pub title: String,
    pub theme: Theme,
    pub box_style: BoxStyle,
    /// Draw the shadow. Default true.
    pub shadow: bool,
}

impl Overlay {
    pub fn new(title: impl Into<String>, theme: Theme, box_style: BoxStyle) -> Self {
        Overlay {
            title: title.into(),
            theme,
            box_style,
            shadow: true,
        }
    }

    /// The content rect of an overlay rendered at `area`.
    pub fn inner(area: Rect) -> Rect {
        Panel::inner(area)
    }
}

impl Widget for Overlay {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.shadow {
            let shadow = Rect {
                x: area.x + 2,
                y: area.y + 1,
                width: area.width,
                height: area.height,
            }
            .intersection(buf.area);
            Clear.render(shadow, buf);
            buf.set_style(shadow, Style::default().bg(RColor::Rgb(8, 10, 16)));
        }
        Clear.render(area, buf);
        Panel::new(self.theme, self.box_style)
            .title(self.title)
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::COOL_BLUE;

    /// node: tests/tui-framework.test.ts (overlay render cases)
    #[test]
    fn centres_and_shadows() {
        let area = Rect::new(0, 0, 40, 12);
        let r = centered(area, 20, 6);
        assert_eq!(r, Rect::new(10, 3, 20, 6));
        let mut buf = Buffer::empty(area);
        Overlay::new("hi", COOL_BLUE, BoxStyle::Rounded).render(r, &mut buf);
        assert_eq!(buf[(10, 3)].symbol(), "╭");
        assert_eq!(buf[(31, 9)].bg, RColor::Rgb(8, 10, 16));
        assert_eq!(buf[(29, 8)].symbol(), "╯");
        assert_eq!(Overlay::inner(r), Rect::new(12, 4, 16, 4));
    }
}
