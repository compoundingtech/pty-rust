//! Bar chart (`src/tui/widgets/bar-chart.ts`): a vertical histogram drawn
//! on a [`Canvas`] with 1/8-block resolution, `bar_width` columns per bar
//! (default 2), `gap` between (default 1), `height` rows (default 6, plus a
//! label row with `show_labels`). Also a ratatui `BarChart` wrapper.

use ratatui::style::Style;
use ratatui::widgets::{Bar, BarChart as RBarChart, BarGroup};

use super::canvas::{Canvas, DrawContext};
use crate::theme::{Color, Theme};

const BLOCKS: [&str; 9] = [
    " ", "\u{2581}", "\u{2582}", "\u{2583}", "\u{2584}", "\u{2585}", "\u{2586}", "\u{2587}", "\u{2588}",
];

/// `BarChartItem`.
#[derive(Debug, Clone, PartialEq)]
pub struct BarChartItem {
    pub label: Option<String>,
    pub value: f64,
    pub color: Option<Color>,
}

impl BarChartItem {
    pub fn new(value: f64) -> Self {
        BarChartItem {
            label: None,
            value,
            color: None,
        }
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

/// `BarChartOptions`.
#[derive(Debug, Clone, PartialEq)]
pub struct BarChartOptions {
    pub height: u16,
    pub bar_width: u16,
    pub gap: u16,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub color: Color,
    pub show_labels: bool,
    pub label_color: Color,
}

impl Default for BarChartOptions {
    fn default() -> Self {
        BarChartOptions {
            height: 6,
            bar_width: 2,
            gap: 1,
            min: None,
            max: None,
            color: Color::Accent,
            show_labels: false,
            label_color: Color::Muted,
        }
    }
}

fn bounds(items: &[BarChartItem], opts: &BarChartOptions) -> (f64, f64) {
    let finite = || items.iter().map(|i| i.value).filter(|v| v.is_finite());
    let lo = opts.min.unwrap_or_else(|| {
        let m = finite().fold(f64::INFINITY, f64::min);
        if m.is_finite() { m } else { 0.0 }
    });
    let hi = opts.max.unwrap_or_else(|| {
        let m = finite().fold(f64::NEG_INFINITY, f64::max);
        if m.is_finite() { m } else { 1.0 }
    });
    (lo, hi)
}

/// Total rows the chart occupies (`height` + 1 with labels).
pub fn bar_chart_height(opts: &BarChartOptions) -> u16 {
    let height = opts.height.max(2);
    if opts.show_labels { height + 1 } else { height }
}

/// Draw the bars into `ctx` (`barChart` draw callback).
pub fn bar_chart_draw(items: &[BarChartItem], opts: &BarChartOptions, ctx: &mut DrawContext) {
    let height = opts.height.max(2) as i32;
    let bar_width = opts.bar_width.max(1) as i32;
    let gap = opts.gap as i32;
    let (lo, hi) = bounds(items, opts);
    let range = hi - lo;
    for (idx, item) in items.iter().enumerate() {
        let v = if item.value.is_finite() { item.value } else { lo };
        let frac = if range <= 0.0 { 0.5 } else { (v - lo) / range };
        let clamped = frac.clamp(0.0, 1.0);
        let total_eighths = (clamped * height as f64 * 8.0).round() as i32;
        let full_rows = total_eighths / 8;
        let top = total_eighths % 8;
        let color = Some(item.color.unwrap_or(opts.color));
        let left = idx as i32 * (bar_width + gap);
        for r in 0..full_rows {
            let y = height - 1 - r;
            for c in 0..bar_width {
                ctx.write(left + c, y, BLOCKS[8], color, None, false);
            }
        }
        if top > 0 {
            let y = height - 1 - full_rows;
            if y >= 0 {
                for c in 0..bar_width {
                    ctx.write(left + c, y, BLOCKS[top as usize], color, None, false);
                }
            }
        }
        if opts.show_labels
            && let Some(label) = &item.label
        {
            let s: String = label.chars().take(bar_width as usize).collect();
            if !s.is_empty() {
                ctx.write(left, height, &s, Some(opts.label_color), None, false);
            }
        }
    }
}

/// The chart as a canvas with a fixed height (`barChart`).
pub fn bar_chart<'a>(theme: Theme, items: &'a [BarChartItem], opts: &'a BarChartOptions) -> Canvas<'a> {
    Canvas::new(theme, move |ctx| bar_chart_draw(items, opts, ctx)).height(bar_chart_height(opts))
}

/// A ratatui `BarChart` with the same items, widths and colours (values in
/// 1/8 rows of `height`).
pub fn bar_chart_widget<'a>(theme: &Theme, items: &[BarChartItem], opts: &BarChartOptions) -> RBarChart<'a> {
    let height = opts.height.max(2) as f64;
    let (lo, hi) = bounds(items, opts);
    let range = hi - lo;
    let bars: Vec<Bar<'a>> = items
        .iter()
        .map(|item| {
            let v = if item.value.is_finite() { item.value } else { lo };
            let frac = if range <= 0.0 { 0.5 } else { ((v - lo) / range).clamp(0.0, 1.0) };
            let value = (frac * height * 8.0).round() as u64;
            let mut bar = Bar::default()
                .value(value)
                .text_value(String::new())
                .style(Style::default().fg(theme.color(item.color.unwrap_or(opts.color))));
            if opts.show_labels && let Some(l) = &item.label {
                bar = bar.label(ratatui::text::Line::from(l.clone()));
            }
            bar
        })
        .collect();
    RBarChart::default()
        .data(BarGroup::default().bars(&bars))
        .bar_width(opts.bar_width.max(1))
        .bar_gap(opts.gap)
        .max((height * 8.0) as u64)
        .label_style(Style::default().fg(theme.color(opts.label_color)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::Widget;

    /// node: tests/widgets-bar-chart.test.ts:4-27
    #[test]
    fn heights_and_draw() {
        let t = crate::theme::COOL_BLUE;
        let items = vec![BarChartItem::new(1.0), BarChartItem::new(2.0)];
        let opts = BarChartOptions {
            height: 4,
            ..Default::default()
        };
        assert_eq!(bar_chart(t, &items, &opts).height, Some(4));
        let labelled = vec![BarChartItem::new(1.0).label("A")];
        let opts_l = BarChartOptions {
            height: 4,
            show_labels: true,
            ..Default::default()
        };
        assert_eq!(bar_chart(t, &labelled, &opts_l).height, Some(5));
        let empty: Vec<BarChartItem> = vec![];
        let opts3 = BarChartOptions {
            height: 3,
            ..Default::default()
        };
        assert_eq!(bar_chart(t, &empty, &opts3).height, Some(3));
        assert!(bar_chart(t, &empty, &opts3).execute(10, 3).cells.is_empty());
        let same = vec![BarChartItem::new(5.0), BarChartItem::new(5.0)];
        let opts_same = BarChartOptions {
            min: Some(5.0),
            max: Some(5.0),
            ..Default::default()
        };
        assert!(!bar_chart(t, &same, &opts_same).execute(10, 6).cells.is_empty());
        // Two bars, height 4: value 2 fills all 4 rows, value 1 fills 2.
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 4));
        let opts = BarChartOptions {
            height: 4,
            min: Some(0.0),
            ..Default::default()
        };
        bar_chart(t, &items, &opts).render(buf.area, &mut buf);
        assert_eq!(buf[(0, 3)].symbol(), "\u{2588}");
        assert_eq!(buf[(0, 1)].symbol(), " ");
        assert_eq!(buf[(3, 0)].symbol(), "\u{2588}");
        assert_eq!(buf[(2, 3)].symbol(), " "); // the gap
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 4));
        bar_chart_widget(&t, &items, &opts).render(buf.area, &mut buf);
        assert_eq!(buf[(3, 0)].symbol(), "\u{2588}");
    }
}
