//! The typed cell grid: what a renderer (the TUI pane, an embedding
//! application) reads instead of parsing VT. Node's `readCells` /
//! `readWrappedFlags` contract (`src/tui/builders.ts:341-430`).

use libghostty_vt::screen::{CellContentTag, CellWide};
use libghostty_vt::style::{StyleColor, Underline};
use libghostty_vt::terminal::{Point, PointCoordinate, Terminal};

/// A cell colour. The palette index is preserved when the child used one
/// (SGR 30-37, 90-97, 38;5;n, 48;5;n) so a re-emitter can let the outer
/// terminal's theme win; truecolor stays RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorSnap {
    /// The terminal's default.
    #[default]
    Default,
    /// Palette entry `n` (0-255).
    Indexed(u8),
    /// 24-bit colour.
    Rgb(u8, u8, u8),
}

/// A cell's width class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Wide {
    /// One column.
    #[default]
    Narrow,
    /// Two columns; the next cell is its [`Wide::Spacer`].
    Wide,
    /// The second half of a wide character (or the padding before one that
    /// wrapped). Nothing to draw.
    Spacer,
}

/// One cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellSnap {
    /// The grapheme cluster (`" "` for an empty cell, `""` for a spacer).
    pub text: String,
    /// Foreground.
    pub fg: ColorSnap,
    /// Background.
    pub bg: ColorSnap,
    /// SGR 1.
    pub bold: bool,
    /// SGR 2.
    pub dim: bool,
    /// SGR 3.
    pub italic: bool,
    /// SGR 4 (any underline style).
    pub underline: bool,
    /// SGR 7.
    pub inverse: bool,
    /// SGR 9.
    pub strikethrough: bool,
    /// Width class.
    pub wide: Wide,
}

impl Default for CellSnap {
    fn default() -> Self {
        CellSnap {
            text: " ".to_string(),
            fg: ColorSnap::Default,
            bg: ColorSnap::Default,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            inverse: false,
            strikethrough: false,
            wide: Wide::Narrow,
        }
    }
}

/// A viewport-sized window onto the buffer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CellGrid {
    /// `rows_n` rows of `cols` cells.
    pub rows: Vec<Vec<CellSnap>>,
    /// Per row: true when the row continues the previous one because the
    /// terminal soft-wrapped a long line (xterm's `isWrapped`).
    pub wrapped: Vec<bool>,
    /// `(row, col, visible)` of the cursor, relative to the live viewport
    /// (not to this window). `col` may equal `cols` when a wrap is pending.
    pub cursor: (u16, u16, bool),
    /// Buffer row where the live viewport starts (Node's `baseY`).
    pub base_y: usize,
    /// Rows in the buffer, history + viewport (Node's `bufferLength`).
    pub len: usize,
    /// Terminal width.
    pub cols: u16,
    /// Terminal height (the number of rows in this grid).
    pub rows_n: u16,
    /// Buffer row this window starts at (`max(0, base_y - scroll_offset)`).
    pub start: usize,
}

impl CellGrid {
    /// Rows as text, cells concatenated (spacers contribute nothing),
    /// joined by `\n`. No trimming.
    pub fn text(&self) -> String {
        self.rows
            .iter()
            .map(|r| r.iter().map(|c| c.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The first cell whose text is exactly `text`.
    pub fn find(&self, text: &str) -> Option<&CellSnap> {
        self.rows.iter().flatten().find(|c| c.text == text)
    }
}

fn color(c: StyleColor) -> ColorSnap {
    match c {
        StyleColor::None => ColorSnap::Default,
        StyleColor::Palette(i) => ColorSnap::Indexed(i.0),
        StyleColor::Rgb(rgb) => ColorSnap::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

fn read_cell(term: &Terminal, point: Point) -> CellSnap {
    let Ok(g) = term.grid_ref(point) else {
        return CellSnap::default();
    };
    let Ok(cell) = g.cell() else {
        return CellSnap::default();
    };
    let wide = match cell.wide() {
        Ok(CellWide::Wide) => Wide::Wide,
        Ok(CellWide::SpacerTail) | Ok(CellWide::SpacerHead) => Wide::Spacer,
        _ => Wide::Narrow,
    };
    let text = if wide == Wide::Spacer {
        String::new()
    } else {
        let mut buf = [char::default(); 16];
        let cluster: String = match g.graphemes(&mut buf) {
            Ok(n) => buf[..n].iter().collect(),
            Err(libghostty_vt::error::Error::OutOfSpace { required }) => {
                let mut big = vec![char::default(); required];
                match g.graphemes(&mut big) {
                    Ok(n) => big[..n].iter().collect(),
                    Err(_) => String::new(),
                }
            }
            Err(_) => String::new(),
        };
        if cluster.is_empty() || cluster.starts_with('\0') {
            " ".to_string()
        } else {
            cluster
        }
    };
    let style = g.style().unwrap_or_default();
    // A cell erased with a background (SGR bg + EL/ED/ECH) keeps the colour
    // in its content, not in a style.
    let bg = match cell.content_tag() {
        Ok(CellContentTag::BgColorPalette) => cell
            .bg_color_palette()
            .map(|i| ColorSnap::Indexed(i.0))
            .unwrap_or_default(),
        Ok(CellContentTag::BgColorRgb) => cell
            .bg_color_rgb()
            .map(|c| ColorSnap::Rgb(c.r, c.g, c.b))
            .unwrap_or_default(),
        _ => color(style.bg_color),
    };
    CellSnap {
        text,
        fg: color(style.fg_color),
        bg,
        bold: style.bold,
        dim: style.faint,
        italic: style.italic,
        underline: style.underline != Underline::None,
        inverse: style.inverse,
        strikethrough: style.strikethrough,
        wide,
    }
}

/// Read the grid `scroll_offset` rows back into history (0 = live viewport).
/// The offset is clamped to the history available; rows past the end of the
/// buffer are empty.
pub fn snapshot(term: &Terminal, scroll_offset: usize) -> CellGrid {
    let rows_n = term.rows().unwrap_or(0);
    let cols = term.cols().unwrap_or(0);
    let base_y = term.scrollback_rows().unwrap_or(0);
    let len = term.total_rows().unwrap_or(rows_n as usize);
    let start = base_y.saturating_sub(scroll_offset);
    let live = start == base_y;

    let mut rows = Vec::with_capacity(rows_n as usize);
    let mut wrapped = Vec::with_capacity(rows_n as usize);
    for r in 0..rows_n as usize {
        let line_idx = start + r;
        if line_idx >= len {
            rows.push(vec![CellSnap::default(); cols as usize]);
            wrapped.push(false);
            continue;
        }
        let point = |x: u16| {
            if live {
                Point::Active(PointCoordinate { x, y: r as u32 })
            } else {
                Point::Screen(PointCoordinate {
                    x,
                    y: line_idx as u32,
                })
            }
        };
        let mut row = Vec::with_capacity(cols as usize);
        for x in 0..cols {
            row.push(read_cell(term, point(x)));
        }
        let is_wrapped = term
            .grid_ref(point(0))
            .ok()
            .and_then(|g| g.row().ok())
            .and_then(|r| r.is_wrap_continuation().ok())
            .unwrap_or(false);
        rows.push(row);
        wrapped.push(is_wrapped);
    }

    let mut cx = term.cursor_x().unwrap_or(0);
    if term.is_cursor_pending_wrap().unwrap_or(false) {
        cx = cx.saturating_add(1);
    }
    CellGrid {
        rows,
        wrapped,
        cursor: (
            term.cursor_y().unwrap_or(0),
            cx,
            term.is_cursor_visible().unwrap_or(true),
        ),
        base_y,
        len,
        cols,
        rows_n,
        start,
    }
}
