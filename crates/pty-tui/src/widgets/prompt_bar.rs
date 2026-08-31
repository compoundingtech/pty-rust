//! Prompt bar (`src/tui/widgets/prompt-bar.ts`): a Claude-Code-style
//! full-width input — a top rule with an optional title (left / center /
//! right), the `❯` glyph and the field (single-line [`TextFieldState`] or
//! multi-line [`TextAreaState`] with continuation rows indented three
//! columns), a bottom rule, and an optional status strip (left, right).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use super::text_area::TextAreaState;
use crate::line_edit::{TextFieldState, render_field_spans};
use crate::text::text_width;
use crate::theme::{Color, Theme};

/// The value (`PromptBarValue`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptBarValue<'a> {
    Single(&'a TextFieldState),
    Multi(&'a TextAreaState),
}

/// Title alignment on the top rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TitleAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// `PromptBarTitle`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBarTitle {
    pub text: String,
    pub align: TitleAlign,
    pub color: Color,
}

impl PromptBarTitle {
    pub fn new(text: impl Into<String>) -> Self {
        PromptBarTitle {
            text: text.into(),
            align: TitleAlign::Left,
            color: Color::Accent,
        }
    }

    pub fn align(mut self, align: TitleAlign) -> Self {
        self.align = align;
        self
    }
}

/// `PromptBarStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptBarStatus {
    pub left: Option<String>,
    pub right: Option<String>,
    pub color: Option<Color>,
}

/// `PromptBarOptions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBarOptions {
    pub glyph: String,
    pub glyph_color: Color,
    pub title: Option<PromptBarTitle>,
    pub status: Option<PromptBarStatus>,
    pub active: bool,
}

impl Default for PromptBarOptions {
    fn default() -> Self {
        PromptBarOptions {
            glyph: "\u{276f}".into(),
            glyph_color: Color::Accent,
            title: None,
            status: None,
            active: true,
        }
    }
}

/// One row of the bar.
#[derive(Debug, Clone, PartialEq)]
pub enum PromptRow {
    /// The plain top or bottom rule.
    Rule,
    /// A content or status line.
    Line(Line<'static>),
}

fn rule_line(theme: &Theme, width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "\u{2500}".repeat(width),
        Style::default()
            .fg(theme.color(Color::Muted))
            .add_modifier(Modifier::DIM),
    ))
}

/// The top rule with a title overlaid (`titleRule`).
pub fn title_rule(theme: &Theme, title: &PromptBarTitle, width: usize) -> Line<'static> {
    let label = format!(" {} ", title.text);
    let lw = text_width(&label);
    let rest = width.saturating_sub(lw);
    let (left, right) = match title.align {
        TitleAlign::Left => (2.min(rest), rest.saturating_sub(2)),
        TitleAlign::Center => (rest / 2, rest - rest / 2),
        TitleAlign::Right => (rest.saturating_sub(2), 2.min(rest)),
    };
    let rule = Style::default()
        .fg(theme.color(Color::Muted))
        .add_modifier(Modifier::DIM);
    Line::from(vec![
        Span::styled("\u{2500}".repeat(left), rule),
        Span::styled(
            label,
            Style::default()
                .fg(theme.color(title.color))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("\u{2500}".repeat(right), rule),
    ])
}

/// The rows of a prompt bar (`promptBar`, `prompt-bar.ts:86-135`) for a
/// given width: title rule, content row(s), bottom rule, optional status.
pub fn prompt_bar_rows(theme: &Theme, value: PromptBarValue<'_>, opts: &PromptBarOptions, width: usize) -> Vec<PromptRow> {
    let primary = Style::default().fg(theme.color(Color::Primary));
    let glyph_style = Style::default()
        .fg(theme.color(opts.glyph_color))
        .add_modifier(Modifier::BOLD);
    let mut rows = vec![match &opts.title {
        Some(t) => PromptRow::Line(title_rule(theme, t, width)),
        None => PromptRow::Rule,
    }];
    match value {
        PromptBarValue::Multi(state) => {
            for (i, line) in state.lines.iter().enumerate() {
                let on_cursor = opts.active && i == state.row;
                let prompt = if i == 0 {
                    Span::styled(format!(" {} ", opts.glyph), glyph_style)
                } else {
                    Span::styled("   ", Style::default().fg(theme.color(Color::Muted)))
                };
                let mut spans = vec![prompt];
                if !on_cursor {
                    let text = if line.is_empty() { " ".to_string() } else { line.clone() };
                    spans.push(Span::styled(text, primary));
                } else {
                    spans.extend(render_field_spans(line, state.col, true, primary));
                }
                rows.push(PromptRow::Line(Line::from(spans)));
            }
        }
        PromptBarValue::Single(state) => {
            let mut spans = vec![Span::styled(format!(" {} ", opts.glyph), glyph_style)];
            spans.extend(render_field_spans(&state.text, state.cursor, opts.active, primary));
            rows.push(PromptRow::Line(Line::from(spans)));
        }
    }
    rows.push(PromptRow::Rule);
    if let Some(s) = &opts.status
        && (s.left.is_some() || s.right.is_some())
    {
        let style = Style::default()
            .fg(theme.color(s.color.unwrap_or(Color::Muted)))
            .add_modifier(Modifier::DIM);
        let left = s.left.clone().unwrap_or_default();
        let right = s.right.clone().map(|r| format!("{r}  ")).unwrap_or_default();
        let gap = width.saturating_sub(text_width(&left) + text_width(&right));
        rows.push(PromptRow::Line(Line::from(vec![
            Span::styled(left, style),
            Span::raw(" ".repeat(gap)),
            Span::styled(right, style),
        ])));
    }
    rows
}

/// The prompt bar widget.
pub struct PromptBar<'a> {
    pub value: PromptBarValue<'a>,
    pub opts: PromptBarOptions,
    pub theme: Theme,
}

impl<'a> PromptBar<'a> {
    pub fn new(theme: Theme, value: PromptBarValue<'a>, opts: PromptBarOptions) -> Self {
        PromptBar { value, opts, theme }
    }

    /// Rows this bar needs.
    pub fn height(&self) -> u16 {
        prompt_bar_rows(&self.theme, self.value.clone(), &self.opts, 80).len() as u16
    }
}

impl Widget for PromptBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let rows = prompt_bar_rows(&self.theme, self.value, &self.opts, area.width as usize);
        for (i, row) in rows.iter().enumerate() {
            if i as u16 >= area.height {
                break;
            }
            let line = match row {
                PromptRow::Rule => rule_line(&self.theme, area.width as usize),
                PromptRow::Line(l) => l.clone(),
            };
            buf.set_line(area.x, area.y + i as u16, &line, area.width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(text: &str) -> TextFieldState {
        TextFieldState::new(text)
    }

    /// node: tests/widgets-prompt-bar.test.ts:6-59
    #[test]
    fn rows() {
        let t = crate::theme::COOL_BLUE;
        let s = single("hi");
        let rows = prompt_bar_rows(&t, PromptBarValue::Single(&s), &PromptBarOptions::default(), 40);
        assert_eq!(rows.len(), 3);
        let e = single("");
        let opts = PromptBarOptions {
            status: Some(PromptBarStatus {
                left: Some("L".into()),
                right: Some("R".into()),
                color: None,
            }),
            ..Default::default()
        };
        assert_eq!(prompt_bar_rows(&t, PromptBarValue::Single(&e), &opts, 40).len(), 4);
        let opts = PromptBarOptions {
            title: Some(PromptBarTitle::new("compose").align(TitleAlign::Center)),
            ..Default::default()
        };
        let rows = prompt_bar_rows(&t, PromptBarValue::Single(&e), &opts, 40);
        let PromptRow::Line(top) = &rows[0] else { panic!("title row") };
        assert!(top.to_string().contains("compose"));
        assert_eq!(top.to_string().chars().count(), 40);
        let opts = PromptBarOptions {
            glyph: "$".into(),
            ..Default::default()
        };
        let rows = prompt_bar_rows(&t, PromptBarValue::Single(&e), &opts, 40);
        let PromptRow::Line(input) = &rows[1] else { panic!("input row") };
        assert_eq!(input.spans[0].content, " $ ");
        let area = TextAreaState::new("line one\nline two");
        let opts = PromptBarOptions {
            title: Some(PromptBarTitle::new("chat")),
            ..Default::default()
        };
        let rows = prompt_bar_rows(&t, PromptBarValue::Multi(&area), &opts, 40);
        assert_eq!(rows.len(), 4);
        let PromptRow::Line(first) = &rows[1] else { panic!() };
        assert!(first.spans[0].content.contains('\u{276f}'));
        let PromptRow::Line(second) = &rows[2] else { panic!() };
        assert_eq!(second.spans[0].content, "   ");
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 4));
        PromptBar::new(t, PromptBarValue::Single(&s), PromptBarOptions::default()).render(buf.area, &mut buf);
        let row = |y: u16| -> String { (0..20).map(|x| buf[(x, y)].symbol().to_string()).collect() };
        assert_eq!(row(0), "─".repeat(20));
        assert!(row(1).starts_with(" ❯ hi"));
    }
}
