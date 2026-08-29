//! The terminal actor: the one owner of a libghostty `Terminal`.
//!
//! [`TerminalActor`] is synchronous and lives on the thread that created it
//! (the `Terminal` is `!Send`). The daemon and the testkit already run their
//! loop on that thread; [`crate::handle::TerminalHandle`] wraps an actor in a
//! thread + channel for everyone else.
//!
//! Every call is ordered: when [`TerminalActor::write`] returns, every byte it
//! was given has been parsed, every query in it has been answered into
//! [`TerminalActor::take_pty_replies`], and every mode flag and event is
//! up to date. That is what makes a SCREEN cut an exact baseline.

use std::cell::RefCell;
use std::rc::Rc;

use libghostty_vt::style::RgbColor;
use libghostty_vt::terminal::{Options, Terminal};

use crate::queries;
use crate::screenshot::{self, Screenshot};
use crate::serialize::{self, SerializeOpts};
use crate::snapshot::{self, CellGrid};
use crate::strip::{OutputScanner, Osc, Token};

/// Node's scrollback (`src/server.ts:333-338`).
pub const DEFAULT_SCROLLBACK: usize = 10_000;

/// A desktop notification the child asked for (OSC 9, 99, or 777).
/// Shapes follow Node (`src/server.ts:421-454`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// OSC 99 `title=`/`t=` or OSC 777's second field; OSC 9 has none.
    pub title: Option<String>,
    /// OSC 9's whole payload, OSC 99 `body=`/`b=`, OSC 777's remaining fields.
    pub body: Option<String>,
    /// `"osc9"`, `"osc99"`, or `"osc777"`.
    pub source: &'static str,
}

/// Something the child did that a session consumer wants to hear about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// BEL.
    Bell,
    /// OSC 0/2 set a *different* title than the last one reported
    /// (`src/server.ts:413-418` deduplicates).
    TitleChange(String),
    /// OSC 9 / 99 / 777.
    Notification(Notification),
    /// `CSI ? 1004 h` — the child asked for focus events.
    FocusRequest,
    /// `CSI ? 25 h` while the cursor was hidden.
    CursorVisible,
}

/// The mode flags Node tracks itself, from the child's own escape sequences
/// (`src/server.ts:343-391`), plus the kitty keyboard stack. These feed the
/// mode prefix that precedes every SCREEN (see [`crate::serialize`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Modes {
    /// `?1006`.
    pub sgr_mouse: bool,
    /// `?1000`.
    pub mouse_1000: bool,
    /// `?1002`.
    pub mouse_1002: bool,
    /// `?1003`.
    pub mouse_1003: bool,
    /// `?1049` / `?1047` / `?47`.
    pub alt_screen: bool,
    /// `?25 l` seen and not yet undone by `?25 h`.
    pub cursor_hidden: bool,
    /// `?2004`.
    pub bracketed_paste: bool,
    /// `?1004`.
    pub focus_events: bool,
    /// `CSI > flags u` pushes, oldest first; `CSI < u` pops one.
    pub kitty_stack: Vec<u8>,
}

impl Modes {
    /// Any of the three tracking modes (Node's `mouseMode`).
    pub fn mouse_tracking(&self) -> bool {
        self.mouse_1000 || self.mouse_1002 || self.mouse_1003
    }

    fn apply_dec(&mut self, mode: u16, set: bool, events: &mut Vec<TerminalEvent>) {
        match mode {
            1006 => self.sgr_mouse = set,
            1000 => self.mouse_1000 = set,
            1002 => self.mouse_1002 = set,
            1003 => self.mouse_1003 = set,
            1049 | 1047 | 47 => self.alt_screen = set,
            25 => {
                if set {
                    if self.cursor_hidden {
                        events.push(TerminalEvent::CursorVisible);
                    }
                    self.cursor_hidden = false;
                } else {
                    self.cursor_hidden = true;
                }
            }
            1004 => {
                self.focus_events = set;
                if set {
                    events.push(TerminalEvent::FocusRequest);
                }
            }
            2004 => self.bracketed_paste = set,
            _ => {}
        }
    }
}

/// Which rows a plain-text read covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    /// The active area only (rows `base_y..len`) — Node's default peek.
    Viewport,
    /// Scrollback and active area — Node's `peek --full`.
    Full,
}

#[derive(Default)]
struct Shared {
    pty_replies: Vec<u8>,
    bells: u32,
    titles: Vec<String>,
}

/// The owner of a libghostty terminal. See the [module docs](self).
pub struct TerminalActor {
    term: Terminal<'static, 'static>,
    shared: Rc<RefCell<Shared>>,
    scanner: OutputScanner,
    modes: Modes,
    events: Vec<TerminalEvent>,
    last_title: Option<String>,
    scrollback: usize,
}

impl TerminalActor {
    /// A terminal of `rows` x `cols` with `scrollback` lines of history.
    pub fn new(rows: u16, cols: u16, scrollback: usize) -> TerminalActor {
        let shared: Rc<RefCell<Shared>> = Rc::new(RefCell::new(Shared::default()));
        let mut term = Terminal::new(Options {
            cols: cols.max(1),
            rows: rows.max(1),
            max_scrollback: scrollback,
        })
        .expect("libghostty terminal");
        {
            let s = shared.clone();
            term.on_pty_write(move |_t, data| s.borrow_mut().pty_replies.extend_from_slice(data))
                .expect("install on_pty_write");
        }
        {
            let s = shared.clone();
            term.on_bell(move |_t| s.borrow_mut().bells += 1)
                .expect("install on_bell");
        }
        {
            let s = shared.clone();
            term.on_title_changed(move |t| {
                let title = t.title().unwrap_or("").to_string();
                s.borrow_mut().titles.push(title);
            })
            .expect("install on_title_changed");
        }
        queries::install(&mut term);
        TerminalActor {
            term,
            shared,
            scanner: OutputScanner::new(),
            modes: Modes::default(),
            events: Vec::new(),
            last_title: None,
            scrollback,
        }
    }

    /// Node's defaults: 24 x 80, 10 000 lines of scrollback.
    pub fn with_defaults() -> TerminalActor {
        TerminalActor::new(24, 80, DEFAULT_SCROLLBACK)
    }

    /// The underlying terminal, for reads this API does not cover.
    pub fn terminal(&self) -> &Terminal<'static, 'static> {
        &self.term
    }

    /// Feed the child's output. Returns the bytes to broadcast to attached
    /// clients: the input minus terminal queries (which are answered into
    /// [`TerminalActor::take_pty_replies`] instead). Mode flags, the kitty
    /// stack, and events are updated as a side effect.
    pub fn write(&mut self, data: &[u8]) -> Vec<u8> {
        let tokens = self.scanner.feed(data);
        let mut feed: Vec<u8> = Vec::with_capacity(data.len());
        let mut broadcast: Vec<u8> = Vec::with_capacity(data.len());
        for tok in tokens {
            match tok {
                Token::Raw(b) => {
                    feed.extend_from_slice(&b);
                    broadcast.extend_from_slice(&b);
                }
                Token::Csi(c) => {
                    if let Some(flags) = c.kitty_push() {
                        self.modes.kitty_stack.push(flags);
                    } else if c.is_kitty_pop() {
                        self.modes.kitty_stack.pop();
                    } else if let Some((params, set)) = c.dec_modes() {
                        for &p in params {
                            self.modes.apply_dec(p, set, &mut self.events);
                        }
                    }
                    feed.extend_from_slice(&c.raw);
                    if !c.is_stripped_query() {
                        broadcast.extend_from_slice(&c.raw);
                    }
                }
                Token::Osc(o) => {
                    if let Some((id, index)) = o.color_query() {
                        // Answer in stream order: everything before the query
                        // reaches the terminal (and may itself be answered)
                        // before this reply is queued.
                        self.flush_feed(&mut feed);
                        if let Some(reply) = queries::color_query_reply(id, index) {
                            self.shared.borrow_mut().pty_replies.extend_from_slice(&reply);
                        }
                        continue;
                    }
                    self.tap_notification(&o);
                    feed.extend_from_slice(&o.raw);
                    broadcast.extend_from_slice(&o.raw);
                }
            }
        }
        self.flush_feed(&mut feed);
        self.collect_callback_events();
        broadcast
    }

    fn flush_feed(&mut self, feed: &mut Vec<u8>) {
        if !feed.is_empty() {
            self.term.vt_write(feed);
            feed.clear();
        }
    }

    fn collect_callback_events(&mut self) {
        let (bells, titles) = {
            let mut s = self.shared.borrow_mut();
            (std::mem::take(&mut s.bells), std::mem::take(&mut s.titles))
        };
        for _ in 0..bells {
            self.events.push(TerminalEvent::Bell);
        }
        for title in titles {
            if self.last_title.as_deref() != Some(title.as_str()) {
                self.last_title = Some(title.clone());
                self.events.push(TerminalEvent::TitleChange(title));
            }
        }
    }

    /// OSC 9 / 99 / 777 → [`TerminalEvent::Notification`]
    /// (`src/server.ts:421-454`, field by field).
    fn tap_notification(&mut self, osc: &Osc) {
        let (id, data) = osc.split();
        let data = String::from_utf8_lossy(data).into_owned();
        let n = match id {
            Some(9) => Notification {
                title: None,
                body: Some(data),
                source: "osc9",
            },
            Some(99) => {
                let mut fields: Vec<(String, String)> = Vec::new();
                for part in data.split(';') {
                    if let Some(eq) = part.find('=') {
                        fields.push((part[..eq].to_string(), part[eq + 1..].to_string()));
                    }
                }
                let get = |k: &str| fields.iter().rev().find(|(fk, _)| fk == k).map(|(_, v)| v.clone());
                Notification {
                    title: get("title").or_else(|| get("t")),
                    body: get("body").or_else(|| get("b")),
                    source: "osc99",
                }
            }
            Some(777) => {
                let parts: Vec<&str> = data.split(';').collect();
                if parts.first() == Some(&"notify") && parts.len() >= 2 {
                    Notification {
                        title: Some(parts[1].to_string()),
                        body: Some(parts[2..].join(";")),
                        source: "osc777",
                    }
                } else {
                    return;
                }
            }
            _ => return,
        };
        self.events.push(TerminalEvent::Notification(n));
    }

    /// Resize the terminal (the primary screen reflows).
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let _ = self.term.resize(cols.max(1), rows.max(1), 0, 0);
    }

    /// Full reset (RIS): screen, scrollback, modes, title. The tracked mode
    /// flags and any partial sequence in the scanner are cleared too. Used
    /// before replaying a SCREEN.
    pub fn reset(&mut self) {
        self.term.reset();
        self.scanner.reset();
        self.modes = Modes::default();
        self.shared.borrow_mut().titles.clear();
        self.shared.borrow_mut().bells = 0;
    }

    /// The plain-text screen: rows right-trimmed of never-written cells
    /// (written spaces kept, like xterm's `translateToString(true)`), trailing
    /// empty rows dropped, joined by `\n`.
    pub fn plain(&self, range: Range) -> String {
        match range {
            Range::Viewport => serialize::plain_viewport(&self.term),
            Range::Full => serialize::plain_full(&self.term),
        }
    }

    /// The replay payload: Node's mode prefix followed by the VT
    /// serialization of the screen (cursor, modes, kitty keyboard).
    pub fn serialize(&self, opts: SerializeOpts) -> String {
        serialize::serialize_for_replay(self, opts)
    }

    /// The typed cell grid, `scroll_offset` rows back into history (0 = the
    /// live viewport).
    pub fn snapshot(&self, scroll_offset: usize) -> CellGrid {
        snapshot::snapshot(&self.term, scroll_offset)
    }

    /// The testkit's screenshot (all rows, VT with modes).
    pub fn screenshot(&self) -> Screenshot {
        screenshot::capture(&self.term)
    }

    /// The Node-tracked mode flags and kitty stack.
    pub fn modes(&self) -> Modes {
        self.modes.clone()
    }

    /// Bytes the terminal wants written back to the child (query answers).
    pub fn take_pty_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.shared.borrow_mut().pty_replies)
    }

    /// Events since the last call, in order.
    pub fn take_events(&mut self) -> Vec<TerminalEvent> {
        std::mem::take(&mut self.events)
    }

    /// `(x, y, visible)`: the cursor in the active area, 0-based. Like xterm,
    /// `x` equals `cols` when the cursor is pending a wrap after printing
    /// in the last column.
    pub fn cursor(&self) -> (u16, u16, bool) {
        let mut x = self.term.cursor_x().unwrap_or(0);
        if self.term.is_cursor_pending_wrap().unwrap_or(false) {
            x = x.saturating_add(1);
        }
        (
            x,
            self.term.cursor_y().unwrap_or(0),
            self.term.is_cursor_visible().unwrap_or(true),
        )
    }

    /// The window title (OSC 0/2), `""` when none was set.
    pub fn title(&self) -> String {
        self.term.title().unwrap_or("").to_string()
    }

    /// Node's `scrollbackUsed`: rows in the buffer, history and viewport
    /// (`src/server.ts:1128`, `buffer.active.length`).
    pub fn scrollback_used(&self) -> usize {
        self.buffer_length()
    }

    /// Node's `scrollbackCapacity`: `rows + scrollback`.
    pub fn scrollback_capacity(&self) -> usize {
        self.rows() as usize + self.scrollback
    }

    /// Configured scrollback lines.
    pub fn scrollback(&self) -> usize {
        self.scrollback
    }

    /// Node's `baseY`: the buffer row where the active area starts.
    pub fn base_y(&self) -> usize {
        self.term.scrollback_rows().unwrap_or(0)
    }

    /// Node's `bufferLength`: history rows + viewport rows.
    pub fn buffer_length(&self) -> usize {
        self.term.total_rows().unwrap_or(self.rows() as usize)
    }

    /// Terminal height.
    pub fn rows(&self) -> u16 {
        self.term.rows().unwrap_or(0)
    }

    /// Terminal width.
    pub fn cols(&self) -> u16 {
        self.term.cols().unwrap_or(0)
    }

    /// Whether libghostty's active screen is the alternate one (its own
    /// view of `?1049`/`?1047`/`?47`; [`Modes::alt_screen`] is Node's).
    pub fn alt_screen_active(&self) -> bool {
        matches!(
            self.term.active_screen(),
            Ok(libghostty_vt::screen::Screen::Alternate)
        )
    }

    /// The kitty keyboard flags currently in effect (libghostty's value; the
    /// push/pop history is [`Modes::kitty_stack`]).
    pub fn kitty_flags(&self) -> u8 {
        self.term.kitty_keyboard_flags().map(|f| f.bits()).unwrap_or(0)
    }

    /// Override palette entries `0..colors.len()` (a theme). Cells keep
    /// their palette index; only what an RGB read resolves to changes.
    pub fn set_palette(&mut self, colors: &[(u8, u8, u8)]) {
        let mut palette = match self.term.default_color_palette() {
            Ok(p) => p,
            Err(_) => return,
        };
        for (i, &(r, g, b)) in colors.iter().take(256).enumerate() {
            palette.0[i] = RgbColor { r, g, b };
        }
        let _ = self.term.set_default_color_palette(Some(palette));
    }
}

impl std::fmt::Debug for TerminalActor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalActor")
            .field("rows", &self.rows())
            .field("cols", &self.cols())
            .field("modes", &self.modes)
            .finish()
    }
}
