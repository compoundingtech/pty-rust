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

use libghostty_vt::terminal::{Mode, Options, Terminal};
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
    /// `-e/--ephemeral`: force reap-on-exit (see [`should_reap`]).
    pub ephemeral: bool,
    /// Session tags — `keep=true` forces preserve, `strategy=permanent` too.
    pub tags: std::collections::BTreeMap<String, String>,
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
    geometry_neutral: bool,
}

/// The child (session) pid, for the signal handler to forward to. Node records
/// the DAEMON pid in `<name>.pid`, so `kill` targets the daemon; the daemon then
/// forwards the signal to the child, which triggers a clean exit + metadata.
static CHILD_PID: AtomicI32 = AtomicI32::new(0);

/// Set by the signal handler when the daemon is stopped EXTERNALLY (kill /
/// SIGTERM / SIGINT). An external stop preserves the session regardless of the
/// reap config (unless ephemeral) — only a child terminating on its own
/// consults the config default.
static EXTERNAL_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_external_stop(_sig: i32) {
    EXTERNAL_STOP.store(true, Ordering::SeqCst);
    let pid = CHILD_PID.load(Ordering::SeqCst);
    if pid > 0 {
        // SAFETY: kill() is async-signal-safe. SIGHUP (terminal hangup) is the
        // natural "session ended" signal — interactive shells (which IGNORE
        // SIGTERM) and most programs terminate on it. A watchdog thread
        // escalates to SIGKILL if the child ignores SIGHUP too.
        unsafe {
            libc::kill(pid, libc::SIGHUP);
        }
    }
}

/// Falsey values for `PTY_REAP_ON_EXIT` (→ PRESERVE). Same set node uses.
fn is_falsey(v: &str) -> bool {
    matches!(v.trim().to_lowercase().as_str(), "false" | "0" | "no" | "off")
}

/// The reap default from `PTY_REAP_ON_EXIT`: unset → reap; falsey → preserve;
/// anything else → reap.
fn config_reap() -> bool {
    match std::env::var("PTY_REAP_ON_EXIT") {
        Ok(v) => !is_falsey(&v),
        Err(_) => true,
    }
}

/// Decide whether to reap (remove) the session on exit. Precedence (node #114):
/// an external stop preserves unless ephemeral; otherwise keep=true → preserve,
/// ephemeral → reap, strategy=permanent → preserve, else the config default.
pub fn should_reap(
    external_stop: bool,
    ephemeral: bool,
    keep: bool,
    permanent: bool,
    config_reap: bool,
) -> bool {
    if external_stop {
        // External stop preserves unless the session is ephemeral.
        return ephemeral;
    }
    if keep {
        return false; // preserve (also gc-exempt)
    }
    if ephemeral {
        return true; // reap
    }
    if permanent {
        return false; // preserve (supervisor respawn needs metadata)
    }
    config_reap
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
        let handler = on_external_stop as extern "C" fn(i32);
        libc::signal(libc::SIGTERM, handler as libc::sighandler_t);
        libc::signal(libc::SIGINT, handler as libc::sighandler_t);
    }
    // Watchdog: on an external stop, if the child ignores SIGHUP (e.g. a
    // program that traps it), escalate to SIGKILL after a short grace so the
    // daemon can proceed to its clean shutdown.
    std::thread::spawn(move || loop {
        if EXTERNAL_STOP.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if child_pid > 0 {
                unsafe {
                    libc::kill(child_pid, libc::SIGKILL);
                }
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    });

    let reader = pair.master.try_clone_reader().map_err(std::io::Error::other)?;
    let writer = pair.master.take_writer().map_err(std::io::Error::other)?;
    let master = pair.master;

    // ── metadata + pid ──
    let created_at = now_iso8601();
    let created_epoch = now_epoch_f64();
    let meta = SessionMetadata {
        command: cfg.command.clone(),
        args: cfg.args.clone(),
        display_command: cfg.display_command.clone(),
        cwd: cfg.cwd.clone(),
        created_at: created_at.clone(),
        exit_code: None,
        exited_at: None,
        last_lines: None,
        tags: if cfg.tags.is_empty() {
            None
        } else {
            Some(cfg.tags.clone())
        },
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
                clients.insert(
                    id,
                    Client {
                        tx,
                        streaming: false,
                        geometry_neutral: false,
                    },
                );
            }
            DaemonMsg::ClientAttach { id, rows, cols, geometry_neutral } => {
                if let Some(c) = clients.get_mut(&id) {
                    c.streaming = true;
                    c.geometry_neutral = geometry_neutral;
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
                // Client counts (streaming only; transient status/peek
                // connections are not counted). geometry-neutral streaming
                // clients (e.g. `peek -f`) count as read-only.
                let attached = clients
                    .values()
                    .filter(|c| c.streaming && !c.geometry_neutral)
                    .count();
                let read_only = clients
                    .values()
                    .filter(|c| c.streaming && c.geometry_neutral)
                    .count();
                let kitty_bits = terminal
                    .kitty_keyboard_flags()
                    .map(|f| f.bits())
                    .unwrap_or(0);
                let stats = crate::stats::StatsResult {
                    name: cfg.name.clone(),
                    terminal: crate::stats::TerminalStats {
                        cols: cur_cols,
                        rows: cur_rows,
                        cursor_x: terminal.cursor_x().unwrap_or(0),
                        cursor_y: terminal.cursor_y().unwrap_or(0),
                        scrollback_used: terminal.scrollback_rows().unwrap_or(0),
                        scrollback_capacity: cur_rows as usize + 10_000,
                    },
                    process: crate::stats::ProcessStats {
                        alive: true,
                        exit_code: None,
                        pid: Some(child_pid),
                        resources: crate::stats::read_resources(child_pid),
                    },
                    daemon: crate::stats::DaemonStats {
                        pid: std::process::id() as i32,
                        resources: crate::stats::read_resources(std::process::id() as i32),
                    },
                    clients: crate::stats::ClientStats {
                        total: attached + read_only,
                        attached,
                        read_only,
                        geometry_neutral: if read_only > 0 { Some(read_only) } else { None },
                    },
                    modes: crate::stats::ModeStats {
                        sgr_mouse: terminal.mode(Mode::SGR_MOUSE).unwrap_or(false),
                        cursor_hidden: !terminal.is_cursor_visible().unwrap_or(true),
                        kitty_keyboard: kitty_bits != 0,
                        kitty_keyboard_flags: if kitty_bits != 0 {
                            vec![kitty_bits]
                        } else {
                            vec![]
                        },
                    },
                    uptime_seconds: Some((now_epoch_f64() - created_epoch).max(0.0)),
                    created_at: Some(created_at.clone()),
                };
                let json = serde_json::to_string(&stats).unwrap_or_else(|_| "{}".into());
                if let Some(c) = clients.get(&id) {
                    let _ = c.tx.send(encode_status_response(&json));
                }
            }
            DaemonMsg::ClientDetach { id } => {
                clients.remove(&id);
            }
            DaemonMsg::PtyExited(code) => {
                // Decide reap vs preserve (node #114 precedence).
                let external = EXTERNAL_STOP.load(Ordering::SeqCst);
                let keep = cfg.tags.get("keep").map(|v| v == "true").unwrap_or(false);
                let permanent = cfg
                    .tags
                    .get("strategy")
                    .map(|v| v == "permanent")
                    .unwrap_or(false);
                let reap = should_reap(external, cfg.ephemeral, keep, permanent, config_reap());

                // Notify clients of the exit either way.
                let packet = encode_exit(code);
                for c in clients.values() {
                    let _ = c.tx.send(packet.clone());
                }

                if reap {
                    // Remove the session entirely (metadata + events + socket +
                    // pid + final-screen). Post-exit peek -> ENOENT; ls omits it.
                    registry::cleanup(&cfg.name);
                } else {
                    // Preserve: keep the session as status=exited, peekable.
                    let ss = capture(&terminal);
                    let tail: Vec<String> =
                        ss.lines.iter().rev().take(50).rev().cloned().collect();
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

/// Current epoch time in seconds (fractional), for uptime.
pub fn now_epoch_f64() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
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
