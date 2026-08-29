//! Toast queue (`src/tui/widgets/toast.ts`): ephemeral notifications that
//! expire. `push` appends with `expires_at = now + duration` (default 3 s);
//! `prune_expired` drops what has passed; `dismiss` removes by id. Each
//! toast renders as ` <glyph> text`: `●` info (accent), `✓` success (ok),
//! `⚠` warn, `✗` error.

use std::sync::atomic::{AtomicU64, Ordering};

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{Color, Theme};

/// `ToastKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToastKind {
    #[default]
    Info,
    Success,
    Warn,
    Error,
}

impl ToastKind {
    /// The semantic colour of the glyph (`kindColor`).
    pub fn color(self) -> Color {
        match self {
            ToastKind::Success => Color::Ok,
            ToastKind::Warn => Color::Warn,
            ToastKind::Error => Color::Error,
            ToastKind::Info => Color::Accent,
        }
    }

    /// The glyph (`kindGlyph`).
    pub fn glyph(self) -> &'static str {
        match self {
            ToastKind::Success => "\u{2713}",
            ToastKind::Warn => "\u{26a0}",
            ToastKind::Error => "\u{2717}",
            ToastKind::Info => "\u{25cf}",
        }
    }
}

/// `Toast` (`toast.ts:15-22`). Times are epoch milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub id: String,
    pub kind: ToastKind,
    pub text: String,
    pub expires_at: u64,
}

/// `ToastQueue`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToastQueue {
    pub toasts: Vec<Toast>,
}

/// `pushToast` options.
#[derive(Debug, Clone, Copy, Default)]
pub struct PushToastOptions {
    pub kind: ToastKind,
    /// Default 3000.
    pub duration_ms: Option<u64>,
    /// Default: the wall clock.
    pub now: Option<u64>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl ToastQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// `pushToast`.
    pub fn push(&self, message: impl Into<String>, opts: PushToastOptions) -> ToastQueue {
        let now = opts.now.unwrap_or_else(now_ms);
        let n = NEXT_ID.fetch_add(1, Ordering::SeqCst) + 1;
        let mut toasts = self.toasts.clone();
        toasts.push(Toast {
            id: format!("toast-{n}"),
            kind: opts.kind,
            text: message.into(),
            expires_at: now + opts.duration_ms.unwrap_or(3000),
        });
        ToastQueue { toasts }
    }

    /// `pruneExpired`: drop toasts whose `expires_at <= now`.
    pub fn prune_expired(&self, now: Option<u64>) -> ToastQueue {
        let t = now.unwrap_or_else(now_ms);
        ToastQueue {
            toasts: self.toasts.iter().filter(|x| x.expires_at > t).cloned().collect(),
        }
    }

    /// `dismissToast`.
    pub fn dismiss(&self, id: &str) -> ToastQueue {
        ToastQueue {
            toasts: self.toasts.iter().filter(|t| t.id != id).cloned().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    /// One line per toast (`renderToasts`).
    pub fn render(&self, theme: &Theme) -> Vec<Line<'static>> {
        self.toasts
            .iter()
            .map(|t| {
                Line::from(vec![
                    Span::styled(
                        format!(" {} ", t.kind.glyph()),
                        Style::default()
                            .fg(theme.color(t.kind.color()))
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(t.text.clone(), Style::default().fg(theme.color(Color::Primary))),
                ])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(now: u64, duration_ms: u64) -> PushToastOptions {
        PushToastOptions {
            now: Some(now),
            duration_ms: Some(duration_ms),
            ..Default::default()
        }
    }

    /// node: tests/widgets-toast.test.ts:6-49
    #[test]
    fn queue() {
        let q = ToastQueue::new().push("saved", at(1000, 2000));
        assert_eq!(q.toasts.len(), 1);
        assert_eq!(q.toasts[0].text, "saved");
        assert_eq!(q.toasts[0].expires_at, 3000);
        let q0 = ToastQueue::new().push("a", at(1000, 500)).push("b", at(1000, 2000));
        let q1 = q0.prune_expired(Some(1600));
        assert_eq!(q1.toasts.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), vec!["b"]);
        let q0 = ToastQueue::new().push("x", at(1000, 5000));
        assert_eq!(q0.prune_expired(Some(1100)), q0);
        let q0 = ToastQueue::new().push("a", at(0, 3000));
        let id = q0.toasts[0].id.clone();
        assert!(q0.dismiss(&id).is_empty());
        let q = ToastQueue::new().push(
            "boom",
            PushToastOptions {
                kind: ToastKind::Error,
                now: Some(0),
                ..Default::default()
            },
        );
        let t = crate::theme::COOL_BLUE;
        let lines = q.render(&t);
        assert_eq!(lines[0].spans[0].style.fg, Some(t.color(Color::Error)));
        assert_eq!(lines[0].to_string(), " ✗ boom");
        let q = ToastQueue::new().push("one", at(0, 3000)).push("two", at(0, 3000));
        assert_eq!(q.render(&t).len(), 2);
    }
}
