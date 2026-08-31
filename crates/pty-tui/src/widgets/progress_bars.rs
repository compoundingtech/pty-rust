//! Progress bars (`src/tui/widgets/progress-bars.ts`): `bar_progress` fills
//! with `░`, `bar_loader` with `█`, proportional to a percent clamped to
//! 0-100, on a border-toned track (`background: None` disables it). Also a
//! ratatui `Gauge` wrapper.

use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Gauge;

use crate::theme::{Color, Theme};

/// `BarOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarOptions {
    /// Default 20.
    pub width: usize,
    /// Default accent.
    pub color: Color,
    /// Default border; `None` for no track fill.
    pub background: Option<Color>,
}

impl Default for BarOptions {
    fn default() -> Self {
        BarOptions {
            width: 20,
            color: Color::Accent,
            background: Some(Color::Border),
        }
    }
}

fn clamp_percent(p: f64) -> f64 {
    p.clamp(0.0, 100.0)
}

/// The bar text: `fill` for the filled cells, spaces for the track.
pub fn bar_string(percent: f64, width: usize, fill: &str) -> String {
    let filled = ((clamp_percent(percent) / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!("{}{}", fill.repeat(filled), " ".repeat(width - filled))
}

fn bar(theme: &Theme, percent: f64, opts: &BarOptions, fill: &str) -> Span<'static> {
    let mut style = Style::default().fg(theme.color(opts.color));
    if let Some(bg) = opts.background {
        style = style.bg(theme.color(bg));
    }
    Span::styled(bar_string(percent, opts.width, fill), style)
}

/// `barProgress`: a `░` texture fill.
pub fn bar_progress(theme: &Theme, percent: f64, opts: &BarOptions) -> Span<'static> {
    bar(theme, percent, opts, "\u{2591}")
}

/// `barLoader`: a solid `█` fill.
pub fn bar_loader(theme: &Theme, percent: f64, opts: &BarOptions) -> Span<'static> {
    bar(theme, percent, opts, "\u{2588}")
}

/// A ratatui `Gauge` in the same colours (fills its area).
pub fn gauge<'a>(theme: &Theme, percent: f64, opts: &BarOptions) -> Gauge<'a> {
    let mut style = Style::default().fg(theme.color(opts.color));
    if let Some(bg) = opts.background {
        style = style.bg(theme.color(bg));
    }
    Gauge::default()
        .ratio(clamp_percent(percent) / 100.0)
        .gauge_style(style)
        .label("")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/progress-bars.test.ts:6-37
    #[test]
    fn node_contract() {
        let t = crate::theme::COOL_BLUE;
        let w10 = BarOptions {
            width: 10,
            ..Default::default()
        };
        assert_eq!(bar_progress(&t, 50.0, &w10).content, "░░░░░     ");
        assert_eq!(bar_loader(&t, 30.0, &w10).content, "███       ");
        let b = bar_progress(&t, 50.0, &w10);
        assert_eq!(b.style.fg, Some(t.color(Color::Accent)));
        assert_eq!(b.style.bg, Some(t.color(Color::Border)));
        let w4 = BarOptions {
            width: 4,
            ..Default::default()
        };
        assert_eq!(bar_progress(&t, 150.0, &w4).content, "░░░░");
        assert_eq!(bar_progress(&t, -10.0, &w4).content, "    ");
        let b = bar_progress(
            &t,
            25.0,
            &BarOptions {
                width: 8,
                color: Color::Ok,
                ..Default::default()
            },
        );
        assert_eq!(b.style.fg, Some(t.color(Color::Ok)));
        assert_eq!(b.content, "░░      ");
        let b = bar_progress(
            &t,
            50.0,
            &BarOptions {
                background: None,
                ..Default::default()
            },
        );
        assert_eq!(b.style.bg, None);
    }

    /// node: tests/progress-bars.test.ts:40-55
    #[test]
    fn rendered() {
        let t = crate::theme::COOL_BLUE;
        let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 20, 1));
        let w10 = BarOptions {
            width: 10,
            ..Default::default()
        };
        buf.set_span(0, 0, &bar_progress(&t, 50.0, &w10), 20);
        assert_eq!(buf[(0, 0)].symbol(), "░");
        assert_eq!(buf[(0, 0)].fg, ratatui::style::Color::Rgb(100, 160, 255));
        assert_eq!(buf[(0, 0)].bg, ratatui::style::Color::Rgb(50, 60, 85));
        assert_eq!(buf[(6, 0)].symbol(), " ");
        assert_eq!(buf[(6, 0)].bg, ratatui::style::Color::Rgb(50, 60, 85));
        let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 10, 1));
        ratatui::widgets::Widget::render(gauge(&t, 50.0, &w10), buf.area, &mut buf);
        assert_eq!(buf[(0, 0)].fg, ratatui::style::Color::Rgb(100, 160, 255));
    }
}
