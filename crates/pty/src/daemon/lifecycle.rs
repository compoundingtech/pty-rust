//! The daemon's life: spawn the child, publish the session, serve clients,
//! record the exit, shut down.
//!
//! Everything that touches the terminal runs on one thread — this one. The
//! PTY reader, the child waiter, the listener, every client socket, the
//! signal handler and the spawner watchdog only send [`Msg`]s here; timers
//! are deadlines the loop wakes for with `recv_timeout`.
//!
//! node: src/server.ts:323-690 (constructor), 571-598 (exit), 1295-1337
//! (exit metadata), 1340-1456 (close, watchdog), 1458-1616 (entry, shutdown)

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use pty_core::events::{Event, EventWriter};
use pty_core::protocol::{Packet, PacketReader, encode_data, encode_exit};
use pty_core::registry::{
    self, MutateOptions, MutateStatus, SESSION_EXIT_LAST_LINES_LIMIT, SessionGenerationOwner,
    SessionMetadata, TagMap,
};
use pty_terminal::{TerminalActor, serialize};

use super::clients::{Client, Out, REDRAW_SETTLE};
use super::config::DaemonConfig;
use super::env::{build_child_env, describe_invalid_cwd, invalid_cwd_error};
use super::tree::{
    KILL_WAIT, ProcessIdentity, TERM_WAIT, signal_process_identities,
    snapshot_descendant_processes, terminate_process_identities,
};

/// What the helper threads tell the actor.
pub(crate) enum Msg {
    PtyData(Vec<u8>),
    PtyEof,
    /// The raw `waitpid` status, `None` when the wait itself failed.
    ChildExited(Option<i32>),
    Connect { id: u64, tx: Sender<Out> },
    Packet { id: u64, packet: Packet },
    Closed { id: u64 },
    /// SIGTERM, SIGINT, or the spawner watchdog.
    ExternalKill,
}

/// Grace after the child's exit before the daemon shuts down, so attached
/// clients receive EXIT.
pub const EXIT_GRACE: Duration = Duration::from_millis(500);
/// How long a child's last output may trail its exit status before the
/// exit is recorded without waiting for the PTY to close.
const EXIT_DRAIN: Duration = Duration::from_millis(300);
/// `saveExitMetadata` retry budget at exit time.
const EXIT_METADATA_RETRY: Duration = Duration::from_millis(400);
/// `saveExitMetadataUntilSettled` budget at close time.
const EXIT_METADATA_SETTLE: Duration = Duration::from_millis(2_000);
/// How long `close()` waits for the child after SIGHUP.
const CHILD_HUP_WAIT: Duration = Duration::from_millis(2_000);
/// …and after SIGKILL.
const CHILD_KILL_WAIT: Duration = Duration::from_millis(500);
/// `SPAWNER_POLL_INTERVAL_MS`.
const SPAWNER_POLL: Duration = Duration::from_millis(5_000);
/// The default `PTY_SHUTDOWN_DEADLINE_MS`.
const SHUTDOWN_DEADLINE_DEFAULT_MS: f64 = 5_000.0;

pub(crate) struct Daemon {
    pub(crate) name: String,
    pub(crate) generation: String,
    pub(crate) cfg: DaemonConfig,
    pub(crate) actor: TerminalActor,
    pub(crate) master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    pub(crate) child_pid: i32,
    pub(crate) clients: BTreeMap<u64, Client>,
    pub(crate) attach_counter: u64,
    pub(crate) last_resize: Option<Instant>,
    pub(crate) settle: Duration,
    pub(crate) exited: bool,
    pub(crate) exit_code: i32,
    pub(crate) events: EventWriter,
    child_status: Option<Option<i32>>,
    pty_eof: bool,
    rx: Receiver<Msg>,
    external_kill: bool,
    shutdown_code: Option<i32>,
    exit_drain_deadline: Option<Instant>,
    exit_shutdown_at: Option<Instant>,
    exit_meta_retry: Option<(Instant, Instant)>,
    listener_fd: i32,
}

/// 32 hex characters, Node's `randomBytes(16).toString("hex")`.
fn new_generation() -> String {
    registry::atomic::random_bytes(16)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `PTY_REDRAW_SETTLE_MS` overrides Node's 80 ms for tests that need a
/// wide synchronization window; the default is Node's.
fn settle_duration() -> Duration {
    std::env::var("PTY_REDRAW_SETTLE_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(REDRAW_SETTLE)
}

/// `PTY_SHUTDOWN_DEADLINE_MS`: finite and > 0, else 5000.
///
/// node: src/server.ts:1535-1538
fn shutdown_deadline() -> Duration {
    let ms = std::env::var("PTY_SHUTDOWN_DEADLINE_MS")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(SHUTDOWN_DEADLINE_DEFAULT_MS);
    Duration::from_millis(ms as u64)
}

/// `code = signal ? 128 + signal : exitCode`.
///
/// node: src/server.ts:571-578
pub fn decode_wait_status(status: Option<i32>) -> (i32, Option<i32>) {
    let Some(status) = status else {
        return (-1, None);
    };
    if libc::WIFSIGNALED(status) {
        let sig = libc::WTERMSIG(status);
        (128 + sig, Some(sig))
    } else if libc::WIFEXITED(status) {
        (libc::WEXITSTATUS(status), None)
    } else {
        (-1, None)
    }
}

/// Node's exit-time reap decision, re-reading the on-disk tags: refuse when
/// the on-disk generation is someone else's; never on an external kill
/// unless ephemeral; else the tag/ephemeral/config precedence.
///
/// node: src/server.ts:1481-1524
pub fn reap_at_exit(
    name: &str,
    generation: &str,
    external_kill: bool,
    ephemeral: bool,
    config_tags: Option<&TagMap>,
) -> bool {
    let metadata = registry::read_metadata(name);
    if let Some(g) = metadata.as_ref().and_then(|m| m.generation.as_deref())
        && g != generation
    {
        return false;
    }
    if external_kill && !ephemeral {
        return false;
    }
    let tags = metadata.as_ref().and_then(|m| m.tags.as_ref()).or(config_tags);
    registry::should_reap_at_exit(tags, ephemeral, registry::reap_on_exit_default())
}

fn pid_alive(pid: i32) -> bool {
    registry::pid_alive(pid)
}

fn kill(pid: i32, signal: i32) {
    if pid > 0 {
        // SAFETY: kill(2) on the child we spawned.
        unsafe {
            libc::kill(pid, signal);
        }
    }
}

/// Run the daemon for `cfg` to completion; the return value is the process
/// exit status (the child's code after a natural exit, 0 after a kill).
pub fn run(cfg: DaemonConfig) -> Result<i32, String> {
    let name = cfg.name.clone();
    let generation = cfg
        .generation
        .clone()
        .filter(|g| !g.is_empty())
        .unwrap_or_else(new_generation);
    let events = EventWriter::new(&name);
    let (rows, cols, cwd) = (cfg.rows(), cfg.cols(), cfg.cwd());

    let child_env = build_child_env(&cfg, &generation)?;
    if let Some(reason) = describe_invalid_cwd(&cwd) {
        return Err(invalid_cwd_error(&reason, &name, &cfg.command));
    }

    // The child: `/bin/sh -c 'exec "$@"' sh <command> <args...>`, so PATH
    // lookups, shebangs and symlinks behave like a shell's.
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Failed to open a PTY for session \"{name}\": {e}"))?;
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "exec \"$@\"", "sh"]);
    command.arg(&cfg.command);
    command.args(&cfg.args);
    command.cwd(&cwd);
    command.env_clear();
    for (k, v) in &child_env {
        command.env(k, v);
    }
    let child = pair.slave.spawn_command(command).map_err(|e| {
        format!(
            "Failed to spawn PTY shell \"/bin/sh\" for command \"{}\" in cwd \"{cwd}\": {e}",
            cfg.command
        )
    })?;
    drop(pair.slave);
    let child_pid = child.process_id().map(|p| p as i32).unwrap_or(0);
    // The child is reaped by the waiter thread below, never through this handle.
    std::mem::forget(child);
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("Failed to read the PTY for session \"{name}\": {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("Failed to write the PTY for session \"{name}\": {e}"))?;

    let (tx, rx) = mpsc::channel::<Msg>();
    spawn_pty_reader(reader, tx.clone());
    spawn_child_waiter(child_pid, tx.clone());

    // Publication: dir → clear events → stale socket → listen (umask 077,
    // chmod 600) → pid → metadata → session_start.
    registry::ensure_session_dir().map_err(|e| e.to_string())?;
    pty_core::events::clear_events(&name)?;
    let socket_path = registry::socket_path(&name);
    let _ = std::fs::remove_file(&socket_path);
    // SAFETY: umask(2) has no preconditions.
    let prev_umask = unsafe { libc::umask(0o077) };
    let listener = UnixListener::bind(&socket_path);
    // SAFETY: restoring the mask we read above.
    unsafe {
        libc::umask(prev_umask);
    }
    let listener = listener.map_err(|e| format!("Socket server error: {e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600));
    }
    registry::write_pid(&name, std::process::id()).map_err(|e| e.to_string())?;
    let created_at = registry::now_iso8601();
    let metadata = SessionMetadata {
        generation: Some(generation.clone()),
        daemon_pid: Some(std::process::id() as i32),
        recovery: None,
        command: cfg.command.clone(),
        args: cfg.args.clone(),
        display_command: cfg.display_command.clone(),
        cwd: cwd.clone(),
        rows: Some(rows),
        cols: Some(cols),
        ephemeral: Some(cfg.ephemeral),
        created_at,
        display_name: cfg.display_name().map(str::to_string),
        tags: cfg.tags().cloned(),
        isolate_env: cfg.isolate_env().then_some(true),
        extra_env: cfg.extra_env().cloned(),
        unset_env: (!cfg.unset_env().is_empty()).then(|| cfg.unset_env().to_vec()),
        env: cfg.env.clone(),
        ..Default::default()
    };
    registry::write_metadata_publication(&name, &metadata).map_err(|e| e.to_string())?;
    events.append(Event::session_start(&name, cfg.tags()));
    events.flush();

    let listener_fd = listener.as_raw_fd();
    spawn_acceptor(listener, tx.clone());
    spawn_signal_listener(tx.clone());
    install_spawner_watchdog(tx);

    let daemon = Daemon {
        name,
        generation,
        cfg,
        actor: TerminalActor::new(rows, cols, pty_terminal::actor::DEFAULT_SCROLLBACK),
        master: pair.master,
        writer,
        child_pid,
        clients: BTreeMap::new(),
        attach_counter: 0,
        last_resize: None,
        settle: settle_duration(),
        exited: false,
        exit_code: 0,
        events,
        child_status: None,
        pty_eof: false,
        rx,
        external_kill: false,
        shutdown_code: None,
        exit_drain_deadline: None,
        exit_shutdown_at: None,
        exit_meta_retry: None,
        listener_fd,
    };
    Ok(daemon.serve())
}

fn spawn_pty_reader(mut reader: Box<dyn Read + Send>, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 16384];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(Msg::PtyData(buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = tx.send(Msg::PtyEof);
    });
}

fn spawn_child_waiter(pid: i32, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let mut status = 0i32;
        loop {
            // SAFETY: waitpid on our own child.
            let r = unsafe { libc::waitpid(pid, &mut status, 0) };
            if r == pid {
                let _ = tx.send(Msg::ChildExited(Some(status)));
                return;
            }
            if r < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            let _ = tx.send(Msg::ChildExited(None));
            return;
        }
    });
}

fn spawn_acceptor(listener: UnixListener, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let ids = Arc::new(AtomicU64::new(1));
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                break;
            };
            let id = ids.fetch_add(1, Ordering::Relaxed);
            spawn_client(id, stream, tx.clone());
        }
    });
}

/// One writer thread (packets → socket) and one reader thread (socket →
/// [`Msg`]) per connection.
fn spawn_client(id: u64, stream: UnixStream, tx: Sender<Msg>) {
    let (out_tx, out_rx) = mpsc::channel::<Out>();
    let Ok(mut wstream) = stream.try_clone() else {
        return;
    };
    let _ = tx.send(Msg::Connect { id, tx: out_tx });
    std::thread::spawn(move || {
        while let Ok(out) = out_rx.recv() {
            match out {
                Out::Bytes(bytes) => {
                    if wstream.write_all(&bytes).is_err() {
                        break;
                    }
                }
                Out::End => {
                    let _ = wstream.shutdown(std::net::Shutdown::Write);
                }
                Out::Destroy => break,
            }
        }
        let _ = wstream.shutdown(std::net::Shutdown::Both);
    });
    std::thread::spawn(move || {
        let mut stream = stream;
        let mut parser = PacketReader::new();
        let mut buf = [0u8; 16384];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => match parser.feed(&buf[..n]) {
                    Ok(packets) => {
                        for packet in packets {
                            if tx.send(Msg::Packet { id, packet }).is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        // A peer declaring an oversize frame is dropped
                        // rather than buffered without bound.
                        crate::daemon::daemon_warn!("Rejected client packet: {e}");
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                        break;
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = tx.send(Msg::Closed { id });
    });
}

/// SIGTERM and SIGINT are external kills.
///
/// node: src/server.ts:1598-1603
fn spawn_signal_listener(tx: Sender<Msg>) {
    let Ok(mut signals) =
        signal_hook::iterator::Signals::new([signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT])
    else {
        return;
    };
    std::thread::spawn(move || {
        for _ in signals.forever() {
            if tx.send(Msg::ExternalKill).is_err() {
                return;
            }
        }
    });
}

/// `PTY_SPAWNER_PID`: an integer > 1; dead at boot → shut down now; else
/// poll every 5 s.
///
/// node: src/server.ts:1439-1456
fn install_spawner_watchdog(tx: Sender<Msg>) {
    let Some(raw) = std::env::var("PTY_SPAWNER_PID").ok().filter(|r| !r.is_empty()) else {
        return;
    };
    let Some(pid) = raw
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|p| p.fract() == 0.0 && *p > 1.0 && *p <= i32::MAX as f64)
        .map(|p| p as i32)
    else {
        return;
    };
    if !pid_alive(pid) {
        let _ = tx.send(Msg::ExternalKill);
        return;
    }
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(SPAWNER_POLL);
            if !pid_alive(pid) {
                let _ = tx.send(Msg::ExternalKill);
                return;
            }
        }
    });
}

impl Daemon {
    fn owner(&self) -> SessionGenerationOwner {
        SessionGenerationOwner {
            generation: self.generation.clone(),
            pid: std::process::id() as i32,
        }
    }

    pub(crate) fn write_pty(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || self.child_status.is_some() {
            return;
        }
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// The serving loop, then the shutdown. Returns the process exit status.
    fn serve(mut self) -> i32 {
        loop {
            let now = Instant::now();
            let deadline = [
                self.next_cut_deadline(),
                self.exit_drain_deadline,
                self.exit_shutdown_at,
                self.exit_meta_retry.map(|(next, _)| next),
            ]
            .into_iter()
            .flatten()
            .min();
            let msg = match deadline {
                Some(d) => match self.rx.recv_timeout(d.saturating_duration_since(now)) {
                    Ok(m) => Some(m),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match self.rx.recv() {
                    Ok(m) => Some(m),
                    Err(_) => break,
                },
            };
            if let Some(m) = msg {
                self.handle(m);
            }
            self.service_timers(Instant::now());
            if let Some(code) = self.shutdown_code {
                return self.close(code);
            }
        }
        let code = if self.exited { self.exit_code } else { 0 };
        self.close(code)
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::PtyData(bytes) => self.on_pty_data(&bytes),
            Msg::PtyEof => {
                self.pty_eof = true;
                if self.child_status.is_some() && !self.exited {
                    self.finalize_exit();
                }
            }
            Msg::ChildExited(status) => {
                if self.child_status.is_some() {
                    return;
                }
                self.child_status = Some(status);
                if self.pty_eof {
                    if !self.exited {
                        self.finalize_exit();
                    }
                } else {
                    self.exit_drain_deadline = Some(Instant::now() + EXIT_DRAIN);
                }
            }
            Msg::Connect { id, tx } => {
                self.clients
                    .insert(id, Client::new(tx, self.actor.rows(), self.actor.cols()));
            }
            Msg::Packet { id, packet } => self.on_packet(id, packet),
            Msg::Closed { id } => self.on_closed(id),
            Msg::ExternalKill => {
                if self.shutdown_code.is_none() {
                    self.external_kill = true;
                    self.shutdown_code = Some(0);
                }
            }
        }
    }

    /// Child output: into the terminal (queries answered back to the
    /// child), events to the log, the remainder to live clients.
    ///
    /// node: src/server.ts:559-569
    fn on_pty_data(&mut self, bytes: &[u8]) {
        let cleaned = self.actor.write(bytes);
        let replies = self.actor.take_pty_replies();
        self.write_pty(&replies);
        self.forward_terminal_events();
        if !cleaned.is_empty() {
            self.broadcast(&encode_data(&cleaned));
        }
    }

    fn service_timers(&mut self, now: Instant) {
        self.service_cuts(now);
        if let Some(d) = self.exit_drain_deadline
            && d <= now
        {
            self.exit_drain_deadline = None;
            if !self.exited && self.child_status.is_some() {
                self.finalize_exit();
            }
        }
        if let Some((next, deadline)) = self.exit_meta_retry
            && next <= now
        {
            match self.save_exit_metadata() {
                MutateStatus::Busy | MutateStatus::Stale if now < deadline => {
                    self.exit_meta_retry = Some((now + Duration::from_millis(10), deadline));
                }
                _ => self.exit_meta_retry = None,
            }
        }
        if let Some(at) = self.exit_shutdown_at
            && at <= now
        {
            self.exit_shutdown_at = None;
            if self.shutdown_code.is_none() {
                self.shutdown_code = Some(self.exit_code);
            }
        }
    }

    /// The child is gone: EXIT to live clients (settling ones get it after
    /// their SCREEN), `session_exit`, exit metadata, shutdown in 500 ms.
    ///
    /// node: src/server.ts:571-598
    fn finalize_exit(&mut self) {
        let (code, signal) = decode_wait_status(self.child_status.flatten());
        self.exited = true;
        self.exit_code = code;
        self.exit_drain_deadline = None;
        self.broadcast(&encode_exit(code));
        self.events
            .append(Event::session_exit(&self.name, code, signal));
        if matches!(
            self.save_exit_metadata(),
            MutateStatus::Busy | MutateStatus::Stale
        ) {
            let now = Instant::now();
            self.exit_meta_retry = Some((now + Duration::from_millis(10), now + EXIT_METADATA_RETRY));
        }
        if self.shutdown_code.is_none() {
            self.exit_shutdown_at = Some(Instant::now() + EXIT_GRACE);
        }
    }

    /// All rows, trailing empties trimmed, the last 200.
    ///
    /// node: src/server.ts:1295-1309
    fn last_lines(&self) -> Vec<String> {
        let lines = serialize::plain_lines_full(self.actor.terminal());
        let start = lines.len().saturating_sub(SESSION_EXIT_LAST_LINES_LIMIT);
        lines[start..].to_vec()
    }

    /// node: src/server.ts:1311-1319
    fn save_exit_metadata(&self) -> MutateStatus {
        let code = self.exit_code;
        let last_lines = self.last_lines();
        registry::mutate_metadata_under_lock(
            &self.name,
            move |m| {
                m.exit_code = Some(code);
                m.exited_at = Some(registry::now_iso8601());
                m.last_lines = Some(last_lines);
                true
            },
            &MutateOptions {
                expected_generation: Some(self.generation.clone()),
                expected_metadata: None,
            },
        )
    }

    /// node: src/server.ts:1321-1337
    fn save_exit_metadata_until_settled(&self, budget: Duration) {
        let deadline = Instant::now() + budget;
        loop {
            match self.save_exit_metadata() {
                MutateStatus::Changed(_)
                | MutateStatus::Unchanged(_)
                | MutateStatus::Missing
                | MutateStatus::GenerationMismatch => return,
                MutateStatus::Busy | MutateStatus::Stale => {}
            }
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn reap_at_exit(&self) -> bool {
        reap_at_exit(
            &self.name,
            &self.generation,
            self.external_kill,
            self.cfg.ephemeral,
            self.cfg.tags(),
        )
    }

    /// The hard deadline behind a graceful shutdown.
    ///
    /// node: src/server.ts:1545-1558
    fn start_backstop(&self, code: i32, descendants: Arc<Mutex<Vec<ProcessIdentity>>>) {
        let deadline = shutdown_deadline();
        let name = self.name.clone();
        let generation = self.generation.clone();
        let (external, ephemeral) = (self.external_kill, self.cfg.ephemeral);
        let tags = self.cfg.tags().cloned();
        let child_pid = self.child_pid;
        let owner = self.owner();
        std::thread::spawn(move || {
            std::thread::sleep(deadline);
            crate::daemon::daemon_warn!(
                "pty daemon \"{name}\": graceful shutdown exceeded {}ms — forcing exit (child reaped)",
                deadline.as_millis()
            );
            kill(child_pid, libc::SIGKILL);
            let descendants = descendants.lock().map(|d| d.clone()).unwrap_or_default();
            signal_process_identities(&descendants, libc::SIGKILL);
            if reap_at_exit(&name, &generation, external, ephemeral, tags.as_ref()) {
                registry::cleanup_owned_all(&name, &owner);
            } else {
                registry::cleanup_owned_socket(&name, &owner);
            }
            std::process::exit(code);
        });
    }

    /// Wait until the child's exit has been recorded, still feeding its
    /// last output into the terminal. `true` when it exited in time.
    fn wait_child_exit(&mut self, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        while !self.exited {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let wake = self
                .exit_drain_deadline
                .map_or(deadline, |d| d.min(deadline));
            match self.rx.recv_timeout(wake.saturating_duration_since(now)) {
                Ok(Msg::PtyData(bytes)) => self.on_pty_data(&bytes),
                Ok(msg @ (Msg::PtyEof | Msg::ChildExited(_))) => self.handle(msg),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return self.exited,
            }
            let now = Instant::now();
            if let Some(d) = self.exit_drain_deadline
                && d <= now
            {
                self.exit_drain_deadline = None;
                if !self.exited && self.child_status.is_some() {
                    self.finalize_exit();
                }
            }
        }
        true
    }

    /// Node's `close()` followed by the reap decision.
    ///
    /// node: src/server.ts:1340-1408, 1559-1568
    fn close(mut self, code: i32) -> i32 {
        let descendants = Arc::new(Mutex::new(Vec::new()));
        if self.external_kill
            && let Ok(mut d) = descendants.lock()
        {
            *d = snapshot_descendant_processes(self.child_pid);
        }
        self.start_backstop(code, descendants.clone());
        if self.exited {
            self.save_exit_metadata();
        }
        for (_, c) in std::mem::take(&mut self.clients) {
            let _ = c.tx.send(Out::Destroy);
        }
        // SAFETY: shutdown on the listening socket unblocks accept(2).
        unsafe {
            libc::shutdown(self.listener_fd, libc::SHUT_RDWR);
        }
        registry::cleanup_owned_socket(&self.name, &self.owner());
        if self.child_status.is_none() {
            kill(self.child_pid, libc::SIGHUP);
        }
        let descendant_wait = self.external_kill.then(|| {
            let ids = descendants.lock().map(|d| d.clone()).unwrap_or_default();
            std::thread::spawn(move || terminate_process_identities(&ids, TERM_WAIT, KILL_WAIT))
        });
        if !self.wait_child_exit(CHILD_HUP_WAIT) {
            kill(self.child_pid, libc::SIGKILL);
            self.wait_child_exit(CHILD_KILL_WAIT);
        }
        let survivors = descendant_wait
            .and_then(|t| t.join().ok())
            .unwrap_or_default();
        if !survivors.is_empty() {
            crate::daemon::daemon_warn!(
                "pty daemon \"{}\": {} child process(es) did not exit after exact TERM and KILL signals",
                self.name,
                survivors.len()
            );
        }
        if self.exited {
            self.save_exit_metadata_until_settled(EXIT_METADATA_SETTLE);
        }
        self.events.flush();
        if self.reap_at_exit() {
            registry::cleanup_owned_all(&self.name, &self.owner());
        }
        code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_status_maps_signals_to_128_plus() {
        // SIGKILL death: status = 9.
        assert_eq!(decode_wait_status(Some(9)), (137, Some(9)));
        // exit 5: status = 5 << 8.
        assert_eq!(decode_wait_status(Some(5 << 8)), (5, None));
        assert_eq!(decode_wait_status(Some(0)), (0, None));
        assert_eq!(decode_wait_status(None), (-1, None));
    }

    #[test]
    fn generation_is_32_hex() {
        let g = new_generation();
        assert_eq!(g.len(), 32);
        assert!(g.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
