//! Stream view (`src/tui/widgets/stream-view.ts`): a chat/log tail pinned
//! to the newest item; scrolling up unpins. Keys: `up`/`down` one item,
//! `pageup`/`pagedown` one viewport, `home` to the top, `end` re-pins.
//! Mouse wheel scrolls by 3. When scrolled back the render appends a dim
//! `— N more below (end to jump) —` hint.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;

use crate::input::{KeyEvent, MouseAction, MouseEvent};
use crate::theme::{Color, Theme};

/// `StreamViewState` (`stream-view.ts:12-15`): items scrolled back from
/// the newest; 0 = pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StreamViewState {
    pub scrollback: usize,
}

impl StreamViewState {
    pub fn new() -> Self {
        Self::default()
    }

    /// `isPinned`.
    pub fn is_pinned(self) -> bool {
        self.scrollback == 0
    }

    /// `streamPin`.
    pub fn pin(self) -> Self {
        StreamViewState { scrollback: 0 }
    }

    /// `streamScrollUp`, clamped to `total - viewport`.
    pub fn scroll_up(self, delta: usize, total: usize, viewport: usize) -> Self {
        let max = total.saturating_sub(viewport);
        StreamViewState {
            scrollback: (self.scrollback + delta).min(max),
        }
    }

    /// `streamScrollDown`, clamped at 0.
    pub fn scroll_down(self, delta: usize) -> Self {
        StreamViewState {
            scrollback: self.scrollback.saturating_sub(delta),
        }
    }

    /// The half-open item window (`streamWindow`).
    pub fn window(self, total: usize, viewport: usize) -> (usize, usize) {
        if total == 0 || viewport == 0 {
            return (0, 0);
        }
        let end = total.saturating_sub(self.scrollback);
        let start = end.saturating_sub(viewport);
        (start, end)
    }
}

/// `handleStreamKey`; `None` when the key is not one of the map.
pub fn handle_stream_key(
    state: StreamViewState,
    key: &KeyEvent,
    total: usize,
    viewport: usize,
) -> Option<StreamViewState> {
    Some(match key.name.as_str() {
        "up" => state.scroll_up(1, total, viewport),
        "down" => state.scroll_down(1),
        "pageup" => state.scroll_up(viewport, total, viewport),
        "pagedown" => state.scroll_down(viewport),
        "end" => state.pin(),
        "home" => state.scroll_up(total, total, viewport),
        _ => return None,
    })
}

/// `handleStreamMouse`: wheel inside `rect` scrolls by 3.
pub fn handle_stream_mouse(
    state: StreamViewState,
    event: &MouseEvent,
    rect: Rect,
    total: usize,
    viewport: usize,
) -> Option<StreamViewState> {
    let inside = event.x >= rect.x
        && event.x < rect.x + rect.width
        && event.y >= rect.y
        && event.y < rect.y + rect.height;
    if !inside {
        return None;
    }
    match event.action {
        MouseAction::ScrollUp => Some(state.scroll_up(3, total, viewport)),
        MouseAction::ScrollDown => Some(state.scroll_down(3)),
        _ => None,
    }
}

/// The visible items as lines plus the "more below" hint
/// (`renderStreamView`, `stream-view.ts:97-122`).
pub fn render_stream_view<T>(
    theme: &Theme,
    items: &[T],
    state: StreamViewState,
    viewport: usize,
    mut render_item: impl FnMut(&T, usize) -> Line<'static>,
) -> Vec<Line<'static>> {
    let total = items.len();
    let (start, end) = state.window(total, viewport);
    let mut lines: Vec<Line<'static>> = (start..end).map(|i| render_item(&items[i], i)).collect();
    if !state.is_pinned() {
        let behind = total - end;
        if behind > 0 {
            lines.push(Line::styled(
                format!("\u{2014} {behind} more below (end to jump) \u{2014}"),
                Style::default()
                    .fg(theme.color(Color::Accent))
                    .add_modifier(Modifier::DIM),
            ));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/widgets-stream-view.test.ts:13-68
    #[test]
    fn pinning_and_scrollback() {
        let s = StreamViewState::new();
        assert!(s.is_pinned());
        assert_eq!(s.window(10, 3), (7, 10));
        let s1 = s.scroll_up(1, 10, 3);
        assert!(!s1.is_pinned());
        assert_eq!(s1.window(10, 3), (6, 9));
        let s2 = s1.scroll_up(1000, 10, 3);
        assert_eq!(s2.scrollback, 7);
        assert_eq!(s2.window(10, 3), (0, 3));
        let s0 = StreamViewState::new().scroll_up(5, 10, 3);
        let s1 = s0.scroll_down(2);
        assert_eq!(s1.scrollback, 3);
        let s2 = s1.scroll_down(999);
        assert_eq!(s2.scrollback, 0);
        assert!(s2.is_pinned());
        let s0 = StreamViewState::new().scroll_up(3, 20, 5);
        let s1 = handle_stream_key(s0, &KeyEvent::named("end"), 20, 5).unwrap();
        assert!(s1.is_pinned());
        assert_eq!(handle_stream_key(s0, &KeyEvent::named("x"), 20, 5), None);

        let items: Vec<String> = (0..10).map(|i| format!("item-{i}")).collect();
        let s = StreamViewState::new().scroll_up(3, 10, 3);
        let lines = render_stream_view(&crate::theme::COOL_BLUE, &items, s, 3, |it, _| Line::raw(it.clone()));
        assert!(lines.last().unwrap().to_string().contains("more below"));
        let lines = render_stream_view(&crate::theme::COOL_BLUE, &items, StreamViewState::new(), 3, |it, _| {
            Line::raw(it.clone())
        });
        assert_eq!(lines.last().unwrap().to_string(), "item-9");
    }

    #[test]
    fn wheel_scrolls_inside_rect() {
        let rect = Rect::new(0, 0, 10, 5);
        let ev = MouseEvent {
            action: MouseAction::ScrollUp,
            button: crate::input::MouseButton::None,
            x: 1,
            y: 1,
            ctrl: false,
            alt: false,
            shift: false,
        };
        let s = handle_stream_mouse(StreamViewState::new(), &ev, rect, 20, 5).unwrap();
        assert_eq!(s.scrollback, 3);
        let outside = MouseEvent { x: 50, ..ev };
        assert_eq!(handle_stream_mouse(s, &outside, rect, 20, 5), None);
    }
}
