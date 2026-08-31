//! Yes/no modal body (`src/tui/widgets/confirm.ts`). Keys: `left` / `right`
//! / `tab` / `backtab` toggle the focused button, `return` commits it,
//! `escape` is always no, `y`/`n` (any case) commit directly. Default focus
//! is `no`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use super::panel::Panel;
use crate::input::KeyEvent;
use crate::theme::{BoxStyle, Color, Theme};

/// Which button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmChoice {
    Yes,
    No,
}

/// `ConfirmState` (`confirm.ts:13-19`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmState {
    pub title: String,
    pub message: String,
    pub yes_label: String,
    pub no_label: String,
    pub focused: ConfirmChoice,
}

impl ConfirmState {
    /// `createConfirm` with the defaults (`Yes` / `No`, focus `no`).
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        ConfirmState {
            title: title.into(),
            message: message.into(),
            yes_label: "Yes".into(),
            no_label: "No".into(),
            focused: ConfirmChoice::No,
        }
    }

    pub fn labels(mut self, yes: impl Into<String>, no: impl Into<String>) -> Self {
        self.yes_label = yes.into();
        self.no_label = no.into();
        self
    }

    pub fn default_focus(mut self, focus: ConfirmChoice) -> Self {
        self.focused = focus;
        self
    }
}

/// The outcome of a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    Yes,
    No,
    Pending,
}

/// `handleConfirmKey` (`confirm.ts:49-67`).
pub fn handle_confirm_key(state: &ConfirmState, key: &KeyEvent) -> (ConfirmState, ConfirmAction) {
    match key.name.as_str() {
        "left" | "right" | "tab" | "backtab" => {
            let focused = match state.focused {
                ConfirmChoice::Yes => ConfirmChoice::No,
                ConfirmChoice::No => ConfirmChoice::Yes,
            };
            return (
                ConfirmState {
                    focused,
                    ..state.clone()
                },
                ConfirmAction::Pending,
            );
        }
        "return" => {
            let action = match state.focused {
                ConfirmChoice::Yes => ConfirmAction::Yes,
                ConfirmChoice::No => ConfirmAction::No,
            };
            return (state.clone(), action);
        }
        "escape" => return (state.clone(), ConfirmAction::No),
        _ => {}
    }
    match key.ch.as_deref() {
        Some("y") | Some("Y") => (state.clone(), ConfirmAction::Yes),
        Some("n") | Some("N") => (state.clone(), ConfirmAction::No),
        _ => (state.clone(), ConfirmAction::Pending),
    }
}

/// The dialog body (`confirmPanel`, `confirm.ts:70-85`): message, separator,
/// the two buttons, and the key hint. Needs 6 rows.
#[derive(Debug, Clone)]
pub struct ConfirmPanel<'a> {
    pub state: &'a ConfirmState,
    pub theme: Theme,
    pub box_style: BoxStyle,
}

impl<'a> ConfirmPanel<'a> {
    pub fn new(state: &'a ConfirmState, theme: Theme, box_style: BoxStyle) -> Self {
        ConfirmPanel {
            state,
            theme,
            box_style,
        }
    }

    /// The button row.
    pub fn buttons_line(&self) -> Line<'static> {
        let t = &self.theme;
        let muted = Style::default().fg(t.color(Color::Muted));
        let accent = Style::default()
            .fg(t.color(Color::Accent))
            .add_modifier(Modifier::BOLD);
        let s = self.state;
        let yes = Span::styled(
            format!(" {} ", s.yes_label),
            if s.focused == ConfirmChoice::Yes { accent } else { muted },
        );
        let no = Span::styled(
            format!(" {} ", s.no_label),
            if s.focused == ConfirmChoice::No { accent } else { muted },
        );
        Line::from(vec![
            Span::styled("  ", muted),
            yes,
            Span::styled("   ", muted),
            no,
        ])
    }
}

impl Widget for ConfirmPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let panel = Panel::new(self.theme, self.box_style).title(self.state.title.clone());
        let inner = Panel::inner(area);
        panel.clone().render(area, buf);
        if inner.height == 0 {
            return;
        }
        let primary = Style::default().fg(self.theme.color(Color::Primary));
        buf.set_line(inner.x, inner.y, &Line::styled(self.state.message.clone(), primary), inner.width);
        if inner.height > 1 {
            panel.separator(area, inner.y + 1, buf);
        }
        if inner.height > 2 {
            buf.set_line(inner.x, inner.y + 2, &self.buttons_line(), inner.width);
        }
        if inner.height > 3 {
            let hint = Style::default()
                .fg(self.theme.color(Color::Muted))
                .add_modifier(Modifier::DIM);
            buf.set_line(
                inner.x,
                inner.y + 3,
                &Line::styled("  y / n / enter / esc", hint),
                inner.width,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/widgets-confirm.test.ts:11-21
    #[test]
    fn state() {
        assert_eq!(ConfirmState::new("Delete?", "really?").focused, ConfirmChoice::No);
        assert_eq!(
            ConfirmState::new("", "").default_focus(ConfirmChoice::Yes).focused,
            ConfirmChoice::Yes
        );
    }

    /// node: tests/widgets-confirm.test.ts:23-52
    #[test]
    fn keys() {
        let s0 = ConfirmState::new("x", "y");
        let (s1, a) = handle_confirm_key(&s0, &KeyEvent::named("right"));
        assert_eq!((a, s1.focused), (ConfirmAction::Pending, ConfirmChoice::Yes));
        let (s2, a) = handle_confirm_key(&s1, &KeyEvent::named("tab"));
        assert_eq!((a, s2.focused), (ConfirmAction::Pending, ConfirmChoice::No));
        assert_eq!(handle_confirm_key(&s0, &KeyEvent::named("return")).1, ConfirmAction::No);
        let yes = s0.clone().default_focus(ConfirmChoice::Yes);
        assert_eq!(handle_confirm_key(&yes, &KeyEvent::named("return")).1, ConfirmAction::Yes);
        assert_eq!(handle_confirm_key(&yes, &KeyEvent::named("escape")).1, ConfirmAction::No);
        assert_eq!(handle_confirm_key(&s0, &KeyEvent::printable("y")).1, ConfirmAction::Yes);
        assert_eq!(handle_confirm_key(&s0, &KeyEvent::printable("Y")).1, ConfirmAction::Yes);
        assert_eq!(handle_confirm_key(&s0, &KeyEvent::printable("n")).1, ConfirmAction::No);
    }

    /// node: tests/widgets-confirm.test.ts:54-61
    #[test]
    fn renders_panel_with_title() {
        let s = ConfirmState::new("Drop the bomb?", "boom");
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 6));
        ConfirmPanel::new(&s, crate::theme::COOL_BLUE, BoxStyle::Rounded).render(buf.area, &mut buf);
        let row = |y: u16| -> String { (0..30).map(|x| buf[(x, y)].symbol().to_string()).collect() };
        assert!(row(0).contains("Drop the bomb?"));
        assert!(row(1).contains("boom"));
        assert!(row(2).starts_with("├"));
        assert!(row(3).contains("   Yes     No "));
        assert!(row(4).contains("y / n / enter / esc"));
    }
}
