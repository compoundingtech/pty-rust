//! Breadcrumbs (`src/tui/widgets/breadcrumbs.ts`): labels joined by ` ❯ `
//! (muted), ancestors secondary, the last crumb accent + bold; `chips`
//! pads each crumb on a border fill.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{Color, Theme};

/// `BreadCrumbsOptions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreadCrumbsOptions {
    pub separator: String,
    pub emphasize_last: bool,
    pub chips: bool,
}

impl Default for BreadCrumbsOptions {
    fn default() -> Self {
        BreadCrumbsOptions {
            separator: " \u{276f} ".into(),
            emphasize_last: true,
            chips: false,
        }
    }
}

/// `breadCrumbs` as one line.
pub fn bread_crumbs(theme: &Theme, items: &[&str], opts: &BreadCrumbsOptions) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, label) in items.iter().enumerate() {
        let is_last = i + 1 == items.len();
        let current = is_last && opts.emphasize_last;
        let color = if current { Color::Accent } else { Color::Secondary };
        let mut style = Style::default().fg(theme.color(color));
        if current {
            style = style.add_modifier(Modifier::BOLD);
        }
        if opts.chips {
            style = style.bg(theme.color(Color::Border));
        }
        let text = if opts.chips { format!(" {label} ") } else { label.to_string() };
        spans.push(Span::styled(text, style));
        if !is_last {
            spans.push(Span::styled(
                opts.separator.clone(),
                Style::default().fg(theme.color(Color::Muted)),
            ));
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/breadcrumbs.test.ts:6-52
    #[test]
    fn node_contract() {
        let t = crate::theme::COOL_BLUE;
        let d = BreadCrumbsOptions::default();
        let l = bread_crumbs(&t, &["net", "host", "agent"], &d);
        let texts: Vec<&str> = l.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(texts, vec!["net", " ❯ ", "host", " ❯ ", "agent"]);
        assert_eq!(l.spans[0].style.fg, Some(t.color(Color::Secondary)));
        assert_eq!(l.spans[1].style.fg, Some(t.color(Color::Muted)));
        assert_eq!(l.spans[4].style.fg, Some(t.color(Color::Accent)));
        assert!(l.spans[4].style.add_modifier.contains(Modifier::BOLD));
        let l = bread_crumbs(&t, &["a", "b"], &BreadCrumbsOptions { emphasize_last: false, ..d.clone() });
        assert_eq!(l.spans[2].style.fg, Some(t.color(Color::Secondary)));
        assert!(!l.spans[2].style.add_modifier.contains(Modifier::BOLD));
        let l = bread_crumbs(&t, &["a"], &BreadCrumbsOptions { chips: true, ..d.clone() });
        assert_eq!(l.spans[0].content, " a ");
        assert_eq!(l.spans[0].style.bg, Some(t.color(Color::Border)));
        let l = bread_crumbs(&t, &["a", "b"], &BreadCrumbsOptions { separator: " / ".into(), ..d.clone() });
        assert_eq!(l.spans[1].content, " / ");
        assert_eq!(bread_crumbs(&t, &["only"], &d).spans.len(), 1);
        assert_eq!(bread_crumbs(&t, &["net", "host", "agent"], &d).to_string(), "net ❯ host ❯ agent");
    }
}
