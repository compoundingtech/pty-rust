//! Windowed list (`src/tui/widgets/virtual-list.ts`): only the visible
//! slice is rendered. Keys: `up`/`down`, `pageup`/`pagedown` (one viewport),
//! `home`/`end`, `return` activates. Mouse: wheel moves the selection by 3,
//! a left click selects and activates the row under the pointer.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;

use crate::input::{KeyEvent, MouseAction, MouseButton, MouseEvent};
use crate::theme::{Color, Theme};

/// `VirtualListState` (`virtual-list.ts:13-22`). `selected` is `None` for
/// an empty list (Node's `-1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualListState {
    pub total: usize,
    pub selected: Option<usize>,
    pub offset: usize,
    pub viewport: usize,
}

/// The half-open index window to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualWindow {
    pub start: usize,
    pub end: usize,
}

impl VirtualListState {
    /// `createVirtualListState`.
    pub fn new(total: usize, viewport: usize) -> Self {
        VirtualListState {
            total,
            selected: (total > 0).then_some(0),
            offset: 0,
            viewport: viewport.max(1),
        }
    }

    /// Re-normalise after a total/viewport change (`clampVirtual`).
    pub fn clamp(self) -> Self {
        let viewport = self.viewport.max(1);
        if self.total == 0 {
            return VirtualListState {
                total: 0,
                selected: None,
                offset: 0,
                viewport,
            };
        }
        let sel = self.selected.unwrap_or(0).min(self.total - 1);
        let max_offset = self.total.saturating_sub(viewport);
        let mut offset = self.offset.min(max_offset);
        if sel < offset {
            offset = sel;
        }
        if sel >= offset + viewport {
            offset = sel + 1 - viewport;
        }
        VirtualListState {
            total: self.total,
            selected: Some(sel),
            offset,
            viewport,
        }
    }

    /// `virtualWindow`.
    pub fn window(self) -> VirtualWindow {
        let s = self.clamp();
        VirtualWindow {
            start: s.offset,
            end: s.total.min(s.offset + s.viewport),
        }
    }

    /// `moveVirtualSelection`.
    pub fn move_by(self, delta: i64) -> Self {
        if self.total == 0 {
            return self;
        }
        let cur = self.selected.unwrap_or(0) as i64;
        let target = (cur + delta).clamp(0, self.total as i64 - 1) as usize;
        if Some(target) == self.selected {
            return self;
        }
        VirtualListState {
            selected: Some(target),
            ..self
        }
        .clamp()
    }

    /// `pageVirtual`.
    pub fn page(self, delta: i64) -> Self {
        self.move_by(delta * self.viewport as i64)
    }

    /// `jumpVirtualToStart`.
    pub fn to_start(self) -> Self {
        VirtualListState {
            selected: Some(0),
            ..self
        }
        .clamp()
    }

    /// `jumpVirtualToEnd`.
    pub fn to_end(self) -> Self {
        VirtualListState {
            selected: Some(self.total.saturating_sub(1)),
            ..self
        }
        .clamp()
    }

    /// A different total, re-normalised.
    pub fn with_total(self, total: usize) -> Self {
        VirtualListState { total, ..self }.clamp()
    }
}

/// What a key or click did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualAction {
    Moved,
    Activate,
    None,
}

/// `handleVirtualKey` (`virtual-list.ts:130-141`).
pub fn handle_virtual_key(state: VirtualListState, key: &KeyEvent) -> (VirtualListState, VirtualAction) {
    match key.name.as_str() {
        "up" => (state.move_by(-1), VirtualAction::Moved),
        "down" => (state.move_by(1), VirtualAction::Moved),
        "pageup" => (state.page(-1), VirtualAction::Moved),
        "pagedown" => (state.page(1), VirtualAction::Moved),
        "home" => (state.to_start(), VirtualAction::Moved),
        "end" => (state.to_end(), VirtualAction::Moved),
        "return" => (state, VirtualAction::Activate),
        _ => (state, VirtualAction::None),
    }
}

/// `handleVirtualMouse` (`virtual-list.ts:101-125`): wheel ±3, left press
/// selects and activates the row under the pointer, anything outside
/// `rect` (or past the last row) is `None`.
pub fn handle_virtual_mouse(
    state: VirtualListState,
    event: &MouseEvent,
    rect: Rect,
) -> (VirtualListState, VirtualAction) {
    let inside = event.x >= rect.x
        && event.x < rect.x + rect.width
        && event.y >= rect.y
        && event.y < rect.y + rect.height;
    if !inside {
        return (state, VirtualAction::None);
    }
    match event.action {
        MouseAction::ScrollUp => (state.move_by(-3), VirtualAction::Moved),
        MouseAction::ScrollDown => (state.move_by(3), VirtualAction::Moved),
        MouseAction::Press if event.button == MouseButton::Left => {
            let row = (event.y - rect.y) as usize + state.offset;
            if row >= state.total {
                return (state, VirtualAction::None);
            }
            (
                VirtualListState {
                    selected: Some(row),
                    ..state
                }
                .clamp(),
                VirtualAction::Activate,
            )
        }
        _ => (state, VirtualAction::None),
    }
}

/// The visible rows as lines; `render_item(index, selected)` is called only
/// for indexes in the window. An empty list renders a dim `(empty)`.
pub fn render_virtual_list(
    theme: &Theme,
    state: VirtualListState,
    mut render_item: impl FnMut(usize, bool) -> Line<'static>,
) -> Vec<Line<'static>> {
    let win = state.window();
    let mut lines: Vec<Line<'static>> = (win.start..win.end)
        .map(|i| render_item(i, Some(i) == state.selected))
        .collect();
    if lines.is_empty() {
        lines.push(Line::styled(
            "(empty)",
            Style::default()
                .fg(theme.color(Color::Muted))
                .add_modifier(Modifier::DIM),
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(s: usize, e: usize) -> VirtualWindow {
        VirtualWindow { start: s, end: e }
    }

    /// node: tests/widgets-virtual-list.test.ts:16-48
    #[test]
    fn windowing() {
        let s = VirtualListState::new(100, 10);
        assert_eq!(s.window(), win(0, 10));
        let s1 = s.move_by(15);
        assert_eq!(s1.selected, Some(15));
        assert_eq!(s1.window(), win(6, 16));
        let s1 = s.to_end();
        assert_eq!(s1.selected, Some(99));
        assert_eq!(s1.window(), win(90, 100));
        let e = VirtualListState::new(0, 10);
        assert_eq!(e.selected, None);
        assert_eq!(e.window(), win(0, 0));
        let s2 = s.to_end().with_total(20);
        assert_eq!(s2.selected, Some(19));
        assert_eq!(s2.window(), win(10, 20));
    }

    /// node: tests/widgets-virtual-list.test.ts:50-72
    #[test]
    fn navigation() {
        let s = VirtualListState::new(100, 10);
        let s1 = s.page(1);
        assert_eq!(s1.selected, Some(10));
        assert_eq!(s1.page(-1).selected, Some(0));
        let down = handle_virtual_key(s, &KeyEvent::named("end")).0;
        assert_eq!(down.selected, Some(99));
        assert_eq!(handle_virtual_key(down, &KeyEvent::named("home")).0.selected, Some(0));
        let (st, action) = handle_virtual_key(s, &KeyEvent::named("return"));
        assert_eq!(action, VirtualAction::Activate);
        assert_eq!(st, s);
    }

    /// node: tests/widgets-virtual-list.test.ts:74-88
    #[test]
    fn renders_only_the_window() {
        let s = VirtualListState::new(1000, 5).move_by(50);
        let mut touched = Vec::new();
        let lines = render_virtual_list(&crate::theme::COOL_BLUE, s, |i, sel| {
            touched.push(i);
            Line::raw(format!("row {i}{}", if sel { " *" } else { "" }))
        });
        assert_eq!(touched.len(), 5);
        assert!(touched.contains(&50));
        assert!(*touched.iter().min().unwrap() >= 46);
        assert!(*touched.iter().max().unwrap() <= 54);
        assert!(lines.iter().any(|l| l.to_string() == "row 50 *"));
        let empty = render_virtual_list(&crate::theme::COOL_BLUE, VirtualListState::new(0, 3), |_, _| {
            Line::raw("x")
        });
        assert_eq!(empty[0].to_string(), "(empty)");
    }

    fn me(action: MouseAction, x: u16, y: u16, button: MouseButton) -> MouseEvent {
        MouseEvent {
            action,
            button,
            x,
            y,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    /// node: tests/widgets-virtual-list-mouse.test.ts:19-53
    #[test]
    fn mouse() {
        let rect = Rect::new(2, 3, 40, 10);
        let s = VirtualListState::new(100, 10);
        assert_eq!(
            handle_virtual_mouse(s, &me(MouseAction::Press, 0, 0, MouseButton::Left), rect).1,
            VirtualAction::None
        );
        let s = VirtualListState {
            offset: 20,
            selected: Some(20),
            ..s
        };
        let (st, a) = handle_virtual_mouse(s, &me(MouseAction::Press, 5, 5, MouseButton::Left), rect);
        assert_eq!(a, VirtualAction::Activate);
        assert_eq!(st.selected, Some(22));
        let s = VirtualListState {
            selected: Some(20),
            offset: 15,
            ..VirtualListState::new(100, 10)
        };
        let (st, a) = handle_virtual_mouse(s, &me(MouseAction::ScrollUp, 5, 5, MouseButton::None), rect);
        assert_eq!((a, st.selected), (VirtualAction::Moved, Some(17)));
        let (st, a) = handle_virtual_mouse(s, &me(MouseAction::ScrollDown, 5, 5, MouseButton::None), rect);
        assert_eq!((a, st.selected), (VirtualAction::Moved, Some(23)));
        let s = VirtualListState::new(5, 10);
        assert_eq!(
            handle_virtual_mouse(s, &me(MouseAction::Press, 5, 9, MouseButton::Left), rect).1,
            VirtualAction::None
        );
    }
}
