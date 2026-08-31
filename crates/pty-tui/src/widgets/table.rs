//! Sortable table (`src/tui/widgets/table.ts`). Keys: `up`/`down`,
//! `pageup`/`pagedown` (10 rows), `home`/`end`, `return` activates, `1`-`9`
//! sort by that column (again flips the direction). The header shows ` ▲`
//! / ` ▼` on the sort column; a dim rule separates it from the rows; the
//! selected row is bold accent. Column widths are explicit or the widest
//! of header and cells. Also a ratatui `Table` wrapper with the same look.

use ratatui::layout::Constraint;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row, Table as RTable};

use crate::input::KeyEvent;
use crate::theme::{Color, Theme};

/// Cell alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableAlign {
    #[default]
    Left,
    Right,
}

/// A sort key: strings and numbers compare within their kind
/// (`getSortValue`).
#[derive(Debug, Clone, PartialEq)]
pub enum SortValue {
    Str(String),
    Num(f64),
}

impl PartialOrd for SortValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (SortValue::Str(a), SortValue::Str(b)) => a.partial_cmp(b),
            (SortValue::Num(a), SortValue::Num(b)) => a.partial_cmp(b),
            // JS compares mixed kinds after coercion; string wins here.
            (SortValue::Str(a), SortValue::Num(b)) => a.partial_cmp(&b.to_string()),
            (SortValue::Num(a), SortValue::Str(b)) => a.to_string().partial_cmp(b),
        }
    }
}

/// A cell renderer.
pub type CellRender<R> = Box<dyn Fn(&R) -> String>;
/// A sort-key extractor.
pub type SortKey<R> = Box<dyn Fn(&R) -> SortValue>;

/// `TableColumn<Row>` (`table.ts:12-22`).
pub struct TableColumn<R> {
    pub id: String,
    pub header: String,
    pub render: CellRender<R>,
    pub sort_value: Option<SortKey<R>>,
    pub align: TableAlign,
    pub width: Option<usize>,
}

impl<R> TableColumn<R> {
    pub fn new(id: impl Into<String>, header: impl Into<String>, render: impl Fn(&R) -> String + 'static) -> Self {
        TableColumn {
            id: id.into(),
            header: header.into(),
            render: Box::new(render),
            sort_value: None,
            align: TableAlign::Left,
            width: None,
        }
    }

    pub fn sort_by(mut self, f: impl Fn(&R) -> SortValue + 'static) -> Self {
        self.sort_value = Some(Box::new(f));
        self
    }

    pub fn align(mut self, align: TableAlign) -> Self {
        self.align = align;
        self
    }

    pub fn width(mut self, width: usize) -> Self {
        self.width = Some(width);
        self
    }

    fn value_of(&self, r: &R) -> SortValue {
        match &self.sort_value {
            Some(f) => f(r),
            None => SortValue::Str((self.render)(r)),
        }
    }
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDirection {
    #[default]
    Asc,
    Desc,
}

/// `TableState` (`table.ts:24-28`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableState {
    pub sort_column_id: Option<String>,
    pub sort_direction: SortDirection,
    pub selected: usize,
}

impl TableState {
    /// `createTableState`: sorts by `initial_sort_id` or the first column.
    pub fn new<R>(columns: &[TableColumn<R>], initial_sort_id: Option<&str>) -> Self {
        TableState {
            sort_column_id: initial_sort_id
                .map(str::to_string)
                .or_else(|| columns.first().map(|c| c.id.clone())),
            sort_direction: SortDirection::Asc,
            selected: 0,
        }
    }
}

/// A stable sort per `state` (`sortRows`); returns indexes into `rows`.
pub fn sort_rows<'a, R>(rows: &'a [R], columns: &[TableColumn<R>], state: &TableState) -> Vec<&'a R> {
    let col = state
        .sort_column_id
        .as_ref()
        .and_then(|id| columns.iter().find(|c| &c.id == id));
    let Some(col) = col else {
        return rows.iter().collect();
    };
    let mut indexed: Vec<(usize, &R, SortValue)> =
        rows.iter().enumerate().map(|(i, r)| (i, r, col.value_of(r))).collect();
    indexed.sort_by(|a, b| {
        let ord = a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal);
        let ord = match state.sort_direction {
            SortDirection::Asc => ord,
            SortDirection::Desc => ord.reverse(),
        };
        ord.then(a.0.cmp(&b.0))
    });
    indexed.into_iter().map(|(_, r, _)| r).collect()
}

/// What a key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableAction {
    Moved,
    Sorted,
    Activate,
    None,
}

/// `handleTableKey` (`table.ts:77-114`): the new state, the action, and the
/// activated row index (into the sorted rows) for `Activate`.
pub fn handle_table_key<R>(
    state: &TableState,
    sorted_len: usize,
    columns: &[TableColumn<R>],
    key: &KeyEvent,
) -> (TableState, TableAction, Option<usize>) {
    let clamp = |i: i64| i.clamp(0, sorted_len.saturating_sub(1) as i64) as usize;
    let sel = state.selected as i64;
    let moved = |selected: usize| {
        (
            TableState {
                selected,
                ..state.clone()
            },
            TableAction::Moved,
            None,
        )
    };
    match key.name.as_str() {
        "up" => return moved(clamp(sel - 1)),
        "down" => return moved(clamp(sel + 1)),
        "pageup" => return moved(clamp(sel - 10)),
        "pagedown" => return moved(clamp(sel + 10)),
        "home" => return moved(0),
        "end" => return moved(clamp(sorted_len as i64 - 1)),
        "return" => {
            let activated = (state.selected < sorted_len).then_some(state.selected);
            return (state.clone(), TableAction::Activate, activated);
        }
        _ => {}
    }
    if let Some(ch) = key.ch.as_deref()
        && !key.ctrl
        && !key.alt
        && ch.len() == 1
        && let Some(d) = ch.chars().next().and_then(|c| c.to_digit(10))
        && (1..=9).contains(&d)
        && let Some(col) = columns.get(d as usize - 1)
    {
        if state.sort_column_id.as_deref() == Some(&col.id) {
            let dir = match state.sort_direction {
                SortDirection::Asc => SortDirection::Desc,
                SortDirection::Desc => SortDirection::Asc,
            };
            return (
                TableState {
                    sort_direction: dir,
                    ..state.clone()
                },
                TableAction::Sorted,
                None,
            );
        }
        return (
            TableState {
                sort_column_id: Some(col.id.clone()),
                sort_direction: SortDirection::Asc,
                ..state.clone()
            },
            TableAction::Sorted,
            None,
        );
    }
    (state.clone(), TableAction::None, None)
}

/// Explicit widths or the widest of header and cells (`columnWidths`).
pub fn column_widths<R>(rows: &[&R], columns: &[TableColumn<R>]) -> Vec<usize> {
    columns
        .iter()
        .map(|col| {
            if let Some(w) = col.width {
                return w;
            }
            let mut w = col.header.chars().count();
            for r in rows {
                w = w.max((col.render)(r).chars().count());
            }
            w
        })
        .collect()
}

/// `padCell`: pad or cut to `w` characters.
pub fn pad_cell(s: &str, w: usize, align: TableAlign) -> String {
    let len = s.chars().count();
    if len >= w {
        return s.chars().take(w).collect();
    }
    let pad = " ".repeat(w - len);
    match align {
        TableAlign::Right => format!("{pad}{s}"),
        TableAlign::Left => format!("{s}{pad}"),
    }
}

fn arrow(state: &TableState, col_id: &str) -> &'static str {
    if state.sort_column_id.as_deref() != Some(col_id) {
        return "  ";
    }
    match state.sort_direction {
        SortDirection::Asc => " \u{25b2}",
        SortDirection::Desc => " \u{25bc}",
    }
}

/// Header, rule and body as lines (`renderTable`, `table.ts:132-168`).
pub fn render_table<R>(
    theme: &Theme,
    sorted: &[&R],
    columns: &[TableColumn<R>],
    state: &TableState,
) -> Vec<Line<'static>> {
    let widths = column_widths(sorted, columns);
    let accent_bold = Style::default()
        .fg(theme.color(Color::Accent))
        .add_modifier(Modifier::BOLD);
    let dim = Style::default()
        .fg(theme.color(Color::Muted))
        .add_modifier(Modifier::DIM);
    let header: Vec<Span<'static>> = columns
        .iter()
        .zip(&widths)
        .map(|(col, w)| {
            let label = pad_cell(&format!("{}{}", col.header, arrow(state, &col.id)), w + 2, col.align);
            Span::styled(label, accent_bold)
        })
        .collect();
    let rule: Vec<Span<'static>> = widths
        .iter()
        .map(|w| Span::styled(format!("{}  ", "\u{2500}".repeat(*w)), dim))
        .collect();
    let mut lines = vec![Line::from(header), Line::from(rule)];
    for (idx, r) in sorted.iter().enumerate() {
        let selected = idx == state.selected;
        let mut style = Style::default().fg(theme.color(if selected {
            Color::Accent
        } else {
            Color::Primary
        }));
        if selected {
            style = style.add_modifier(Modifier::BOLD);
        }
        let cells: Vec<Span<'static>> = columns
            .iter()
            .zip(&widths)
            .map(|(col, w)| Span::styled(format!("{}  ", pad_cell(&(col.render)(r), *w, col.align)), style))
            .collect();
        lines.push(Line::from(cells));
    }
    lines
}

/// A ratatui `Table` with Node's header arrows, widths and highlight.
pub fn table_widget<'a, R>(
    theme: &Theme,
    sorted: &[&R],
    columns: &[TableColumn<R>],
    state: &TableState,
) -> RTable<'a> {
    let widths = column_widths(sorted, columns);
    let header = Row::new(
        columns
            .iter()
            .map(|col| Cell::from(format!("{}{}", col.header, arrow(state, &col.id))))
            .collect::<Vec<_>>(),
    )
    .style(
        Style::default()
            .fg(theme.color(Color::Accent))
            .add_modifier(Modifier::BOLD),
    );
    let rows: Vec<Row<'a>> = sorted
        .iter()
        .enumerate()
        .map(|(idx, r)| {
            let selected = idx == state.selected;
            let mut style = Style::default().fg(theme.color(if selected {
                Color::Accent
            } else {
                Color::Primary
            }));
            if selected {
                style = style.add_modifier(Modifier::BOLD);
            }
            Row::new(
                columns
                    .iter()
                    .map(|col| Cell::from(pad_cell(&(col.render)(r), 0, col.align)))
                    .collect::<Vec<_>>(),
            )
            .style(style)
        })
        .collect();
    let constraints: Vec<Constraint> = widths.iter().map(|w| Constraint::Length(*w as u16 + 2)).collect();
    RTable::new(rows, constraints).header(header).column_spacing(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Person {
        name: &'static str,
        age: i64,
    }

    fn cols() -> Vec<TableColumn<Person>> {
        vec![
            TableColumn::new("name", "Name", |p: &Person| p.name.to_string()),
            TableColumn::new("age", "Age", |p: &Person| p.age.to_string())
                .sort_by(|p| SortValue::Num(p.age as f64))
                .align(TableAlign::Right),
        ]
    }

    fn people() -> Vec<Person> {
        vec![
            Person { name: "Bea", age: 30 },
            Person { name: "Alex", age: 25 },
            Person { name: "Dan", age: 40 },
            Person { name: "Cam", age: 35 },
        ]
    }

    /// node: tests/widgets-table.test.ts:25-51
    #[test]
    fn sorting() {
        let c = cols();
        let p = people();
        let s = TableState::new(&c, None);
        let names: Vec<&str> = sort_rows(&p, &c, &s).iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["Alex", "Bea", "Cam", "Dan"]);
        let s = TableState {
            sort_column_id: Some("age".into()),
            ..s
        };
        let ages: Vec<i64> = sort_rows(&p, &c, &s).iter().map(|r| r.age).collect();
        assert_eq!(ages, vec![25, 30, 35, 40]);
        let s = TableState {
            sort_direction: SortDirection::Desc,
            ..s
        };
        let ages: Vec<i64> = sort_rows(&p, &c, &s).iter().map(|r| r.age).collect();
        assert_eq!(ages, vec![40, 35, 30, 25]);
        let same = vec![
            Person { name: "Alex", age: 25 },
            Person { name: "Bea", age: 25 },
            Person { name: "Cam", age: 25 },
        ];
        let s = TableState {
            sort_direction: SortDirection::Asc,
            ..s
        };
        let names: Vec<&str> = sort_rows(&same, &c, &s).iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["Alex", "Bea", "Cam"]);
    }

    /// node: tests/widgets-table.test.ts:53-82
    #[test]
    fn keys() {
        let c = cols();
        let p = people();
        let s0 = TableState::new(&c, None);
        let (s1, a, _) = handle_table_key(&s0, p.len(), &c, &KeyEvent::printable("2"));
        assert_eq!(a, TableAction::Sorted);
        assert_eq!(s1.sort_column_id.as_deref(), Some("age"));
        assert_eq!(s1.sort_direction, SortDirection::Asc);
        let (s2, _, _) = handle_table_key(&s1, p.len(), &c, &KeyEvent::printable("2"));
        assert_eq!(s2.sort_direction, SortDirection::Desc);
        let (s, a, _) = handle_table_key(&s0, p.len(), &c, &KeyEvent::named("down"));
        assert_eq!((s.selected, a), (1, TableAction::Moved));
        let s0 = TableState { selected: 2, ..s0 };
        let sorted = sort_rows(&p, &c, &s0);
        let (_, a, idx) = handle_table_key(&s0, sorted.len(), &c, &KeyEvent::named("return"));
        assert_eq!(a, TableAction::Activate);
        assert_eq!(sorted[idx.unwrap()].name, "Cam");
    }

    /// node: tests/widgets-table.test.ts:84-99
    #[test]
    fn rendering() {
        let c = cols();
        let p = people();
        let t = crate::theme::COOL_BLUE;
        let s = TableState::new(&c, None);
        let sorted = sort_rows(&p, &c, &s);
        let lines = render_table(&t, &sorted, &c, &s);
        assert_eq!(lines.len(), 2 + p.len());
        let s = TableState::new(&c, Some("age"));
        let sorted = sort_rows(&p, &c, &s);
        let lines = render_table(&t, &sorted, &c, &s);
        let age = lines[0].spans.iter().find(|s| s.content.contains("Age")).unwrap();
        assert!(age.content.contains('▲'));
        assert_eq!(lines[0].to_string(), "Name  Age ▲");
        assert_eq!(lines[2].to_string(), "Alex   25  ");
        let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 20, 6));
        ratatui::widgets::Widget::render(table_widget(&t, &sorted, &c, &s), buf.area, &mut buf);
        let row0: String = (0..20).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(row0.starts_with("Name  Age ▲"), "{row0}");
    }
}
