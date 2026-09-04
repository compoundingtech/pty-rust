//! Badge (`src/tui/widgets/badge.ts`): an uppercase, padded status chip on
//! a border-toned fill; a variant colours the label (`ok warn error accent
//! info`), `solid` fills with the variant colour and uses primary text.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::theme::{Color, Theme};

/// `BadgeVariant`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BadgeVariant {
    #[default]
    Neutral,
    Ok,
    Warn,
    Error,
    Accent,
    Info,
}

impl BadgeVariant {
    fn color(self) -> Color {
        match self {
            BadgeVariant::Neutral => Color::Primary,
            BadgeVariant::Ok => Color::Ok,
            BadgeVariant::Warn => Color::Warn,
            BadgeVariant::Error => Color::Error,
            BadgeVariant::Accent => Color::Accent,
            BadgeVariant::Info => Color::Info,
        }
    }
}

/// `BadgeOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadgeOptions {
    pub variant: BadgeVariant,
    pub solid: bool,
    pub uppercase: bool,
    pub bold: bool,
}

impl Default for BadgeOptions {
    fn default() -> Self {
        BadgeOptions {
            variant: BadgeVariant::Neutral,
            solid: false,
            uppercase: true,
            bold: false,
        }
    }
}

/// The resolved chip: text plus semantic fg/bg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadgeSpec {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
}

/// `badge` as a spec (the node contract).
pub fn badge_spec(label: &str, opts: &BadgeOptions) -> BadgeSpec {
    let shown = if opts.uppercase { label.to_uppercase() } else { label.to_string() };
    let text = format!(" {shown} ");
    let variant_color = opts.variant.color();
    if opts.solid && opts.variant != BadgeVariant::Neutral {
        return BadgeSpec {
            text,
            fg: Color::Primary,
            bg: variant_color,
            bold: opts.bold,
        };
    }
    BadgeSpec {
        text,
        fg: variant_color,
        bg: Color::Border,
        bold: opts.bold,
    }
}

/// `badge` as a styled span.
pub fn badge(theme: &Theme, label: &str, opts: &BadgeOptions) -> Span<'static> {
    let spec = badge_spec(label, opts);
    let mut style = Style::default().fg(theme.color(spec.fg)).bg(theme.color(spec.bg));
    if spec.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    Span::styled(spec.text, style)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/badge.test.ts:6-45
    #[test]
    fn node_contract() {
        let d = BadgeOptions::default();
        assert_eq!(badge_spec("live", &d).text, " LIVE ");
        let host = badge_spec("host", &d);
        assert_eq!((host.fg, host.bg), (Color::Primary, Color::Border));
        let ok = badge_spec("available", &BadgeOptions { variant: BadgeVariant::Ok, ..d });
        assert_eq!((ok.fg, ok.bg, ok.text.as_str()), (Color::Ok, Color::Border, " AVAILABLE "));
        let dead = badge_spec("dead", &BadgeOptions { variant: BadgeVariant::Error, solid: true, ..d });
        assert_eq!((dead.fg, dead.bg), (Color::Primary, Color::Error));
        let x = badge_spec("x", &BadgeOptions { variant: BadgeVariant::Neutral, solid: true, ..d });
        assert_eq!((x.fg, x.bg), (Color::Primary, Color::Border));
        assert_eq!(badge_spec("Host", &BadgeOptions { uppercase: false, ..d }).text, " Host ");
        assert!(badge_spec("x", &BadgeOptions { bold: true, ..d }).bold);
    }

    /// node: tests/badge.test.ts:48-69
    #[test]
    fn rendered() {
        let t = crate::theme::COOL_BLUE;
        let span = badge(&t, "ok", &BadgeOptions { variant: BadgeVariant::Ok, ..Default::default() });
        let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 20, 1));
        buf.set_span(0, 0, &span, 20);
        let row: String = (0..20).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(row.contains(" OK "));
        assert_eq!(buf[(1, 0)].fg, ratatui::style::Color::Rgb(80, 200, 120));
        assert_eq!(buf[(1, 0)].bg, ratatui::style::Color::Rgb(50, 60, 85));
        assert_eq!(buf[(0, 0)].bg, ratatui::style::Color::Rgb(50, 60, 85));
    }
}
