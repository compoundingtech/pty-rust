//! Tab strip (`src/tui/widgets/tabs.ts`): `[ Active ]` in bold accent,
//! inactive tabs dim and padded, joined by two spaces. Keys: `ctrl+tab` /
//! `ctrl+backtab` cycle, `1`-`9` jump. Mouse: a left click on a label
//! selects it. Wraps ratatui's `Tabs` for the buffer rendering.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Tabs as RTabs;

use crate::input::{KeyEvent, MouseAction, MouseButton, MouseEvent};
use crate::theme::{Color, Theme};

/// `TabDef<T>` (`tabs.ts:12-17`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabDef<T = ()> {
    pub id: String,
    pub label: String,
    pub data: T,
}

impl TabDef<()> {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        TabDef {
            id: id.into(),
            label: label.into(),
            data: (),
        }
    }
}

/// `TabsState` (`tabs.ts:19-21`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TabsState {
    pub active_id: Option<String>,
}

impl TabsState {
    /// `createTabsState`: `initial` or the first tab.
    pub fn new<T>(tabs: &[TabDef<T>], initial: Option<&str>) -> Self {
        TabsState {
            active_id: initial
                .map(str::to_string)
                .or_else(|| tabs.first().map(|t| t.id.clone())),
        }
    }

    /// `selectTab`.
    pub fn select(&self, id: &str) -> Self {
        TabsState {
            active_id: Some(id.to_string()),
        }
    }

    fn step<T>(&self, tabs: &[TabDef<T>], delta: i64) -> Self {
        let Some(active) = &self.active_id else {
            return self.clone();
        };
        if tabs.is_empty() {
            return self.clone();
        }
        let Some(idx) = tabs.iter().position(|t| &t.id == active) else {
            return self.clone();
        };
        let n = tabs.len() as i64;
        let next = ((idx as i64 + delta) % n + n) % n;
        self.select(&tabs[next as usize].id)
    }

    /// `nextTab`, wrapping.
    pub fn next<T>(&self, tabs: &[TabDef<T>]) -> Self {
        self.step(tabs, 1)
    }

    /// `prevTab`, wrapping.
    pub fn prev<T>(&self, tabs: &[TabDef<T>]) -> Self {
        self.step(tabs, -1)
    }

    /// The active index.
    pub fn active_index<T>(&self, tabs: &[TabDef<T>]) -> Option<usize> {
        let a = self.active_id.as_ref()?;
        tabs.iter().position(|t| &t.id == a)
    }
}

/// `handleTabsKey` (`tabs.ts:50-64`); `None` when not consumed (a plain
/// `tab` is left for forms).
pub fn handle_tabs_key<T>(state: &TabsState, tabs: &[TabDef<T>], key: &KeyEvent) -> Option<TabsState> {
    if key.name == "tab" && key.ctrl {
        return Some(state.next(tabs));
    }
    if key.name == "backtab" && key.ctrl {
        return Some(state.prev(tabs));
    }
    if let Some(ch) = key.ch.as_deref()
        && !key.ctrl
        && !key.alt
        && ch.len() == 1
        && let Some(d) = ch.chars().next().and_then(|c| c.to_digit(10))
        && (1..=9).contains(&d)
        && let Some(t) = tabs.get(d as usize - 1)
    {
        return Some(state.select(&t.id));
    }
    None
}

/// `handleTabsMouse` (`tabs.ts:71-91`): a left press on `rect.y` hits the
/// tab whose `[ label ]` span covers `x`; gaps are ignored.
pub fn handle_tabs_mouse<T>(
    state: &TabsState,
    tabs: &[TabDef<T>],
    event: &MouseEvent,
    rect: Rect,
) -> Option<TabsState> {
    if event.action != MouseAction::Press || event.button != MouseButton::Left {
        return None;
    }
    if event.y != rect.y {
        return None;
    }
    let mut cursor = rect.x as usize;
    for (i, t) in tabs.iter().enumerate() {
        if i > 0 {
            cursor += 2;
        }
        let w = t.label.chars().count() + 4;
        let x = event.x as usize;
        if x >= cursor && x < cursor + w {
            return Some(state.select(&t.id));
        }
        cursor += w;
    }
    None
}

/// The strip as one line (`renderTabs`, `tabs.ts:95-106`).
pub fn render_tabs<T>(theme: &Theme, state: &TabsState, tabs: &[TabDef<T>]) -> Line<'static> {
    let mut spans = Vec::new();
    let muted = Style::default().fg(theme.color(Color::Muted));
    for (i, t) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", muted));
        }
        if Some(&t.id) == state.active_id.as_ref() {
            spans.push(Span::styled(
                format!("[ {} ]", t.label),
                Style::default()
                    .fg(theme.color(Color::Accent))
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!("  {}  ", t.label),
                muted.add_modifier(Modifier::DIM),
            ));
        }
    }
    Line::from(spans)
}

/// A ratatui `Tabs` widget with Node's labels, divider and highlight.
pub fn tabs_widget<'a, T>(theme: &Theme, state: &TabsState, tabs: &'a [TabDef<T>]) -> RTabs<'a> {
    let titles: Vec<Line<'a>> = tabs
        .iter()
        .map(|t| {
            if Some(&t.id) == state.active_id.as_ref() {
                Line::raw(format!("[ {} ]", t.label))
            } else {
                Line::raw(format!("  {}  ", t.label))
            }
        })
        .collect();
    RTabs::new(titles)
        .divider("  ")
        .padding("", "")
        .style(
            Style::default()
                .fg(theme.color(Color::Muted))
                .add_modifier(Modifier::DIM),
        )
        .highlight_style(
            Style::default()
                .fg(theme.color(Color::Accent))
                .add_modifier(Modifier::BOLD)
                .remove_modifier(Modifier::DIM),
        )
        .select(state.active_index(tabs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs() -> Vec<TabDef> {
        vec![
            TabDef::new("inbox", "Inbox"),
            TabDef::new("sent", "Sent"),
            TabDef::new("drafts", "Drafts"),
        ]
    }

    /// node: tests/widgets-tabs.test.ts:17-35
    #[test]
    fn state() {
        let t = tabs();
        assert_eq!(TabsState::new(&t, None).active_id.as_deref(), Some("inbox"));
        assert_eq!(TabsState::new(&t, None).select("sent").active_id.as_deref(), Some("sent"));
        let s0 = TabsState::new(&t, None);
        let s1 = s0.next(&t);
        let s2 = s1.next(&t);
        let s3 = s2.next(&t);
        assert_eq!(
            [&s1, &s2, &s3].map(|s| s.active_id.clone().unwrap()),
            ["sent", "drafts", "inbox"]
        );
        assert_eq!(s0.prev(&t).active_id.as_deref(), Some("drafts"));
    }

    /// node: tests/widgets-tabs.test.ts:37-58
    #[test]
    fn keys() {
        let t = tabs();
        let s0 = TabsState::new(&t, None);
        assert_eq!(
            handle_tabs_key(&s0, &t, &KeyEvent::named("tab").with_ctrl()).unwrap().active_id.as_deref(),
            Some("sent")
        );
        assert_eq!(
            handle_tabs_key(&s0, &t, &KeyEvent::named("backtab").with_ctrl()).unwrap().active_id.as_deref(),
            Some("drafts")
        );
        assert_eq!(
            handle_tabs_key(&s0, &t, &KeyEvent::printable("3")).unwrap().active_id.as_deref(),
            Some("drafts")
        );
        assert_eq!(handle_tabs_key(&s0, &t, &KeyEvent::named("x")), None);
        assert_eq!(handle_tabs_key(&s0, &t, &KeyEvent::named("tab")), None);
    }

    /// node: tests/widgets-tabs.test.ts:60-72
    #[test]
    fn rendering() {
        let t = tabs();
        let line = render_tabs(&crate::theme::COOL_BLUE, &TabsState::new(&t, None), &t);
        assert_eq!(line.to_string(), "[ Inbox ]    Sent      Drafts  ");
        let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 40, 1));
        ratatui::widgets::Widget::render(
            tabs_widget(&crate::theme::COOL_BLUE, &TabsState::new(&t, None), &t),
            buf.area,
            &mut buf,
        );
        let row: String = (0..31).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert_eq!(row, "[ Inbox ]    Sent      Drafts  ");
    }

    fn me(x: u16, y: u16, action: MouseAction, button: MouseButton) -> MouseEvent {
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

    /// node: tests/widgets-tabs-mouse.test.ts:20-50
    #[test]
    fn mouse() {
        let t = tabs();
        let rect = Rect::new(0, 2, 40, 1);
        let s = TabsState {
            active_id: Some("sent".into()),
        };
        let press = |x, y| me(x, y, MouseAction::Press, MouseButton::Left);
        assert_eq!(handle_tabs_mouse(&s, &t, &press(3, 2), rect).unwrap().active_id.as_deref(), Some("inbox"));
        let s = TabsState::new(&t, None);
        assert_eq!(handle_tabs_mouse(&s, &t, &press(15, 2), rect).unwrap().active_id.as_deref(), Some("sent"));
        assert_eq!(handle_tabs_mouse(&s, &t, &press(10, 2), rect), None);
        assert_eq!(handle_tabs_mouse(&s, &t, &press(3, 5), rect), None);
        assert_eq!(handle_tabs_mouse(&s, &t, &me(3, 2, MouseAction::Release, MouseButton::Left), rect), None);
        assert_eq!(handle_tabs_mouse(&s, &t, &me(3, 2, MouseAction::Press, MouseButton::Right), rect), None);
    }
}
