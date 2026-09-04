//! Free-form cell drawing (`canvas(...)`, `builders.ts:299-310`;
//! `executeCanvasDraw`, `renderer.ts:462-502`): a draw callback gets a
//! [`DrawContext`] with `set`, `write` (width-aware) and `fill`, all clipped
//! to the area. The [`Canvas`] widget runs it against its render area.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;

use crate::text::char_width;
use crate::theme::{Color, Theme};

/// One drawn cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasCell {
    pub x: u16,
    pub y: u16,
    pub ch: String,
    pub color: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
}

/// `DrawContext` (`nodes.ts:222-229`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawContext {
    pub width: u16,
    pub height: u16,
    pub cells: Vec<CanvasCell>,
}

impl DrawContext {
    pub fn new(width: u16, height: u16) -> Self {
        DrawContext {
            width,
            height,
            cells: Vec::new(),
        }
    }

    /// Put one character (clipped).
    pub fn set(&mut self, x: i32, y: i32, ch: &str, color: Option<Color>, bg: Option<Color>, bold: bool) {
        if x >= 0 && x < self.width as i32 && y >= 0 && y < self.height as i32 {
            self.cells.push(CanvasCell {
                x: x as u16,
                y: y as u16,
                ch: ch.to_string(),
                color,
                bg,
                bold,
            });
        }
    }

    /// Write a string left to right, advancing by display width.
    pub fn write(&mut self, x: i32, y: i32, s: &str, color: Option<Color>, bg: Option<Color>, bold: bool) {
        let mut cx = x;
        for ch in s.chars() {
            if cx >= self.width as i32 {
                break;
            }
            if cx >= 0 && y >= 0 && y < self.height as i32 {
                self.cells.push(CanvasCell {
                    x: cx as u16,
                    y: y as u16,
                    ch: ch.to_string(),
                    color,
                    bg,
                    bold,
                });
            }
            cx += char_width(ch) as i32;
        }
    }

    /// Fill a rectangle with `ch` (default space).
    #[allow(clippy::too_many_arguments)]
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, ch: Option<&str>, color: Option<Color>, bg: Option<Color>) {
        let ch = ch.unwrap_or(" ");
        let mut fy = y;
        while fy < y + h && fy < self.height as i32 {
            let mut fx = x;
            while fx < x + w && fx < self.width as i32 {
                if fx >= 0 && fy >= 0 {
                    self.cells.push(CanvasCell {
                        x: fx as u16,
                        y: fy as u16,
                        ch: ch.to_string(),
                        color,
                        bg,
                        bold: false,
                    });
                }
                fx += 1;
            }
            fy += 1;
        }
    }

    /// Paint the cells into `buf` at `area`.
    pub fn paint(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        for c in &self.cells {
            if c.x >= area.width || c.y >= area.height {
                continue;
            }
            let Some(cell) = buf.cell_mut((area.x + c.x, area.y + c.y)) else {
                continue;
            };
            cell.set_symbol(&c.ch);
            let mut style = Style::default();
            if let Some(fg) = c.color {
                style = style.fg(theme.color(fg));
            }
            if let Some(bg) = c.bg {
                style = style.bg(theme.color(bg));
            }
            if c.bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            cell.set_style(style);
        }
    }
}

/// A canvas node: `height`/`width` hints (else it fills its area) and the
/// draw callback.
pub struct Canvas<'a> {
    pub theme: Theme,
    pub height: Option<u16>,
    pub width: Option<u16>,
    pub draw: Box<dyn Fn(&mut DrawContext) + 'a>,
}

impl<'a> Canvas<'a> {
    pub fn new(theme: Theme, draw: impl Fn(&mut DrawContext) + 'a) -> Self {
        Canvas {
            theme,
            height: None,
            width: None,
            draw: Box::new(draw),
        }
    }

    pub fn height(mut self, h: u16) -> Self {
        self.height = Some(h);
        self
    }

    pub fn width(mut self, w: u16) -> Self {
        self.width = Some(w);
        self
    }

    /// Run the callback for a `width x height` area and return the cells.
    pub fn execute(&self, width: u16, height: u16) -> DrawContext {
        let mut ctx = DrawContext::new(width, height);
        (self.draw)(&mut ctx);
        ctx
    }
}

impl Widget for Canvas<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let w = self.width.map_or(area.width, |w| w.min(area.width));
        let h = self.height.map_or(area.height, |h| h.min(area.height));
        let ctx = self.execute(w, h);
        ctx.paint(&self.theme, Rect::new(area.x, area.y, w, h), buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/tui-framework.test.ts (canvas draw API cases)
    #[test]
    fn draw_api_clips_and_advances_by_width() {
        let mut ctx = DrawContext::new(5, 2);
        ctx.write(3, 0, "abcd", Some(Color::Accent), None, false);
        assert_eq!(ctx.cells.len(), 2);
        ctx.set(10, 0, "x", None, None, false);
        assert_eq!(ctx.cells.len(), 2);
        ctx.write(0, 1, "日x", None, None, true);
        assert_eq!(ctx.cells[2].x, 0);
        assert_eq!(ctx.cells[3].x, 2);
        let mut ctx = DrawContext::new(3, 3);
        ctx.fill(1, 1, 5, 5, Some("#"), None, Some(Color::Border));
        assert_eq!(ctx.cells.len(), 4);
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 2));
        Canvas::new(crate::theme::COOL_BLUE, |c| c.write(0, 0, "hi", Some(Color::Ok), None, true))
            .height(1)
            .render(buf.area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), "h");
        assert_eq!(buf[(0, 0)].fg, ratatui::style::Color::Rgb(80, 200, 120));
        assert!(buf[(0, 0)].modifier.contains(Modifier::BOLD));
    }
}
