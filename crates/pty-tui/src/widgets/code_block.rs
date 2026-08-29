//! Code block (`src/tui/widgets/code-block.ts`): one line per source line
//! with a right-aligned, at-least-3-wide muted line-number gutter
//! (`showLineNumbers: false` drops it) and an optional per-line highlight
//! callback returning styled spans. Wraps in a ratatui `Paragraph` via
//! [`code_block_paragraph`].

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme::{Color, Theme};

/// A per-line highlighter: the line and its 0-based index → spans.
pub type Highlighter<'a> = dyn Fn(&str, usize) -> Vec<Span<'static>> + 'a;

/// `CodeBlockOptions`.
pub struct CodeBlockOptions<'a> {
    /// Default 1.
    pub start_line: usize,
    /// Default muted.
    pub gutter_color: Color,
    /// Default true.
    pub show_line_numbers: bool,
    pub highlight: Option<Box<Highlighter<'a>>>,
}

impl Default for CodeBlockOptions<'_> {
    fn default() -> Self {
        CodeBlockOptions {
            start_line: 1,
            gutter_color: Color::Muted,
            show_line_numbers: true,
            highlight: None,
        }
    }
}

/// `codeBlock` as lines.
pub fn code_block(theme: &Theme, code: &str, opts: &CodeBlockOptions<'_>) -> Vec<Line<'static>> {
    let lines: Vec<&str> = code.split('\n').collect();
    let gutter_width = 3.max((opts.start_line + lines.len() - 1).to_string().len());
    let primary = Style::default().fg(theme.color(Color::Primary));
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let content: Vec<Span<'static>> = match &opts.highlight {
                Some(h) => h(line, i),
                None => vec![Span::styled(line.to_string(), primary)],
            };
            if !opts.show_line_numbers {
                return Line::from(content);
            }
            let mut spans = vec![Span::styled(
                format!("{:>gutter_width$} ", opts.start_line + i),
                Style::default().fg(theme.color(opts.gutter_color)),
            )];
            spans.extend(content);
            Line::from(spans)
        })
        .collect()
}

/// The block as a `Paragraph`.
pub fn code_block_paragraph<'a>(theme: &Theme, code: &str, opts: &CodeBlockOptions<'_>) -> Paragraph<'a> {
    Paragraph::new(code_block(theme, code, opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/code-block.test.ts:6-33
    #[test]
    fn node_contract() {
        let t = crate::theme::COOL_BLUE;
        let d = CodeBlockOptions::default();
        let c = code_block(&t, "a\nb\nc", &d);
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].spans[0].content, "  1 ");
        assert_eq!(c[0].spans[1].content, "a");
        let c = code_block(
            &t,
            "x\ny",
            &CodeBlockOptions {
                start_line: 41,
                ..Default::default()
            },
        );
        assert_eq!(c[0].spans[0].content, " 41 ");
        assert_eq!(c[1].spans[0].content, " 42 ");
        let c = code_block(
            &t,
            "a\nb",
            &CodeBlockOptions {
                show_line_numbers: false,
                ..Default::default()
            },
        );
        assert_eq!(c[0].spans.len(), 1);
        assert_eq!(c[0].spans[0].content, "a");
        let accent = Style::default().fg(t.color(Color::Accent));
        let c = code_block(
            &t,
            "kw x",
            &CodeBlockOptions {
                highlight: Some(Box::new(move |line, _| {
                    vec![Span::styled(line[..2].to_string(), accent), Span::raw(line[2..].to_string())]
                })),
                ..Default::default()
            },
        );
        assert_eq!(c[0].spans[1].content, "kw");
        assert_eq!(c[0].spans[1].style.fg, Some(t.color(Color::Accent)));
    }

    /// node: tests/code-block.test.ts:36-43
    #[test]
    fn rendered() {
        let t = crate::theme::COOL_BLUE;
        let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 30, 2));
        ratatui::widgets::Widget::render(
            code_block_paragraph(&t, "hello\nworld", &CodeBlockOptions::default()),
            buf.area,
            &mut buf,
        );
        let row = |y: u16| -> String { (0..30).map(|x| buf[(x, y)].symbol().to_string()).collect() };
        assert!(row(0).contains("  1 hello"));
        assert!(row(1).contains("  2 world"));
    }
}
