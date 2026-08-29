//! [`TerminalHandle`]: a `Send + Sync` handle over a [`TerminalActor`] that
//! runs on its own thread. Either spawns a child in a PTY
//! ([`TerminalHandle::spawn`], Node's `createPty`) or attaches to a session
//! daemon over its unix socket ([`TerminalHandle::attach`], Node's
//! `attachPty`; `src/tui/builders.ts:432-779`).
//!
//! Every byte source is tagged with the [`AttemptId`] it belongs to. A
//! reconnect bumps the attempt, so frames still in flight from the previous
//! socket (or a reader thread that has not noticed the close yet) are dropped
//! before they can touch the terminal. Readiness is explicit: an attach is
//! ready once the daemon's first SCREEN for the current attempt has been
//! parsed ([`TerminalHandle::wait_ready`]), not after a fixed delay.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use pty_core::protocol::{
    MessageType, Packet, PacketReader, decode_exit, decode_size, encode_attach, encode_data,
    encode_detach, encode_resize,
};

use crate::actor::{Modes, Notification, Range, TerminalActor, TerminalEvent};
use crate::serialize::SerializeOpts;
use crate::snapshot::CellGrid;

/// GEOMETRY (server → client, 4 bytes `rows u16BE, cols u16BE`), not yet a
/// named `MessageType` in `pty-core`.
const GEOMETRY_TYPE: u8 = 10;

/// Identifies one connection attempt (or the spawned child). Bumped by
/// [`TerminalHandle::reconnect`]; frames tagged with an older id are ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttemptId(pub u64);

/// A session daemon to attach to: `<root>/<id>.sock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef {
    /// The registry root (`$PTY_ROOT`).
    pub root: PathBuf,
    /// The session id.
    pub id: String,
}

impl SessionRef {
    /// `<root>/<id>.sock`.
    pub fn socket_path(&self) -> PathBuf {
        self.root.join(format!("{}.sock", self.id))
    }
}

/// Options for [`TerminalHandle::spawn`]. Defaults follow Node's `createPty`:
/// 80 x 24, no scrollback.
#[derive(Debug, Clone)]
pub struct SpawnOptions {
    /// Terminal height.
    pub rows: u16,
    /// Terminal width.
    pub cols: u16,
    /// Working directory of the child.
    pub cwd: Option<PathBuf>,
    /// Extra environment, merged over the inherited one.
    pub env: Vec<(String, String)>,
    /// Scrollback lines.
    pub scrollback: usize,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        SpawnOptions {
            rows: 24,
            cols: 80,
            cwd: None,
            env: Vec::new(),
            scrollback: 0,
        }
    }
}

/// Options for [`TerminalHandle::attach`].
#[derive(Debug, Clone)]
pub struct AttachOptions {
    /// Requested height (sent in ATTACH).
    pub rows: u16,
    /// Requested width (sent in ATTACH).
    pub cols: u16,
    /// Watch without input or resize: a geometry-neutral ATTACH. The daemon
    /// counts it as read-only and it never sends DATA or RESIZE.
    pub readonly: bool,
    /// Scrollback lines kept locally.
    pub scrollback: usize,
}

impl Default for AttachOptions {
    fn default() -> Self {
        AttachOptions {
            rows: 24,
            cols: 80,
            readonly: false,
            scrollback: 0,
        }
    }
}

/// What a subscriber hears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandleEvent {
    /// The screen changed; the new revision.
    Dirty(u64),
    /// The title changed (deduplicated).
    Title(String),
    /// BEL.
    Bell,
    /// The effective size changed (GEOMETRY from the daemon, or a local
    /// resize): `(rows, cols)`.
    Geometry(u16, u16),
    /// The child (or session) exited with this code.
    Exited(i32),
    /// OSC 9 / 99 / 777.
    Notification(Notification),
}

enum Msg {
    Output { attempt: AttemptId, bytes: Vec<u8> },
    Frame { attempt: AttemptId, packet: Packet },
    Disconnected { attempt: AttemptId },
    ChildExited { attempt: AttemptId, code: i32 },
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Snapshot { offset: usize, reply: Sender<(u64, CellGrid)> },
    Plain { range: Range, reply: Sender<String> },
    Serialize { opts: SerializeOpts, reply: Sender<String> },
    SetPalette(Vec<(u8, u8, u8)>),
    Reconnect { reply: Sender<io::Result<()>> },
    Close,
}

#[derive(Default)]
struct State {
    rev: u64,
    ready: bool,
    connected: bool,
    closed: bool,
    exit_code: Option<i32>,
    attempt: u64,
    cols: u16,
    rows: u16,
    modes: Modes,
    cursor: (u16, u16, bool),
    title: String,
    base_y: usize,
    len: usize,
    scrollback: usize,
    snap_cache: Option<(u64, CellGrid)>,
}

struct Shared {
    state: Mutex<State>,
    cv: Condvar,
    subs: Mutex<Vec<Sender<HandleEvent>>>,
}

impl Shared {
    fn emit(&self, ev: HandleEvent) {
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|s| s.send(ev.clone()).is_ok());
    }
}

enum Backend {
    Spawn {
        master: Box<dyn MasterPty + Send>,
        writer: Box<dyn Write + Send>,
    },
    Attach {
        session: SessionRef,
        opts: AttachOptions,
        stream: Option<UnixStream>,
        tx: Sender<Msg>,
    },
}

struct Core {
    actor: TerminalActor,
    attempt: AttemptId,
    shared: Arc<Shared>,
    backend: Backend,
}

impl Core {
    fn rev(&self) -> u64 {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rev
    }

    /// Publish the actor's state to the handle side, bump the revision, and
    /// fan out events.
    fn publish(&mut self) {
        let events = self.actor.take_events();
        let rev = {
            let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            st.rev += 1;
            st.snap_cache = None;
            st.cols = self.actor.cols();
            st.rows = self.actor.rows();
            st.modes = self.actor.modes();
            st.cursor = self.actor.cursor();
            st.title = self.actor.title();
            st.base_y = self.actor.base_y();
            st.len = self.actor.buffer_length();
            st.rev
        };
        self.shared.cv.notify_all();
        for ev in events {
            let ev = match ev {
                TerminalEvent::Bell => HandleEvent::Bell,
                TerminalEvent::TitleChange(t) => HandleEvent::Title(t),
                TerminalEvent::Notification(n) => HandleEvent::Notification(n),
                TerminalEvent::FocusRequest | TerminalEvent::CursorVisible => continue,
            };
            self.shared.emit(ev);
        }
        self.shared.emit(HandleEvent::Dirty(rev));
    }

    fn on_output(&mut self, bytes: Vec<u8>) {
        self.actor.write(&bytes);
        let replies = self.actor.take_pty_replies();
        if !replies.is_empty()
            && let Backend::Spawn { writer, .. } = &mut self.backend
        {
            let _ = writer.write_all(&replies);
            let _ = writer.flush();
        }
        self.publish();
    }

    fn on_frame(&mut self, packet: Packet) {
        match packet.type_ {
            MessageType::Screen => {
                self.actor.reset();
                self.actor.write(&packet.payload);
                // The daemon's own terminal answers queries; a second answer
                // from here would reach the child twice.
                let _ = self.actor.take_pty_replies();
                {
                    let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
                    st.ready = true;
                }
                self.publish();
            }
            MessageType::Data => {
                self.actor.write(&packet.payload);
                let _ = self.actor.take_pty_replies();
                self.publish();
            }
            MessageType::Exit => {
                let code = decode_exit(&packet.payload);
                self.on_exit(code);
            }
            MessageType::Unknown(GEOMETRY_TYPE) => {
                let (rows, cols) = decode_size(&packet.payload);
                self.actor.resize(cols, rows);
                self.publish();
                self.shared.emit(HandleEvent::Geometry(rows, cols));
            }
            _ => {}
        }
    }

    fn on_exit(&mut self, code: i32) {
        {
            let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            st.exit_code = Some(code);
            st.ready = true;
        }
        self.publish();
        self.shared.emit(HandleEvent::Exited(code));
    }

    fn on_disconnected(&mut self) {
        {
            let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            st.connected = false;
        }
        if let Backend::Attach { stream, .. } = &mut self.backend {
            *stream = None;
        }
        self.shared.cv.notify_all();
    }

    fn input(&mut self, data: &[u8]) {
        match &mut self.backend {
            Backend::Spawn { writer, .. } => {
                let _ = writer.write_all(data);
                let _ = writer.flush();
            }
            Backend::Attach { stream, opts, .. } => {
                if !opts.readonly
                    && let Some(s) = stream
                {
                    let _ = s.write_all(&encode_data(data));
                    let _ = s.flush();
                }
            }
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        if (cols, rows) == (self.actor.cols(), self.actor.rows()) {
            return;
        }
        match &mut self.backend {
            Backend::Spawn { master, .. } => {
                let _ = master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
            Backend::Attach { stream, opts, .. } => {
                if opts.readonly {
                    return;
                }
                if let Some(s) = stream {
                    let _ = s.write_all(&encode_resize(rows, cols));
                    let _ = s.flush();
                }
                // Applied locally as well: a daemon that speaks GEOMETRY will
                // confirm (or correct) the effective size; one that does not
                // has resized the PTY to what we asked for.
            }
        }
        self.actor.resize(cols, rows);
        self.publish();
        self.shared.emit(HandleEvent::Geometry(rows, cols));
    }

    fn reconnect(&mut self) -> io::Result<()> {
        let Backend::Attach {
            session,
            opts,
            stream,
            tx,
        } = &mut self.backend
        else {
            return Err(io::Error::other("reconnect is only for attached handles"));
        };
        if let Some(old) = stream.take() {
            let _ = (&old).write_all(&encode_detach());
            let _ = old.shutdown(std::net::Shutdown::Both);
        }
        self.attempt = AttemptId(self.attempt.0 + 1);
        {
            let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            st.ready = false;
            st.connected = false;
            st.exit_code = None;
            st.attempt = self.attempt.0;
        }
        let new_stream = connect_and_attach(session, opts, self.attempt, tx.clone())?;
        *stream = Some(new_stream);
        {
            let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            st.connected = true;
        }
        self.shared.cv.notify_all();
        Ok(())
    }

    fn dispatch(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Output { attempt, bytes } => {
                if attempt == self.attempt {
                    self.on_output(bytes);
                }
            }
            Msg::Frame { attempt, packet } => {
                if attempt == self.attempt {
                    self.on_frame(packet);
                }
            }
            Msg::Disconnected { attempt } => {
                if attempt == self.attempt {
                    self.on_disconnected();
                }
            }
            Msg::ChildExited { attempt, code } => {
                if attempt == self.attempt {
                    self.on_exit(code);
                }
            }
            Msg::Input(b) => self.input(&b),
            Msg::Resize { cols, rows } => self.resize(cols, rows),
            Msg::Snapshot { offset, reply } => {
                let _ = reply.send((self.rev(), self.actor.snapshot(offset)));
            }
            Msg::Plain { range, reply } => {
                let _ = reply.send(self.actor.plain(range));
            }
            Msg::Serialize { opts, reply } => {
                let _ = reply.send(self.actor.serialize(opts));
            }
            Msg::SetPalette(colors) => {
                self.actor.set_palette(&colors);
                self.publish();
            }
            Msg::Reconnect { reply } => {
                let r = self.reconnect();
                let _ = reply.send(r);
            }
            Msg::Close => return false,
        }
        true
    }

    fn shutdown(&mut self) {
        if let Backend::Attach { stream, .. } = &mut self.backend
            && let Some(s) = stream.take()
        {
            let _ = (&s).write_all(&encode_detach());
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        st.closed = true;
        st.connected = false;
        drop(st);
        self.shared.cv.notify_all();
    }
}

fn run(mut core: Core, rx: Receiver<Msg>) {
    core.publish();
    while let Ok(msg) = rx.recv() {
        if !core.dispatch(msg) {
            break;
        }
    }
    core.shutdown();
}

/// Connect to the daemon, send ATTACH, and start a reader thread that tags
/// every packet with `attempt`.
fn connect_and_attach(
    session: &SessionRef,
    opts: &AttachOptions,
    attempt: AttemptId,
    tx: Sender<Msg>,
) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(session.socket_path())?;
    (&stream).write_all(&encode_attach(opts.rows, opts.cols, opts.readonly))?;
    (&stream).flush()?;
    let reader = stream.try_clone()?;
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut parser = PacketReader::new();
        let mut buf = [0u8; 16384];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let packets = match parser.feed(&buf[..n]) {
                        Ok(p) => p,
                        Err(_) => break,
                    };
                    for packet in packets {
                        if tx.send(Msg::Frame { attempt, packet }).is_err() {
                            return;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(Msg::Disconnected { attempt });
    });
    Ok(stream)
}

fn exit_code(status: portable_pty::ExitStatus) -> i32 {
    if status.success() {
        0
    } else {
        status.exit_code() as i32
    }
}

/// A live terminal you can write to, resize, and read typed cells from.
/// Cheap to share (`Send + Sync`); every method is non-blocking except the
/// explicit waits and the reads that must ask the actor thread.
pub struct TerminalHandle {
    tx: Sender<Msg>,
    shared: Arc<Shared>,
    spawned_pid: Option<u32>,
}

impl TerminalHandle {
    /// Spawn `cmd args` in a new PTY (`TERM=xterm-256color` unless `env`
    /// says otherwise) and track it.
    pub fn spawn(cmd: &str, args: &[&str], opts: SpawnOptions) -> io::Result<TerminalHandle> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: opts.rows,
                cols: opts.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io::Error::other)?;
        let mut command = CommandBuilder::new(cmd);
        command.args(args);
        if let Some(cwd) = &opts.cwd {
            command.cwd(cwd);
        }
        if !opts.env.iter().any(|(k, _)| k == "TERM") {
            command.env("TERM", "xterm-256color");
        }
        for (k, v) in &opts.env {
            command.env(k, v);
        }
        let mut child = pair.slave.spawn_command(command).map_err(io::Error::other)?;
        drop(pair.slave);
        let pid = child.process_id();
        let reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let writer = pair.master.take_writer().map_err(io::Error::other)?;
        let master = pair.master;

        let (tx, rx) = mpsc::channel::<Msg>();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                ready: true,
                connected: true,
                attempt: 1,
                cols: opts.cols,
                rows: opts.rows,
                len: opts.rows as usize,
                cursor: (0, 0, true),
                scrollback: opts.scrollback,
                ..State::default()
            }),
            cv: Condvar::new(),
            subs: Mutex::new(Vec::new()),
        });
        let attempt = AttemptId(1);

        // PTY reader: bytes → actor; on EOF reap the child and report its code.
        {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut reader = reader;
                let mut buf = [0u8; 16384];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx
                                .send(Msg::Output {
                                    attempt,
                                    bytes: buf[..n].to_vec(),
                                })
                                .is_err()
                            {
                                let _ = child.kill();
                                let _ = child.wait();
                                return;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let code = child.wait().map(exit_code).unwrap_or(-1);
                let _ = tx.send(Msg::ChildExited { attempt, code });
            });
        }

        let (rows, cols, scrollback) = (opts.rows, opts.cols, opts.scrollback);
        let core_shared = shared.clone();
        std::thread::spawn(move || {
            let core = Core {
                actor: TerminalActor::new(rows, cols, scrollback),
                attempt,
                shared: core_shared,
                backend: Backend::Spawn { master, writer },
            };
            run(core, rx);
        });

        let handle = TerminalHandle {
            tx,
            shared,
            spawned_pid: pid,
        };
        handle.wait_first_publish();
        Ok(handle)
    }

    /// Attach to a running session daemon. Returns once the socket is
    /// connected and ATTACH was sent; use [`TerminalHandle::wait_ready`] to
    /// wait for the first SCREEN.
    pub fn attach(session: SessionRef, opts: AttachOptions) -> io::Result<TerminalHandle> {
        let (tx, rx) = mpsc::channel::<Msg>();
        let attempt = AttemptId(1);
        let stream = connect_and_attach(&session, &opts, attempt, tx.clone())?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                ready: false,
                connected: true,
                attempt: 1,
                cols: opts.cols,
                rows: opts.rows,
                len: opts.rows as usize,
                cursor: (0, 0, true),
                scrollback: opts.scrollback,
                ..State::default()
            }),
            cv: Condvar::new(),
            subs: Mutex::new(Vec::new()),
        });
        let core_shared = shared.clone();
        let core_tx = tx.clone();
        std::thread::spawn(move || {
            let core = Core {
                actor: TerminalActor::new(opts.rows, opts.cols, opts.scrollback),
                attempt,
                shared: core_shared,
                backend: Backend::Attach {
                    session,
                    opts,
                    stream: Some(stream),
                    tx: core_tx,
                },
            };
            run(core, rx);
        });
        let handle = TerminalHandle {
            tx,
            shared,
            spawned_pid: None,
        };
        handle.wait_first_publish();
        Ok(handle)
    }

    /// The actor thread publishes once before it reads any input; reads
    /// made after construction see the terminal, not the placeholder.
    fn wait_first_publish(&self) {
        self.wait_state(Duration::from_secs(5), |st| st.rev >= 1 || st.closed);
    }

    fn state<T>(&self, f: impl FnOnce(&State) -> T) -> T {
        let st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        f(&st)
    }

    fn wait_state(&self, timeout: Duration, mut done: impl FnMut(&State) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if done(&st) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (guard, _) = self
                .shared
                .cv
                .wait_timeout(st, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            st = guard;
        }
    }

    /// Block until the handle is ready: a spawned child immediately; an
    /// attach once the first SCREEN of the current attempt has been parsed
    /// (or the session reported EXIT). Returns false on timeout or close.
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        self.wait_state(timeout, |st| st.ready) && !self.state(|st| st.closed)
    }

    /// Whether the first SCREEN of the current attempt has been parsed.
    pub fn is_ready(&self) -> bool {
        self.state(|st| st.ready)
    }

    /// Block until the revision passes `after` (i.e. something changed).
    pub fn wait_rev(&self, after: u64, timeout: Duration) -> bool {
        self.wait_state(timeout, |st| st.rev > after || st.closed)
    }

    /// Block until `pred` holds for a fresh viewport snapshot, polling on
    /// every revision. Returns the matching grid or `None` on timeout.
    pub fn wait_for(
        &self,
        timeout: Duration,
        mut pred: impl FnMut(&CellGrid) -> bool,
    ) -> Option<CellGrid> {
        let deadline = Instant::now() + timeout;
        loop {
            let rev = self.rev();
            let grid = self.snapshot(0);
            if pred(&grid) {
                return Some(grid);
            }
            let now = Instant::now();
            if now >= deadline || self.state(|st| st.closed) {
                return None;
            }
            self.wait_rev(rev, deadline - now);
        }
    }

    /// Raw input to the child (spawn) or a DATA packet (attach). Ignored by a
    /// read-only attach.
    pub fn write(&self, data: &[u8]) {
        let _ = self.tx.send(Msg::Input(data.to_vec()));
    }

    /// Resize the PTY (spawn) or request a resize (attach). No-op when the
    /// size is unchanged or the handle is read-only.
    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.tx.send(Msg::Resize { cols, rows });
    }

    /// The cell grid `scroll_offset` rows back into history (0 = live). The
    /// live grid is cached per revision.
    pub fn snapshot(&self, scroll_offset: usize) -> CellGrid {
        if scroll_offset == 0 {
            let cached = self.state(|st| {
                st.snap_cache
                    .as_ref()
                    .filter(|(rev, _)| *rev == st.rev)
                    .map(|(_, g)| g.clone())
            });
            if let Some(g) = cached {
                return g;
            }
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        if self
            .tx
            .send(Msg::Snapshot {
                offset: scroll_offset,
                reply: reply_tx,
            })
            .is_err()
        {
            return CellGrid::default();
        }
        match reply_rx.recv() {
            Ok((rev, grid)) => {
                if scroll_offset == 0 {
                    let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
                    if st.rev == rev {
                        st.snap_cache = Some((rev, grid.clone()));
                    }
                }
                grid
            }
            Err(_) => CellGrid::default(),
        }
    }

    /// The plain-text screen (asks the actor).
    pub fn plain(&self, range: Range) -> String {
        let (reply_tx, reply_rx) = mpsc::channel();
        if self.tx.send(Msg::Plain { range, reply: reply_tx }).is_err() {
            return String::new();
        }
        reply_rx.recv().unwrap_or_default()
    }

    /// The replay serialization (asks the actor).
    pub fn serialize(&self, opts: SerializeOpts) -> String {
        let (reply_tx, reply_rx) = mpsc::channel();
        if self
            .tx
            .send(Msg::Serialize {
                opts,
                reply: reply_tx,
            })
            .is_err()
        {
            return String::new();
        }
        reply_rx.recv().unwrap_or_default()
    }

    /// The current revision; bumps on every change.
    pub fn rev(&self) -> u64 {
        self.state(|st| st.rev)
    }

    /// Receive events. Each subscriber gets its own channel.
    pub fn subscribe(&self) -> Receiver<HandleEvent> {
        let (tx, rx) = mpsc::channel();
        self.shared
            .subs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(tx);
        rx
    }

    /// The Node-tracked mode flags and kitty stack (a copy).
    pub fn modes(&self) -> Modes {
        self.state(|st| st.modes.clone())
    }

    /// `(row, col, visible)` of the cursor, viewport-relative.
    pub fn cursor(&self) -> (u16, u16, bool) {
        self.state(|st| (st.cursor.1, st.cursor.0, st.cursor.2))
    }

    /// Current width.
    pub fn cols(&self) -> u16 {
        self.state(|st| st.cols)
    }

    /// Current height.
    pub fn rows(&self) -> u16 {
        self.state(|st| st.rows)
    }

    /// The window title.
    pub fn title(&self) -> String {
        self.state(|st| st.title.clone())
    }

    /// Buffer row where the live viewport starts.
    pub fn base_y(&self) -> usize {
        self.state(|st| st.base_y)
    }

    /// History rows + viewport rows.
    pub fn buffer_length(&self) -> usize {
        self.state(|st| st.len)
    }

    /// Configured scrollback lines.
    pub fn scrollback(&self) -> usize {
        self.state(|st| st.scrollback)
    }

    /// Whether the child (or session) has exited.
    pub fn exited(&self) -> bool {
        self.state(|st| st.exit_code.is_some())
    }

    /// The exit code, once exited.
    pub fn exit_code(&self) -> Option<i32> {
        self.state(|st| st.exit_code)
    }

    /// Whether the socket (attach) is connected. Always true for a spawn
    /// until closed.
    pub fn connected(&self) -> bool {
        self.state(|st| st.connected)
    }

    /// The current attempt id.
    pub fn attempt(&self) -> AttemptId {
        AttemptId(self.state(|st| st.attempt))
    }

    /// Override the first `colors.len()` palette entries (a theme).
    pub fn set_palette(&self, colors: &[(u8, u8, u8)]) {
        let _ = self.tx.send(Msg::SetPalette(colors.to_vec()));
    }

    /// Reconnect an attached handle: a new attempt, a new socket, ATTACH
    /// again; frames from the old socket are dropped. Returns once the new
    /// socket is connected (then [`TerminalHandle::wait_ready`]).
    pub fn reconnect(&self) -> io::Result<()> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Msg::Reconnect { reply: reply_tx })
            .map_err(|_| io::Error::other("handle closed"))?;
        reply_rx
            .recv()
            .unwrap_or_else(|_| Err(io::Error::other("handle closed")))
    }

    /// Spawn: kill the child and reap it. Attach: DETACH and drop the socket
    /// (the daemon keeps running). The actor thread stops either way.
    pub fn kill(&self) {
        if let Some(pid) = self.spawned_pid
            && !self.exited()
        {
            // SAFETY: plain kill(2) on a pid we spawned.
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
        let _ = self.tx.send(Msg::Close);
        self.wait_state(Duration::from_secs(5), |st| st.closed);
    }

    /// Alias for [`TerminalHandle::kill`].
    pub fn close(&self) {
        self.kill();
    }
}

impl Drop for TerminalHandle {
    fn drop(&mut self) {
        if !self.state(|st| st.closed) {
            self.kill();
        }
    }
}

impl std::fmt::Debug for TerminalHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.state(|st| {
            f.debug_struct("TerminalHandle")
                .field("rev", &st.rev)
                .field("ready", &st.ready)
                .field("cols", &st.cols)
                .field("rows", &st.rows)
                .field("exit_code", &st.exit_code)
                .finish()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pty_core::protocol::encode_screen;

    fn detached_core() -> (Core, Sender<Msg>) {
        let (tx, _rx) = mpsc::channel::<Msg>();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                attempt: 1,
                ..State::default()
            }),
            cv: Condvar::new(),
            subs: Mutex::new(Vec::new()),
        });
        let core = Core {
            actor: TerminalActor::new(5, 20, 0),
            attempt: AttemptId(1),
            shared,
            backend: Backend::Attach {
                session: SessionRef {
                    root: PathBuf::from("/nonexistent"),
                    id: "x".into(),
                },
                opts: AttachOptions::default(),
                stream: None,
                tx: tx.clone(),
            },
        };
        (core, tx)
    }

    fn packet(bytes: Vec<u8>) -> Packet {
        let mut parser = PacketReader::new();
        parser.feed(&bytes).unwrap().remove(0)
    }

    /// Frames tagged with an older attempt never reach the terminal, and a
    /// stale EXIT never marks the replacement exited.
    #[test]
    fn frames_from_an_older_attempt_are_dropped() {
        let (mut core, _tx) = detached_core();
        core.dispatch(Msg::Frame {
            attempt: AttemptId(1),
            packet: packet(encode_screen(b"first")),
        });
        assert_eq!(core.actor.plain(Range::Viewport), "first");
        assert!(core.shared.state.lock().unwrap().ready);

        // A reconnect bumps the attempt (simulated: no socket here).
        core.attempt = AttemptId(2);
        core.shared.state.lock().unwrap().ready = false;

        core.dispatch(Msg::Frame {
            attempt: AttemptId(1),
            packet: packet(encode_data(b" stale")),
        });
        core.dispatch(Msg::Frame {
            attempt: AttemptId(1),
            packet: packet(pty_core::protocol::encode_exit(9)),
        });
        core.dispatch(Msg::Disconnected {
            attempt: AttemptId(1),
        });
        assert_eq!(core.actor.plain(Range::Viewport), "first");
        assert!(!core.shared.state.lock().unwrap().ready);
        assert_eq!(core.shared.state.lock().unwrap().exit_code, None);

        core.dispatch(Msg::Frame {
            attempt: AttemptId(2),
            packet: packet(encode_screen(b"second")),
        });
        assert_eq!(core.actor.plain(Range::Viewport), "second");
        assert!(core.shared.state.lock().unwrap().ready);
    }

    /// A daemon that speaks GEOMETRY resizes the local terminal; one that
    /// sends SCREEN first works too.
    #[test]
    fn geometry_and_screen_in_either_order() {
        let (mut core, _tx) = detached_core();
        core.dispatch(Msg::Frame {
            attempt: AttemptId(1),
            packet: packet(pty_core::protocol::encode_packet(
                MessageType::Unknown(GEOMETRY_TYPE),
                &[0, 3, 0, 10],
            )),
        });
        assert_eq!((core.actor.rows(), core.actor.cols()), (3, 10));
        core.dispatch(Msg::Frame {
            attempt: AttemptId(1),
            packet: packet(encode_screen(b"\x1b[?25l\x1b[>7uhi")),
        });
        assert!(core.shared.state.lock().unwrap().ready);
        assert_eq!(core.actor.plain(Range::Viewport), "hi");
        assert!(core.actor.modes().cursor_hidden);
        assert_eq!(core.actor.modes().kitty_stack, vec![7]);
    }
}
