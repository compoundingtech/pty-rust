//! Sparkline (`src/tui/widgets/sparkline.ts`): one block glyph per sample
//! (` ▁▂▃▄▅▆▇█`), the tail of the series when it is longer than `width`,
//! left-padded with empties when shorter, NaN/∞ as empty, an all-equal
//! series as the middle block. Also a ratatui `Sparkline` wrapper.

use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Sparkline as RSparkline;

use crate::theme::{Color, Theme};

const BLOCKS: [&str; 9] = [
    " ", "\u{2581}", "\u{2582}", "\u{2583}", "\u{2584}", "\u{2585}", "\u{2586}", "\u{2587}", "\u{2588}",
];

/// `SparklineOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SparklineOptions {
    /// Render width; default the series length.
    pub width: Option<usize>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Colour of the text form; default accent.
    pub color: Option<Color>,
}

/// The block levels (0-8) per rendered cell (`sparklineString` before the
/// glyph lookup).
pub fn sparkline_levels(series: &[f64], opts: &SparklineOptions) -> Vec<u8> {
    let width = opts.width.unwrap_or(series.len());
    if width == 0 || series.is_empty() {
        return Vec::new();
    }
    let slice = if series.len() <= width {
        series
    } else {
        &series[series.len() - width..]
    };
    let (mut lo, mut hi) = (opts.min, opts.max);
    if lo.is_none() || hi.is_none() {
        let mut ilo = f64::INFINITY;
        let mut ihi = f64::NEG_INFINITY;
        for &v in slice {
            if !v.is_finite() {
                continue;
            }
            ilo = ilo.min(v);
            ihi = ihi.max(v);
        }
        if !ilo.is_finite() {
            ilo = 0.0;
        }
        if !ihi.is_finite() {
            ihi = 1.0;
        }
        lo = lo.or(Some(ilo));
        hi = hi.or(Some(ihi));
    }
    let (lo, hi) = (lo.unwrap(), hi.unwrap());
    let range = hi - lo;
    let mut out = vec![0u8; width - slice.len()];
    for &v in slice {
        if !v.is_finite() {
            out.push(0);
        } else if range <= 0.0 {
            out.push(4);
        } else {
            let frac = (v - lo) / range;
            out.push((frac * 8.0).round().clamp(0.0, 8.0) as u8);
        }
    }
    out
}

/// `sparklineString`.
pub fn sparkline_string(series: &[f64], opts: &SparklineOptions) -> String {
    sparkline_levels(series, opts)
        .into_iter()
        .map(|l| BLOCKS[l as usize])
        .collect()
}

/// The sparkline as a coloured span (`sparkline`).
pub fn sparkline(theme: &Theme, series: &[f64], opts: &SparklineOptions) -> Span<'static> {
    Span::styled(
        sparkline_string(series, opts),
        Style::default().fg(theme.color(opts.color.unwrap_or(Color::Accent))),
    )
}

/// A ratatui `Sparkline` with the same levels (max 8).
pub fn sparkline_widget<'a>(theme: &Theme, series: &[f64], opts: &SparklineOptions) -> RSparkline<'a> {
    let data: Vec<u64> = sparkline_levels(series, opts).into_iter().map(u64::from).collect();
    RSparkline::default()
        .data(data)
        .max(8)
        .style(Style::default().fg(theme.color(opts.color.unwrap_or(Color::Accent))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(width: Option<usize>, min: Option<f64>, max: Option<f64>) -> SparklineOptions {
        SparklineOptions {
            width,
            min,
            max,
            color: None,
        }
    }

    /// node: tests/widgets-sparkline.test.ts:4-46
    #[test]
    fn string_form() {
        assert_eq!(sparkline_string(&[0.0, 1.0, 2.0, 3.0], &o(None, None, None)).chars().count(), 4);
        let s: Vec<char> = sparkline_string(&[8.0], &o(Some(4), Some(0.0), Some(8.0))).chars().collect();
        assert_eq!(s.len(), 4);
        assert_eq!(s[3], '\u{2588}');
        assert_eq!(s[0], ' ');
        let s: Vec<char> = sparkline_string(&[1., 2., 3., 4., 5., 6., 7., 8.], &o(Some(3), Some(1.0), Some(8.0)))
            .chars()
            .collect();
        assert_eq!(s.len(), 3);
        assert_eq!(s[2], '\u{2588}');
        let a = sparkline_string(&[50.0], &o(Some(1), Some(0.0), Some(100.0)));
        let b = sparkline_string(&[50.0], &o(Some(1), Some(0.0), Some(100.0)));
        assert_eq!(a, b);
        let s: Vec<char> = sparkline_string(&[f64::NAN, f64::INFINITY, 1.0], &o(None, Some(0.0), Some(1.0)))
            .chars()
            .collect();
        assert_eq!(s, vec![' ', ' ', '\u{2588}']);
        assert_eq!(sparkline_string(&[5.0, 5.0, 5.0], &o(None, Some(5.0), Some(5.0))), "\u{2584}\u{2584}\u{2584}");
        assert_eq!(sparkline_string(&[], &o(None, None, None)), "");
        assert_eq!(sparkline_string(&[1.0, 2.0, 3.0], &o(Some(0), None, None)), "");
        let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 4, 1));
        ratatui::widgets::Widget::render(
            sparkline_widget(&crate::theme::COOL_BLUE, &[0.0, 4.0, 8.0, 8.0], &o(None, Some(0.0), Some(8.0))),
            buf.area,
            &mut buf,
        );
        assert_eq!(buf[(3, 0)].symbol(), "\u{2588}");
        assert_eq!(buf[(1, 0)].symbol(), "\u{2584}");
    }
}
