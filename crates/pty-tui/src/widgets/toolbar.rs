//! Toolbar (`src/tui/widgets/toolbar.ts`): a hotkey legend row. `bracket`
//! format renders `[N]ew  [S]ave` (the key uppercased and bold); `inline`
//! highlights the first occurrence of the key inside the label. Active
//! items are accent + bold, disabled items dim muted.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{Color, Theme};

/// `ToolbarItem`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarItem {
    pub key: String,
    pub label: String,
    pub hint: Option<String>,
    pub active: bool,
    pub disabled: bool,
}

impl ToolbarItem {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        ToolbarItem {
            key: key.into(),
            label: label.into(),
            hint: None,
            active: false,
            disabled: false,
        }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// Render format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolbarFormat {
    #[default]
    Bracket,
    Inline,
}

/// `ToolbarOptions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarOptions {
    pub separator: String,
    pub format: ToolbarFormat,
    pub active_color: Color,
}

impl Default for ToolbarOptions {
    fn default() -> Self {
        ToolbarOptions {
            separator: "  ".into(),
            format: ToolbarFormat::Bracket,
            active_color: Color::Accent,
        }
    }
}

fn style(theme: &Theme, color: Color, bold: bool, dim: bool) -> Style {
    let mut s = Style::default().fg(theme.color(color));
    if bold {
        s = s.add_modifier(Modifier::BOLD);
    }
    if dim {
        s = s.add_modifier(Modifier::DIM);
    }
    s
}

fn bracketize(theme: &Theme, item: &ToolbarItem, opts: &ToolbarOptions) -> Vec<Span<'static>> {
    let base = if item.active { opts.active_color } else { Color::Primary };
    let mut cell = vec![
        Span::styled("[", style(theme, base, item.active, false)),
        Span::styled(item.key.to_uppercase(), style(theme, base, true, false)),
        Span::styled("]", style(theme, base, item.active, false)),
        Span::styled(
            item.label.clone(),
            style(
                theme,
                if item.disabled { Color::Muted } else { base },
                item.active,
                item.disabled,
            ),
        ),
    ];
    if let Some(h) = &item.hint {
        cell.push(Span::styled(format!(" {h}"), style(theme, Color::Muted, false, true)));
    }
    cell
}

fn inlineize(theme: &Theme, item: &ToolbarItem, opts: &ToolbarOptions) -> Vec<Span<'static>> {
    let base = if item.active { opts.active_color } else { Color::Primary };
    let label_l = item.label.to_lowercase();
    let Some(idx) = label_l.find(&item.key.to_lowercase()) else {
        return bracketize(theme, item, opts);
    };
    let chars: Vec<char> = item.label.chars().collect();
    let cidx = item.label[..idx].chars().count();
    let before: String = chars[..cidx].iter().collect();
    let k: String = chars[cidx].to_string();
    let after: String = chars[cidx + 1..].iter().collect();
    let label_color = if item.disabled { Color::Muted } else { base };
    let mut cell = vec![
        Span::styled(before, style(theme, label_color, false, item.disabled)),
        Span::styled(k, style(theme, Color::Accent, true, item.disabled)),
        Span::styled(after, style(theme, label_color, false, item.disabled)),
    ];
    if let Some(h) = &item.hint {
        cell.push(Span::styled(format!(" {h}"), style(theme, Color::Muted, false, true)));
    }
    cell
}

/// The toolbar as one line (`toolbar`).
pub fn toolbar(theme: &Theme, items: &[ToolbarItem], opts: &ToolbarOptions) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(opts.separator.clone(), style(theme, Color::Muted, false, false)));
        }
        spans.extend(match opts.format {
            ToolbarFormat::Inline => inlineize(theme, item, opts),
            ToolbarFormat::Bracket => bracketize(theme, item, opts),
        });
    }
    Line::from(spans)
}

/// The enabled item bound to `ch` (case-insensitive) (`toolbarItemFor`).
pub fn toolbar_item_for<'a>(items: &'a [ToolbarItem], ch: Option<&str>) -> Option<&'a ToolbarItem> {
    let c = ch?.to_lowercase();
    items.iter().find(|it| !it.disabled && it.key.to_lowercase() == c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<ToolbarItem> {
        vec![
            ToolbarItem::new("n", "ew"),
            ToolbarItem::new("s", "ave").active(true),
            ToolbarItem::new("/", "Search").hint("fuzzy"),
            ToolbarItem::new("q", "uit").disabled(true),
        ]
    }

    /// node: tests/widgets-toolbar.test.ts:11-43
    #[test]
    fn bracket_format() {
        let t = crate::theme::COOL_BLUE;
        let line = toolbar(&t, &items(), &ToolbarOptions::default());
        let flat = line.to_string();
        assert_eq!(flat, "[N]ew  [S]ave  [/]Search fuzzy  [Q]uit");
        assert!(!flat.contains("[n]"));
        let save = line.spans.iter().find(|s| s.content == "S").unwrap();
        assert!(save.style.add_modifier.contains(Modifier::BOLD));
        let quit = line.spans.iter().find(|s| s.content == "uit").unwrap();
        assert!(quit.style.add_modifier.contains(Modifier::DIM));
    }

    /// node: tests/widgets-toolbar.test.ts:45-60
    #[test]
    fn inline_format_and_lookup() {
        let t = crate::theme::COOL_BLUE;
        let opts = ToolbarOptions {
            format: ToolbarFormat::Inline,
            ..Default::default()
        };
        let line = toolbar(&t, &[ToolbarItem::new("n", "new")], &opts);
        let texts: Vec<&str> = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(texts, vec!["", "n", "ew"]);
        let it = items();
        assert_eq!(toolbar_item_for(&it, Some("n")).unwrap().key, "n");
        assert!(toolbar_item_for(&it, Some("Q")).is_none());
        assert!(toolbar_item_for(&it, Some("x")).is_none());
        assert!(toolbar_item_for(&it, None).is_none());
    }
}
