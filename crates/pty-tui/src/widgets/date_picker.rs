//! Date picker (`src/tui/widgets/date-picker.ts`): a month grid plus time.
//! Keys: `left`/`right` ±1 day, `up`/`down` ±7 days, `[`/`]` ±1 month (day
//! clamped), `h`/`H` ±1 hour, `m`/`M` ±5 minutes; anything else (`return`,
//! `escape`) is left to the caller (`None`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use super::canvas::DrawContext;
use super::panel::Panel;
use crate::input::KeyEvent;
use crate::theme::{BoxStyle, Color, Theme};

/// `DatePickerState` (`date-picker.ts:13-19`): month is 0-11, day 1-31.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatePickerState {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
}

const DAY_HEADERS: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

/// `MONTH_NAMES`.
pub const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Days from 1970-01-01 for a civil date (month 1-12).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Civil date (month 1-12) for days from 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Normalise a (year, month0) pair with month overflow, like `new Date`.
fn norm_month(year: i32, month: i32) -> (i32, i32) {
    let total = year as i64 * 12 + month as i64;
    (total.div_euclid(12) as i32, total.rem_euclid(12) as i32)
}

/// `daysInMonth(year, month0)`.
pub fn days_in_month(year: i32, month: i32) -> i32 {
    let (y, m) = norm_month(year, month);
    let (ny, nm) = norm_month(y, m + 1);
    (days_from_civil(ny as i64, nm as i64 + 1, 1) - days_from_civil(y as i64, m as i64 + 1, 1)) as i32
}

/// Day of week (0 = Sunday) of a date (month 0-11).
pub fn day_of_week(year: i32, month: i32, day: i32) -> i32 {
    let days = days_from_civil(year as i64, month as i64 + 1, day as i64);
    ((days + 4).rem_euclid(7)) as i32
}

impl DatePickerState {
    /// `datePickerFromDate` (month 0-11).
    pub fn new(year: i32, month: i32, day: i32, hour: i32, minute: i32) -> Self {
        DatePickerState {
            year,
            month,
            day,
            hour,
            minute,
        }
    }

    /// `clampDay`: keep the day inside the month.
    pub fn clamp_day(self) -> Self {
        let max = days_in_month(self.year, self.month);
        DatePickerState {
            day: self.day.clamp(1, max),
            ..self
        }
    }

    /// `shiftDay`, rolling over months and years.
    pub fn shift_day(self, delta: i32) -> Self {
        let days = days_from_civil(self.year as i64, self.month as i64 + 1, self.day as i64) + delta as i64;
        let (y, m, d) = civil_from_days(days);
        DatePickerState {
            year: y as i32,
            month: m as i32 - 1,
            day: d as i32,
            ..self
        }
    }

    /// `shiftMonth`, day clamped.
    pub fn shift_month(self, delta: i32) -> Self {
        let (y, m) = norm_month(self.year, self.month + delta);
        DatePickerState {
            year: y,
            month: m,
            ..self
        }
        .clamp_day()
    }

    /// `shiftTime("h", delta)`: wraps at 24.
    pub fn shift_hours(self, delta: i32) -> Self {
        DatePickerState {
            hour: (self.hour + delta).rem_euclid(24),
            ..self
        }
    }

    /// `shiftTime("m", delta)`: wraps at 60.
    pub fn shift_minutes(self, delta: i32) -> Self {
        DatePickerState {
            minute: (self.minute + delta).rem_euclid(60),
            ..self
        }
    }

    /// `toDate`: `(year, month0, day, hour, minute)`.
    pub fn to_date(self) -> (i32, i32, i32, i32, i32) {
        (self.year, self.month, self.day, self.hour, self.minute)
    }

    /// `HH:MM`.
    pub fn time_string(self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    /// `<Month> <year>`.
    pub fn heading(self) -> String {
        format!("{} {}", MONTH_NAMES[self.month.rem_euclid(12) as usize], self.year)
    }
}

/// `handleDatePickerKey` (`date-picker.ts:84-101`).
pub fn handle_date_picker_key(state: DatePickerState, key: &KeyEvent) -> Option<DatePickerState> {
    Some(match key.name.as_str() {
        "up" => state.shift_day(-7),
        "down" => state.shift_day(7),
        "left" => state.shift_day(-1),
        "right" => state.shift_day(1),
        "[" => state.shift_month(-1),
        "]" => state.shift_month(1),
        "h" => state.shift_hours(-1),
        "H" => state.shift_hours(1),
        "m" => state.shift_minutes(-5),
        "M" => state.shift_minutes(5),
        _ => return None,
    })
}

/// The 7-row month grid as canvas cells (`calendarCanvas`): headers in
/// bold accent, the selected day in bold accent on accent.
pub fn calendar_draw(state: DatePickerState, ctx: &mut DrawContext) {
    let cell_w = 4;
    for (d, h) in DAY_HEADERS.iter().enumerate() {
        ctx.write(d as i32 * cell_w, 0, h, Some(Color::Accent), None, true);
    }
    let first_dow = day_of_week(state.year, state.month, 1);
    let max = days_in_month(state.year, state.month);
    let mut row = 1;
    let mut col = first_dow;
    for day in 1..=max {
        let x = col * cell_w;
        let label = format!("{day:>2}");
        if day == state.day {
            ctx.write(x, row, &label, Some(Color::Accent), Some(Color::Accent), true);
        } else {
            ctx.write(x, row, &label, Some(Color::Primary), None, false);
        }
        col += 1;
        if col > 6 {
            col = 0;
            row += 1;
        }
    }
}

/// The calendar as a `DrawContext` (28 wide, 7 high).
pub fn calendar_context(state: DatePickerState) -> DrawContext {
    let mut ctx = DrawContext::new(28, 7);
    calendar_draw(state, &mut ctx);
    ctx
}

/// The calendar as text lines (no styling), one per row.
pub fn calendar_lines(state: DatePickerState) -> Vec<String> {
    let ctx = calendar_context(state);
    let mut rows = vec![vec![' '; 28]; 7];
    for c in &ctx.cells {
        if let Some(ch) = c.ch.chars().next() {
            rows[c.y as usize][c.x as usize] = ch;
        }
    }
    rows.into_iter()
        .map(|r| r.into_iter().collect::<String>().trim_end().to_string())
        .collect()
}

/// The hint line text.
pub const DATE_PICKER_HINT: &str =
    "\u{2190}\u{2192}\u{2191}\u{2193} day    [ ] month    h/H hour    m/M \u{b1}5 min    enter ok    esc cancel";

/// The overlay body (`datePickerBody`): heading, the 7 calendar rows, the
/// time line and the hint. Needs 10 rows.
pub struct DatePickerPanel {
    pub state: DatePickerState,
    pub theme: Theme,
    pub box_style: BoxStyle,
    pub title: String,
}

impl DatePickerPanel {
    pub fn new(state: DatePickerState, theme: Theme, box_style: BoxStyle) -> Self {
        DatePickerPanel {
            state,
            theme,
            box_style,
            title: "pick a date".into(),
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Draw only the body (no panel) at `area`.
    pub fn render_body(&self, area: Rect, buf: &mut Buffer) {
        let t = &self.theme;
        let heading = Line::from(Span::styled(
            self.state.heading(),
            Style::default()
                .fg(t.color(Color::Accent))
                .add_modifier(Modifier::BOLD),
        ));
        buf.set_line(area.x, area.y, &heading, area.width);
        if area.height > 1 {
            let cal = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1).min(7));
            calendar_context(self.state).paint(t, cal, buf);
        }
        if area.height > 8 {
            let time = Line::from(Span::styled(
                format!("time  {}", self.state.time_string()),
                Style::default().fg(t.color(Color::Muted)),
            ));
            buf.set_line(area.x, area.y + 8, &time, area.width);
        }
        if area.height > 9 {
            let hint = Line::from(Span::styled(
                DATE_PICKER_HINT,
                Style::default()
                    .fg(t.color(Color::Muted))
                    .add_modifier(Modifier::DIM),
            ));
            buf.set_line(area.x, area.y + 9, &hint, area.width);
        }
    }
}

impl Widget for DatePickerPanel {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Panel::new(self.theme, self.box_style)
            .title(self.title.clone())
            .render(area, buf);
        self.render_body(Panel::inner(area), buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apr20() -> DatePickerState {
        DatePickerState::new(2026, 3, 20, 9, 0)
    }

    /// node: tests/widgets-date-picker.test.ts:15-61
    #[test]
    fn math() {
        assert_eq!(days_in_month(2024, 1), 29);
        assert_eq!(days_in_month(2025, 1), 28);
        assert_eq!(days_in_month(2026, 1), 28);
        let jan30 = DatePickerState::new(2026, 0, 30, 0, 0);
        assert_eq!(jan30.shift_day(3), DatePickerState::new(2026, 1, 2, 0, 0));
        let dec31 = DatePickerState::new(2026, 11, 31, 0, 0);
        assert_eq!(dec31.shift_day(1), DatePickerState::new(2027, 0, 1, 0, 0));
        let jan31 = DatePickerState::new(2026, 0, 31, 0, 0);
        let feb = jan31.shift_month(1);
        assert_eq!((feb.month, feb.day), (1, 28));
        assert_eq!(apr20().shift_hours(-10).hour, 23);
        assert_eq!(apr20().shift_hours(25).hour, 10);
        assert_eq!(apr20().shift_minutes(-5).minute, 55);
        assert_eq!(apr20().shift_minutes(65).minute, 5);
        assert_eq!(DatePickerState::new(2026, 1, 30, 0, 0).clamp_day().day, 28);
        assert_eq!(DatePickerState::new(2026, 1, 0, 0, 0).clamp_day().day, 1);
        let s = apr20();
        let (y, m, d, h, mi) = s.to_date();
        assert_eq!(DatePickerState::new(y, m, d, h, mi), s);
        assert_eq!(DatePickerState::new(2025, 11, 31, 0, 0).shift_month(1).year, 2026);
        assert_eq!(DatePickerState::new(2026, 0, 15, 0, 0).shift_month(-1).month, 11);
    }

    /// node: tests/widgets-date-picker.test.ts:63-91
    #[test]
    fn keys() {
        let s = apr20();
        let k = |n: &str| KeyEvent::named(n);
        assert_eq!(handle_date_picker_key(s, &k("right")).unwrap().day, 21);
        assert_eq!(handle_date_picker_key(s, &k("left")).unwrap().day, 19);
        assert_eq!(handle_date_picker_key(s, &k("down")).unwrap().day, 27);
        assert_eq!(handle_date_picker_key(s, &k("up")).unwrap().day, 13);
        assert_eq!(handle_date_picker_key(s, &k("[")).unwrap().month, 2);
        assert_eq!(handle_date_picker_key(s, &k("]")).unwrap().month, 4);
        assert_eq!(handle_date_picker_key(s, &k("h")).unwrap().hour, 8);
        assert_eq!(handle_date_picker_key(s, &k("H")).unwrap().hour, 10);
        assert_eq!(handle_date_picker_key(s, &k("M")).unwrap().minute, 5);
        assert_eq!(handle_date_picker_key(s, &k("m")).unwrap().minute, 55);
        assert_eq!(handle_date_picker_key(s, &k("x")), None);
        assert_eq!(handle_date_picker_key(s, &k("return")), None);
        assert_eq!(handle_date_picker_key(s, &k("escape")), None);
    }

    /// node: src/tui/widgets/date-picker.ts:104-131
    #[test]
    fn calendar_grid() {
        // April 2026 starts on a Wednesday.
        assert_eq!(day_of_week(2026, 3, 1), 3);
        let lines = calendar_lines(apr20());
        assert_eq!(lines[0], "Su  Mo  Tu  We  Th  Fr  Sa");
        assert_eq!(lines[1], "             1   2   3   4");
        assert_eq!(lines[4], "19  20  21  22  23  24  25");
        assert_eq!(apr20().heading(), "April 2026");
        assert_eq!(apr20().time_string(), "09:00");
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 12));
        DatePickerPanel::new(apr20(), crate::theme::COOL_BLUE, BoxStyle::Rounded).render(buf.area, &mut buf);
        let row = |y: u16| -> String { (0..40).map(|x| buf[(x, y)].symbol().to_string()).collect() };
        assert!(row(0).contains("pick a date"));
        assert!(row(1).contains("April 2026"));
        assert!(row(2).contains("Su  Mo"));
        assert!(row(9).contains("time  09:00"));
    }
}
