//! Accordion (`src/tui/widgets/accordion.ts`): a `▸ title` / `▾ title`
//! header that toggles indented content. The caller owns `expanded`;
//! `focused` renders the header accent + bold.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{Color, Theme};

/// `AccordionOptions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccordionOptions {
    pub focused: bool,
    pub collapsed_icon: String,
    pub expanded_icon: String,
    pub indent: usize,
}

impl Default for AccordionOptions {
    fn default() -> Self {
        AccordionOptions {
            focused: false,
            collapsed_icon: "\u{25b8}".into(),
            expanded_icon: "\u{25be}".into(),
            indent: 2,
        }
    }
}

/// The header line.
pub fn accordion_header(theme: &Theme, title: &str, expanded: bool, opts: &AccordionOptions) -> Line<'static> {
    let icon = if expanded { &opts.expanded_icon } else { &opts.collapsed_icon };
    let icon_color = if opts.focused { Color::Accent } else { Color::Muted };
    let title_color = if opts.focused { Color::Accent } else { Color::Primary };
    let mut title_style = Style::default().fg(theme.color(title_color));
    if opts.focused {
        title_style = title_style.add_modifier(Modifier::BOLD);
    }
    Line::from(vec![
        Span::styled(format!("{icon} "), Style::default().fg(theme.color(icon_color))),
        Span::styled(title.to_string(), title_style),
    ])
}

/// `accordion`: the header plus, when expanded, `children` indented by
/// `opts.indent` columns.
pub fn accordion(
    theme: &Theme,
    title: &str,
    expanded: bool,
    children: &[Line<'static>],
    opts: &AccordionOptions,
) -> Vec<Line<'static>> {
    let mut lines = vec![accordion_header(theme, title, expanded, opts)];
    if expanded {
        for child in children {
            let mut spans = vec![Span::raw(" ".repeat(opts.indent))];
            spans.extend(child.spans.iter().cloned());
            lines.push(Line::from(spans));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/accordion.test.ts:6-39
    #[test]
    fn node_contract() {
        let t = crate::theme::COOL_BLUE;
        let d = AccordionOptions::default();
        let child = vec![Line::raw("child")];
        let a = accordion(&t, "Group", false, &child, &d);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].spans[0].content, "▸ ");
        assert_eq!(a[0].spans[1].content, "Group");
        let a = accordion(&t, "Group", true, &child, &d);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].spans[0].content, "▾ ");
        assert_eq!(a[1].spans[0].content, "  ");
        assert_eq!(a[1].to_string(), "  child");
        assert_eq!(accordion(&t, "Empty", true, &[], &d).len(), 1);
        let f = accordion(&t, "Group", false, &[], &AccordionOptions { focused: true, ..d.clone() });
        assert_eq!(f[0].spans[1].style.fg, Some(t.color(Color::Accent)));
        assert!(f[0].spans[1].style.add_modifier.contains(Modifier::BOLD));
        let custom = AccordionOptions {
            expanded_icon: "-".into(),
            collapsed_icon: "+".into(),
            ..d
        };
        assert_eq!(accordion(&t, "G", true, &child, &custom)[0].spans[0].content, "- ");
    }
}
