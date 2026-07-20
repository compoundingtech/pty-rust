//! The per-session daemon: owns the PTY + a libghostty terminal, and serves the
//! wire protocol over a unix socket so `pty` clients can attach, peek, send
//! input, resize, and observe exit.
//!
//! Concurrency model: libghostty's `Terminal` is `!Send`, so it lives on a
//! single "actor" thread (this function's thread). A PTY reader thread and one
//! reader thread per client only *message* the actor over a channel; the actor
//! owns all terminal state and the PTY writer.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use libghostty_vt::terminal::{Options, Terminal};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use crate::protocol::{
    decode_peek, decode_size, encode_data, encode_exit, encode_screen, encode_status_response,
    MessageType, PacketReader,
};
use crate::registry::{self, SessionMetadata};
use crate::screenshot::capture;

/// Parameters for launching a session daemon.
pub struct DaemonConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub display_command: String,
    pub cwd: String,
    pub rows: u16,
    pub cols: u16,
    pub env: Vec<(String, String)>,
}

enum DaemonMsg {
    PtyData(Vec<u8>),
    PtyExited(i32),
    ClientConnect { id: u64, tx: Sender<Vec<u8>> },
    ClientAttach { id: u64, rows: u16, cols: u16, geometry_neutral: bool },
    ClientPeek { id: u64, plain: bool },
    ClientData(Vec<u8>),
    ClientResize { rows: u16, cols: u16 },
    ClientStatus { id: u64 },
    ClientDetach { id: u64 },
}

struct Client {
    tx: Sender<Vec<u8>>,
    streaming: bool,
}

/// The child (session) pid, for the SIGTERM handler to forward to. Node records
/// the DAEMON pid in `<name>.pid`, so `kill` targets the daemon; the daemon then
/// forwards the signal to the child, which triggers a clean exit + metadata.
static CHILD_PID: AtomicI32 = AtomicI32::new(0);

extern "C" fn forward_sigterm(_sig: i32) {
    let pid = CHILD_PID.load(Ordering::SeqCst);
    if pid > 0 {
        // SAFETY: kill() is async-signal-safe.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
}

/// Run the session daemon to completion. Blocks until the child process exits
/// or the daemon is torn down. Returns the child's exit code.
pub fn run(cfg: DaemonConfig) -> std::io::Result<i32> {
    registry::ensure_session_dir()?;

    // ── PTY + child ──
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: cfg.rows,
            cols: cfg.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(std::io::Error::other)?;

    let mut cmd = CommandBuilder::new(&cfg.command);
    cmd.args(&cfg.args);
    cmd.cwd(&cfg.cwd);
    // Inherit env, then apply extras and mark the session for nesting detection.
    for (k, v) in &cfg.env {
        cmd.env(k, v);
    }
    cmd.env("PTY_SESSION", &cfg.name);
    cmd.env("TERM", "xterm-256color");

    let mut child = pair.slave.spawn_command(cmd).map_err(std::io::Error::other)?;
    drop(pair.slave);
    // Forward SIGTERM (sent to the daemon by `pty kill`) to the child, so the
    // child's exit runs our clean-shutdown path (exit metadata + final screen).
    let child_pid = child.process_id().unwrap_or(0) as i32;
    CHILD_PID.store(child_pid, Ordering::SeqCst);
    unsafe {
        let handler = forward_sigterm as extern "C" fn(i32);
        libc::signal(libc::SIGTERM, handler as libc::sighandler_t);
    }

    let reader = pair.master.try_clone_reader().map_err(std::io::Error::other)?;
    let writer = pair.master.take_writer().map_err(std::io::Error::other)?;
    let master = pair.master;

    // ── metadata + pid ──
    let meta = SessionMetadata {
        command: cfg.command.clone(),
        args: cfg.args.clone(),
        display_command: cfg.display_command.clone(),
        cwd: cfg.cwd.clone(),
        created_at: now_iso8601(),
        exit_code: None,
        exited_at: None,
        last_lines: None,
        tags: None,
        display_name: None,
        last_attach_at: None,
    };
    registry::write_metadata(&cfg.name, &meta)?;
    // Record the DAEMON pid (matching node: `<name>.pid` holds the server
    // process's own pid, and `ls --json` exposes it). `kill` SIGTERMs this; the
    // handler above forwards to the child.
    std::fs::write(registry::pid_path(&cfg.name), std::process::id().to_string())?;

    // ── libghostty terminal ──
    let pending: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let mut terminal = Terminal::new(Options {
        cols: cfg.cols,
        rows: cfg.rows,
        max_scrollback: 10_000,
    })
    .expect("libghostty terminal");
    {
        let pending = pending.clone();
        terminal
            .on_pty_write(move |_t, data| pending.borrow_mut().extend_from_slice(data))
            .expect("on_pty_write");
    }

    // ── channels + threads ──
    let (tx, rx): (Sender<DaemonMsg>, Receiver<DaemonMsg>) = mpsc::channel();

    // PTY reader thread → PtyData / PtyExited.
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(DaemonMsg::PtyData(buf[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let code = child.wait().ok().and_then(exit_code).unwrap_or(-1);
            let _ = tx.send(DaemonMsg::PtyExited(code));
        });
    }

    // Socket acceptor thread → spawns a reader thread per client.
    let socket = registry::socket_path(&cfg.name);
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    {
        let tx = tx.clone();
        let ids = Arc::new(AtomicU64::new(1));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let stream = match stream {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let id = ids.fetch_add(1, Ordering::Relaxed);
                spawn_client(id, stream, tx.clone());
            }
        });
    }

    // ── actor loop ──
    let mut clients: HashMap<u64, Client> = HashMap::new();
    let mut writer = writer;
    let mut cur_rows = cfg.rows;
    let mut cur_cols = cfg.cols;

    let flush_pending = |writer: &mut Box<dyn Write + Send>, pending: &Rc<RefCell<Vec<u8>>>| {
        let out = std::mem::take(&mut *pending.borrow_mut());
        if !out.is_empty() {
            let _ = writer.write_all(&out);
            let _ = writer.flush();
        }
    };

    let exit_code_final;
    loop {
        let msg = match rx.recv() {
            Ok(m) => m,
            Err(_) => {
                exit_code_final = -1;
                break;
            }
        };
        match msg {
            DaemonMsg::PtyData(bytes) => {
                terminal.vt_write(&bytes);
                flush_pending(&mut writer, &pending);
                // Broadcast to streaming clients; drop dead ones.
                let packet = encode_data(&bytes);
                clients.retain(|_, c| {
                    if c.streaming {
                        c.tx.send(packet.clone()).is_ok()
                    } else {
                        true
                    }
                });
            }
            DaemonMsg::ClientConnect { id, tx } => {
                clients.insert(id, Client { tx, streaming: false });
            }
            DaemonMsg::ClientAttach { id, rows, cols, geometry_neutral } => {
                if let Some(c) = clients.get_mut(&id) {
                    c.streaming = true;
                    // Replay the current screen WITH terminal mode/cursor state
                    // so a reattaching client restores a TUI's full state
                    // (mouse tracking, alt-screen, cursor visibility, kitty kbd),
                    // not just its glyphs.
                    let screen = crate::screenshot::serialize_for_replay(&terminal);
                    let _ = c.tx.send(encode_screen(screen.as_bytes()));
                }
                // A non-neutral attach negotiates the shared PTY geometry.
                if !geometry_neutral && (rows, cols) != (cur_rows, cur_cols) {
                    cur_rows = rows;
                    cur_cols = cols;
                    let _ = master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
                    let _ = terminal.resize(cols, rows, 0, 0);
                }
                // Record lastAttachAt.
                if let Some(mut m) = registry::read_metadata(&cfg.name) {
                    m.last_attach_at = Some(now_iso8601());
                    let _ = registry::write_metadata(&cfg.name, &m);
                }
            }
            DaemonMsg::ClientPeek { id, plain } => {
                let ss = capture(&terminal);
                let payload = if plain { ss.text } else { ss.ansi };
                if let Some(c) = clients.get(&id) {
                    let _ = c.tx.send(encode_screen(payload.as_bytes()));
                }
            }
            DaemonMsg::ClientData(bytes) => {
                let _ = writer.write_all(&bytes);
                let _ = writer.flush();
            }
            DaemonMsg::ClientResize { rows, cols } => {
                cur_rows = rows;
                cur_cols = cols;
                let _ = master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
                let _ = terminal.resize(cols, rows, 0, 0);
            }
            DaemonMsg::ClientStatus { id } => {
                // Count only ATTACHED (streaming) clients — a transient
                // status/peek connection has not attached, so it isn't counted
                // (matches node's `attached`).
                let attached = clients.values().filter(|c| c.streaming).count();
                let json = format!(
                    "{{\"name\":{:?},\"rows\":{},\"cols\":{},\"clients\":{}}}",
                    cfg.name, cur_rows, cur_cols, attached
                );
                if let Some(c) = clients.get(&id) {
                    let _ = c.tx.send(encode_status_response(&json));
                }
            }
            DaemonMsg::ClientDetach { id } => {
                clients.remove(&id);
            }
            DaemonMsg::PtyExited(code) => {
                // Record exit + last screen lines, notify clients.
                let ss = capture(&terminal);
                let tail: Vec<String> = ss
                    .lines
                    .iter()
                    .rev()
                    .take(50)
                    .rev()
                    .cloned()
                    .collect();
                // Persist the final screen so `peek` still works after the
                // daemon/socket is gone (parity with node).
                let _ = registry::write_final_screen(
                    &cfg.name,
                    &registry::FinalScreen {
                        plain: ss.text.clone(),
                        ansi: ss.ansi.clone(),
                    },
                );
                if let Some(mut m) = registry::read_metadata(&cfg.name) {
                    m.exit_code = Some(code);
                    m.exited_at = Some(now_iso8601());
                    m.last_lines = Some(tail);
                    let _ = registry::write_metadata(&cfg.name, &m);
                }
                let packet = encode_exit(code);
                for c in clients.values() {
                    let _ = c.tx.send(packet.clone());
                }
                exit_code_final = code;
                break;
            }
        }
    }

    // ── teardown ──
    let _ = std::fs::remove_file(registry::socket_path(&cfg.name));
    let _ = std::fs::remove_file(registry::pid_path(&cfg.name));
    Ok(exit_code_final)
}

/// Spawn the reader + writer threads for a connected client.
fn spawn_client(id: u64, stream: UnixStream, tx: Sender<DaemonMsg>) {
    let (to_client_tx, to_client_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();
    let _ = tx.send(DaemonMsg::ClientConnect { id, tx: to_client_tx });

    // Writer subthread: drain encoded packets → socket.
    if let Ok(mut wstream) = stream.try_clone() {
        std::thread::spawn(move || {
            while let Ok(bytes) = to_client_rx.recv() {
                if wstream.write_all(&bytes).is_err() {
                    break;
                }
                let _ = wstream.flush();
            }
        });
    }

    // Reader thread: socket packets → DaemonMsg.
    std::thread::spawn(move || {
        let mut stream = stream;
        let mut parser = PacketReader::new();
        let mut buf = [0u8; 8192];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let packets = match parser.feed(&buf[..n]) {
                        Ok(p) => p,
                        Err(_) => break,
                    };
                    for p in packets {
                        match p.type_ {
                            MessageType::Attach => {
                                let (rows, cols) = decode_size(&p.payload);
                                let geometry_neutral = crate::protocol::decode_attach_flags(&p.payload)
                                    & crate::protocol::ATTACH_FLAG_GEOMETRY_NEUTRAL
                                    != 0;
                                let _ = tx.send(DaemonMsg::ClientAttach { id, rows, cols, geometry_neutral });
                            }
                            MessageType::Peek => {
                                let (plain, _full) = decode_peek(&p.payload);
                                let _ = tx.send(DaemonMsg::ClientPeek { id, plain });
                            }
                            MessageType::Data => {
                                let _ = tx.send(DaemonMsg::ClientData(p.payload));
                            }
                            MessageType::Resize => {
                                let (rows, cols) = decode_size(&p.payload);
                                let _ = tx.send(DaemonMsg::ClientResize { rows, cols });
                            }
                            MessageType::Status => {
                                let _ = tx.send(DaemonMsg::ClientStatus { id });
                            }
                            MessageType::Detach => {
                                let _ = tx.send(DaemonMsg::ClientDetach { id });
                            }
                            _ => {}
                        }
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(DaemonMsg::ClientDetach { id });
    });
}

fn exit_code(status: portable_pty::ExitStatus) -> Option<i32> {
    Some(if status.success() { 0 } else { status.exit_code() as i32 })
}

/// A minimal ISO-8601 UTC timestamp (no external date crate).
pub fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Convert epoch seconds to civil date/time (UTC).
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

// Howard Hinnant's days→civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The default session working directory when none is given.
pub fn default_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}
