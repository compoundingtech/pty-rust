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
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::MasterPty;
use pty_core::capability;
use pty_core::events::{Event, EventWriter};
use pty_core::protocol::{
    AcceptedSocketOwnershipResult, LifecycleCompareAndSetRequest, LifecycleCompareAndSetResult,
    Packet, PacketReader, decode_accepted_socket_ownership_request,
    decode_lifecycle_compare_and_set_request, encode_accepted_socket_ownership_response,
    encode_data, encode_exit, encode_lifecycle_compare_and_set_response,
};
use pty_core::registry::{
    self, MutateOptions, MutateStatus, SESSION_EXIT_LAST_LINES_LIMIT, SessionGenerationOwner,
    SessionMetadata, TagCompareAndSetResult, TagMap,
};
use pty_terminal::{TerminalActor, serialize};

use super::DaemonConfig;
use super::clients::{Client, Out, REDRAW_SETTLE, Role};
use super::daemon_warn;
use super::env::{build_child_env, describe_invalid_cwd, invalid_cwd_error};
use super::tree::{
    CompleteProcessTreeSnapshot, KILL_WAIT, ProcessIdentity, TERM_WAIT, capture_process_identity,
    control_peer_is_authorized, freeze_descendant_processes, signal_process_identities,
    snapshot_descendant_processes_complete_for, terminate_process_group,
    terminate_process_identities,
};
use pty_lifecycle::{
    ArmedStartupLease, StartupLeaseTerminalCause, arm_startup_lease, monotonic_now_ns,
    remaining_lease_delay, startup_lease_deadline_cause, terminal_startup_lease_value,
};

/// What the helper threads tell the actor.
pub(crate) enum Msg {
    PtyData(Vec<u8>),
    PtyEof,
    /// The raw `waitpid` status, `None` when the wait itself failed.
    ChildExited(Option<i32>),
    Connect {
        id: u64,
        tx: Sender<Out>,
        peer: Option<pty_core::unix_peer::PeerCredentials>,
        capability_fd: Option<OwnedFd>,
    },
    Packet {
        id: u64,
        packet: Packet,
    },
    Closed {
        id: u64,
    },
    /// SIGTERM, SIGINT, or the spawner watchdog.
    ExternalKill,
    OwnershipResult {
        id: u64,
        result: AcceptedSocketOwnershipResult,
    },
    StartupLeaseResponseReleased,
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
/// Retry cadence for a lifecycle terminal write blocked by the metadata lock.
const STARTUP_LEASE_RETRY: Duration = Duration::from_millis(10);
/// Socket-write backstop for response-before-deadline-shutdown ordering.
const STARTUP_RESPONSE_BACKSTOP: Duration = Duration::from_secs(2);

pub(crate) struct Daemon {
    pub(crate) name: String,
    pub(crate) generation: String,
    pub(crate) cfg: DaemonConfig,
    pub(crate) actor: TerminalActor,
    pub(crate) master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    pub(crate) child_pid: i32,
    child_identity: Option<ProcessIdentity>,
    daemon_identity: Option<ProcessIdentity>,
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
    /// Unix milliseconds for the newest child output, in memory. `None`
    /// until the child prints something.
    last_output_at_ms: Option<i64>,
    /// When the pending activity write is due. A trailing-edge debounce: the
    /// first chunk after a quiet period schedules one write a second out, and
    /// every chunk inside that window folds into it, so a chatty session
    /// costs one metadata write per second rather than one per chunk.
    activity_persist_at: Option<Instant>,
    listener_fd: i32,
    startup_lease: Option<ArmedStartupLease>,
    startup_lease_timer: Option<Instant>,
    startup_lease_retry: Option<(Instant, bool)>,
    startup_lease_disarmed: bool,
    startup_lease_terminal_cause: Option<StartupLeaseTerminalCause>,
    startup_lease_terminal_value: Option<String>,
    startup_lease_deadline_notified: bool,
    startup_lease_deadline_notification_pending: bool,
    startup_lease_deadline_response_released: bool,
    shutdown_descendants: Vec<ProcessIdentity>,
    shutdown_tree_unavailable: Option<String>,
    shutdown_tree_snapshotted: bool,
    shutdown_backstop_descendants: Option<Arc<Mutex<Vec<ProcessIdentity>>>>,
    capability_stream: Option<UnixStream>,
    actor_tx: Sender<Msg>,
}

/// How long the activity write waits after the first chunk of a burst.
///
/// node: src/server.ts `scheduleActivityPersist` (1 s), the Node pty
/// repository's `docs/vrs` R14 (not this repository's `docs/vrs`).
const ACTIVITY_PERSIST_DEBOUNCE: Duration = Duration::from_secs(1);

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
    let tags = metadata
        .as_ref()
        .and_then(|m| m.tags.as_ref())
        .or(config_tags);
    registry::should_reap_at_exit(tags, ephemeral, registry::reap_on_exit_default())
}

fn pid_alive(pid: i32) -> bool {
    registry::pid_alive(pid)
}

fn set_fd_cloexec(fd: RawFd) -> bool {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    flags >= 0
        && unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == 0
}

fn should_challenge_capability(
    control_capability: bool,
    capability_fd_present: bool,
    peer_authorized: bool,
) -> bool {
    control_capability && capability_fd_present && peer_authorized
}

fn signal_child(identity: Option<&ProcessIdentity>, signal: i32) {
    if let Some(identity) = identity {
        signal_process_identities(std::slice::from_ref(identity), signal);
    }
}

/// Run the daemon for `cfg` to completion; the return value is the process
/// exit status (the child's code after a natural exit, 0 after a kill).
pub(crate) fn run(cfg: DaemonConfig, readiness: super::ReadyNotifier) -> Result<i32, String> {
    let name = cfg.name.clone();
    let generation = cfg
        .generation
        .clone()
        .filter(|g| !g.is_empty())
        .unwrap_or_else(new_generation);
    let startup_lease = cfg
        .startup_lease
        .as_ref()
        .map(|options| arm_startup_lease(options, &generation))
        .transpose()?;
    let mut published_tags = cfg.tags().cloned().unwrap_or_default();
    if let Some(lease) = &startup_lease {
        published_tags.insert(lease.lifecycle_tag.clone(), lease.starting_value.clone());
    }
    let events = EventWriter::new(&name);
    let (rows, cols, cwd) = (cfg.rows(), cfg.cols(), cfg.cwd());

    let child_env = build_child_env(&cfg, &generation)?;
    if let Some(reason) = describe_invalid_cwd(&cwd) {
        return Err(invalid_cwd_error(&reason, &name, &cfg.command));
    }

    // The PTY pair first: opening it fails before anything is published.
    let pair = pty_spawn::open(rows, cols)
        .map_err(|e| format!("Failed to open a PTY for session \"{name}\": {e}"))?;

    // Publication before the child spawns: dir → clear events → stale
    // socket → listen (umask 077, chmod 600) → pid → metadata →
    // session_start. An attached child's first action runs after this block,
    // so its session record — the `<name>.pid` owner sidecar, the metadata,
    // and the `session_start` line `pty run` waits for — is already on disk
    // when the child starts (compoundingtech/pty#180). Spawning first left
    // the child racing publication: its immediate `metadata patch` met the
    // creation lock its own `pty run` parent still held, and the sidecar
    // could be unpublished when it first ran.
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
        daemon_start_token: registry::read_process_start_token(std::process::id() as i32),
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
        tags: (!published_tags.is_empty()).then(|| published_tags.clone()),
        isolate_env: cfg.isolate_env().then_some(true),
        extra_env: cfg.extra_env().cloned(),
        unset_env: (!cfg.unset_env().is_empty()).then(|| cfg.unset_env().to_vec()),
        env: cfg.env.clone(),
        ..Default::default()
    };
    registry::write_metadata_publication(&name, &metadata).map_err(|e| e.to_string())?;
    events.append(Event::session_start(
        &name,
        (!published_tags.is_empty()).then_some(&published_tags),
    ));
    if cfg.respawn {
        events.append(Event::session_respawn(&name));
    }
    events.flush();
    let capability_stream = std::env::var("PTY_READINESS_CAP_FD")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|&fd| fd >= 0)
        .map(|fd| unsafe { UnixStream::from_raw_fd(fd) });
    if let Some(stream) = &capability_stream {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        // The descriptor must survive the daemon exec, but never cross the
        // daemon's subsequent managed-child exec boundary.
        if !set_fd_cloexec(stream.as_raw_fd()) {
            return Err("Failed to mark readiness capability descriptor CLOEXEC".to_string());
        }
    }


    // The child: `/bin/sh -c 'exec "$@"' sh <command> <args...>`, so PATH
    // lookups, shebangs and symlinks behave like a shell's.
    let mut command = pty_spawn::shell_exec(&cfg.command, &cfg.args);
    command.cwd(&cwd);
    command.env_clear();
    for (k, v) in &child_env {
        command.env(k, v);
    }
    let child = match pair.slave.spawn_command(command) {
        Ok(child) => child,
        Err(error) => {
            // The creation-lock owner is waiting for daemon readiness, so a
            // normal metadata mutation would deadlock here. We are still the
            // only publisher for this generation: replace `starting` directly
            // before withdrawing the liveness artifacts.
            if let Some(lease) = &startup_lease {
                let mut terminal_metadata = metadata.clone();
                terminal_metadata
                    .tags
                    .get_or_insert_with(TagMap::new)
                    .insert(
                        lease.lifecycle_tag.clone(),
                        terminal_startup_lease_value(&generation, StartupLeaseTerminalCause::Exit),
                    );
                let _ = registry::write_metadata_publication(&name, &terminal_metadata);
            }
            let _ = std::fs::remove_file(&socket_path);
            let _ = std::fs::remove_file(registry::pid_path(&name));
            return Err(format!(
                "Failed to spawn PTY shell \"/bin/sh\" for command \"{}\" in cwd \"{cwd}\": {error}",
                cfg.command
            ));
        }
    };
    drop(pair.slave);
    let child_pid = child.process_id().map(|p| p as i32).unwrap_or(0);
    let child_identity = capture_process_identity(child_pid);
    let daemon_identity = capture_process_identity(std::process::id() as i32);
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

    let listener_fd = listener.as_raw_fd();
    spawn_acceptor(listener, tx.clone());
    spawn_signal_listener(tx.clone());
    install_spawner_watchdog(tx.clone());

    let daemon = Daemon {
        name,
        generation,
        cfg,
        actor: terminal_actor(rows, cols),
        master: pair.master,
        writer,
        child_pid,
        child_identity,
        daemon_identity,
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
        last_output_at_ms: None,
        activity_persist_at: None,
        listener_fd,
        startup_lease_timer: startup_lease
            .as_ref()
            .map(|lease| Instant::now() + remaining_lease_delay(lease.deadline_monotonic_ns)),
        startup_lease,
        startup_lease_retry: None,
        startup_lease_disarmed: false,
        startup_lease_terminal_cause: None,
        startup_lease_terminal_value: None,
        startup_lease_deadline_notified: false,
        startup_lease_deadline_notification_pending: false,
        startup_lease_deadline_response_released: false,
        shutdown_descendants: Vec::new(),
        shutdown_tree_unavailable: None,
        shutdown_tree_snapshotted: false,
        capability_stream,
        shutdown_backstop_descendants: None,
        actor_tx: tx,
    };
    readiness.notify();
    Ok(daemon.serve())
}

/// The session's terminal, with kitty graphics on.
///
/// A session is the durable owner of the child's screen, and since libghostty
/// keeps image state per screen, that includes the child's images: without it
/// the `SCREEN` a late client replays would carry placeholder cells naming
/// images nobody has (docs/decisions/0012-kitty-graphics-replay.md). The
/// storage limit is a cap, not an allocation — a session whose child never
/// transmits an image holds nothing and serializes exactly as before.
fn terminal_actor(rows: u16, cols: u16) -> TerminalActor {
    let mut actor = TerminalActor::new(rows, cols, pty_terminal::actor::DEFAULT_SCROLLBACK);
    if !actor.enable_graphics(pty_terminal::GraphicsOptions::DEFAULT) {
        // The session still runs: text is unaffected and the actor rolled the
        // storage limit back, so the only loss is images. Say so rather than
        // leaving a client to wonder why its replay carries placeholder cells
        // and no pictures.
        daemon_warn!("pty: kitty graphics unavailable for this session");
    }
    actor
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

/// Will this `accept` failure pass on its own?
///
/// A signal, a peer that hung up before we reached it, or a machine with no
/// descriptors to spare: all of these end. Anything else says the listener
/// itself is broken and waiting will not mend it.
pub(crate) fn accept_failure_passes(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(
        e.kind(),
        Interrupted | ConnectionAborted | WouldBlock | TimedOut
    ) || e.raw_os_error() == Some(libc::EMFILE)
        || e.raw_os_error() == Some(libc::ENFILE)
}

/// Accept forever, and do not go deaf on a passing failure.
///
/// Leaving this loop is the worst thing it can do. The daemon keeps running,
/// the child keeps running, and the registry keeps saying the session is
/// running, but nothing can ever attach, peek or ask for stats again. A
/// descriptor shortage would do it, and a descriptor shortage ends. So a
/// failure that can pass is reported and retried, and only a listener that
/// cannot work at all shuts the session down, where somebody can see it.
fn spawn_acceptor(listener: UnixListener, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let ids = Arc::new(AtomicU64::new(1));
        let mut consecutive = 0u32;
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(stream) => {
                    consecutive = 0;
                    stream
                }
                Err(e) => {
                    if !accept_failure_passes(&e) {
                        daemon_warn!("pty daemon: the listener failed: {e}");
                        let _ = tx.send(Msg::ExternalKill);
                        break;
                    }
                    consecutive += 1;
                    if consecutive == 1 || consecutive % 100 == 0 {
                        daemon_warn!("pty daemon: accept failed ({consecutive}): {e}");
                    }
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
            };
            let id = ids.fetch_add(1, Ordering::Relaxed);
            spawn_client(id, stream, tx.clone());
        }
    });
}

fn spawn_client(id: u64, stream: UnixStream, tx: Sender<Msg>) {
    let peer = pty_core::unix_peer::credentials(&stream);
    let (out_tx, out_rx) = mpsc::channel::<Out>();
    let Ok(wstream) = stream.try_clone() else {
        return;
    };
    std::thread::spawn(move || {
        let mut wstream = wstream;
        while let Ok(out) = out_rx.recv() {
            match out {
                Out::Bytes(bytes) => {
                    if wstream.write_all(&bytes).is_err() {
                        break;
                    }
                }
                Out::BytesThenRelease(bytes, actor) => {
                    let _ = wstream.write_all(&bytes);
                    let _ = actor.send(Msg::StartupLeaseResponseReleased);
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
        let mut marker = [0u8; 1];
        let (prefix_len, capability_fd) = match capability::recv_fd(&stream, &mut marker) {
            Ok(result) => result,
            Err(_) => return,
        };
        if tx
            .send(Msg::Connect {
                id,
                tx: out_tx,
                peer,
                capability_fd,
            })
            .is_err()
        {
            return;
        }
        let mut parser = PacketReader::new();
        let mut buf = [0u8; 16384];
        if prefix_len > 0 && let Ok(packets) = parser.feed(&marker[..prefix_len]) {
            for packet in packets {
                if tx.send(Msg::Packet { id, packet }).is_err() {
                    return;
                }
            }
        }
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
                    Err(_) => break,
                },
                Err(error)
                    if error.kind() == std::io::ErrorKind::Interrupted
                        || error.kind() == std::io::ErrorKind::WouldBlock => {}
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
    let Ok(mut signals) = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
    ]) else {
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
    let Some(raw) = std::env::var("PTY_SPAWNER_PID")
        .ok()
        .filter(|r| !r.is_empty())
    else {
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
                self.activity_persist_at,
                self.startup_lease_timer,
                self.startup_lease_retry.map(|(next, _)| next),
            ]
            .into_iter()
            .flatten()
            .min();
            let msg = match deadline {
                Some(d) => match self.rx.recv_timeout(d.saturating_duration_since(now)) {
                    Ok(m) => Some(m),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break 0,
                },
                None => match self.rx.recv() {
                    Ok(m) => Some(m),
                    Err(_) => break 0,
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
            Msg::Connect {
                id,
                tx,
                peer,
                capability_fd,
            } => {
                self.clients.insert(
                    id,
                    Client::new(
                        tx,
                        self.actor.rows(),
                        self.actor.cols(),
                        peer,
                        false,
                    ),
                );
                if should_challenge_capability(
                    self.cfg.control_capability,
                    capability_fd.is_some(),
                    self.control_peer_identity_authorized(id),
                ) && self.echo_capability()
                    && let Some(client) = self.clients.get_mut(&id)
                {
                    client.capability_authorized = true;
                }
            }
            Msg::Packet { id, packet } => self.on_packet(id, packet),
            Msg::Closed { id } => self.on_closed(id),
            Msg::ExternalKill => {
                if self.shutdown_code.is_none() {
                    self.external_kill = true;
                    self.shutdown_code = Some(0);
                }
            }
            Msg::OwnershipResult { id, result } => {
                let result = if self.exited || self.startup_lease_deadline_notified {
                    AcceptedSocketOwnershipResult::Unavailable {
                        reason: "child-exited".to_string(),
                    }
                } else {
                    match registry::read_metadata(&self.name) {
                        None => AcceptedSocketOwnershipResult::Unavailable {
                            reason: "metadata-unavailable".to_string(),
                        },
                        Some(metadata)
                            if metadata.generation.as_deref() != Some(&self.generation) =>
                        {
                            AcceptedSocketOwnershipResult::Unavailable {
                                reason: "generation-mismatch".to_string(),
                            }
                        }
                        Some(_) => result,
                    }
                };
                if let Some(client) = self.clients.get(&id) {
                    client.send(encode_accepted_socket_ownership_response(&result));
                }
            }
            Msg::StartupLeaseResponseReleased => {
                self.startup_lease_deadline_response_released = true;
                self.notify_startup_lease_deadline();
            }
        }
    }

    /// Child output: into the terminal (queries answered back to the
    /// child), events to the log, the remainder to live clients.
    ///
    /// node: src/server.ts:559-569
    fn on_pty_data(&mut self, bytes: &[u8]) {
        self.stamp_output_activity();
        let cleaned = self.actor.write(bytes);
        let replies = self.actor.take_pty_replies();
        self.write_pty(&replies);
        self.forward_terminal_events();
        if !cleaned.is_empty() {
            self.broadcast(&encode_data(&cleaned));
        }
    }

    /// Record that the child has just printed, and make sure a write is
    /// pending. Both halves are O(1); the write itself happens later.
    fn stamp_output_activity(&mut self) {
        self.last_output_at_ms = Some(registry::now_epoch_ms());
        if self.activity_persist_at.is_none() {
            self.activity_persist_at = Some(Instant::now() + ACTIVITY_PERSIST_DEBOUNCE);
        }
    }

    /// Write the newest output stamp, if it is not the one already on disk.
    ///
    /// Best effort on purpose: a lost stamp reads as slightly older activity,
    /// and it must never take the daemon down or block the output path.
    fn persist_output_activity(&mut self) {
        self.activity_persist_at = None;
        if self.exited {
            return;
        }
        let Some(stamped) = self.last_output_at_ms else {
            return;
        };
        registry::mutate_metadata_under_lock(
            &self.name,
            move |m| {
                if m.last_output_at_ms == Some(stamped) {
                    return false;
                }
                m.last_output_at_ms = Some(stamped);
                true
            },
            &MutateOptions::default(),
        );
    }

    fn echo_capability(&mut self) -> bool {
        let Some(stream) = self.capability_stream.as_mut() else {
            return false;
        };
        let challenge = pty_core::registry::atomic::random_bytes(
            pty_core::client::readiness::CAPABILITY_CHALLENGE_LEN,
        );
        if stream.write_all(&challenge).is_err() {
            return false;
        }
        let mut response = vec![0u8; challenge.len()];
        stream.read_exact(&mut response).is_ok() && response == challenge
    }

    fn control_peer_identity_authorized(&self, id: u64) -> bool {
        let Some(client) = self.clients.get(&id) else {
            return false;
        };
        let Some(peer) = client.peer.as_ref() else {
            return false;
        };
        let Some(root) = self.child_identity.as_ref() else {
            return false;
        };
        let Some(daemon) = self.daemon_identity.as_ref() else {
            return false;
        };
        let Some(authority) = self.cfg.control_authority.as_ref() else {
            return false;
        };
        if client.role != Role::Command || peer.uid != pty_core::unix_peer::effective_uid() {
            return false;
        }
        let authority = ProcessIdentity {
            pid: authority.pid,
            identity: authority.process_start_token.clone(),
            depth: 0,
        };
        control_peer_is_authorized(
            root,
            daemon,
            &authority,
            peer.pid,
            &peer.process_start_token,
        )
    }

    pub(crate) fn control_peer_authorized(&self, id: u64) -> bool {
        let Some(client) = self.clients.get(&id) else {
            return false;
        };
        if (self.cfg.control_capability && !client.capability_authorized)
            || (!self.cfg.control_capability && self.cfg.control_authority.is_none())
        {
            return false;
        }
        self.control_peer_identity_authorized(id)
    }
    pub(crate) fn reject_unauthorized_ownership(&self, id: u64) {
        if let Some(client) = self.clients.get(&id) {
            client.send(encode_accepted_socket_ownership_response(
                &AcceptedSocketOwnershipResult::Unavailable {
                    reason: "unauthorized-peer".to_string(),
                },
            ));
        }
    }

    pub(crate) fn reject_unauthorized_lifecycle(&self, id: u64) {
        if let Some(client) = self.clients.get(&id) {
            client.send(encode_lifecycle_compare_and_set_response(
                &LifecycleCompareAndSetResult::InvalidRequest {
                    reason: "unauthorized peer".to_string(),
                },
            ));
        }
    }

    pub(crate) fn on_accepted_socket_ownership(&mut self, id: u64, payload: &[u8]) {
        let Some(request) = decode_accepted_socket_ownership_request(payload) else {
            if let Some(client) = self.clients.get(&id) {
                client.send(encode_accepted_socket_ownership_response(
                    &AcceptedSocketOwnershipResult::Unavailable {
                        reason: "invalid-request".to_string(),
                    },
                ));
            }
            return;
        };
        if request.expected_generation != self.generation {
            if let Some(client) = self.clients.get(&id) {
                client.send(encode_accepted_socket_ownership_response(
                    &AcceptedSocketOwnershipResult::Unavailable {
                        reason: "generation-mismatch".to_string(),
                    },
                ));
            }
            return;
        }
        if self.exited {
            if let Some(client) = self.clients.get(&id) {
                client.send(encode_accepted_socket_ownership_response(
                    &AcceptedSocketOwnershipResult::Unavailable {
                        reason: "child-exited".to_string(),
                    },
                ));
            }
            return;
        }
        let Some(metadata) = registry::read_metadata(&self.name) else {
            if let Some(client) = self.clients.get(&id) {
                client.send(encode_accepted_socket_ownership_response(
                    &AcceptedSocketOwnershipResult::Unavailable {
                        reason: "metadata-unavailable".to_string(),
                    },
                ));
            }
            return;
        };
        if metadata.generation.as_deref() != Some(&self.generation) {
            if let Some(client) = self.clients.get(&id) {
                client.send(encode_accepted_socket_ownership_response(
                    &AcceptedSocketOwnershipResult::Unavailable {
                        reason: "generation-mismatch".to_string(),
                    },
                ));
            }
            return;
        }
        let Some(child_identity) = self.child_identity.clone() else {
            self.reject_unauthorized_ownership(id);
            return;
        };
        let actor = self.actor_tx.clone();
        std::thread::spawn(move || {
            let result = super::ownership::inspect_accepted_socket_ownership(
                child_identity.pid,
                &child_identity.identity,
                &request.connection,
            );
            let _ = actor.send(Msg::OwnershipResult { id, result });
        });
    }

    pub(crate) fn on_lifecycle_cas(&mut self, id: u64, payload: &[u8]) {
        let result = match decode_lifecycle_compare_and_set_request(payload) {
            Some(request) => self.compare_and_set_lifecycle(&request),
            None => LifecycleCompareAndSetResult::InvalidRequest {
                reason: "invalid request payload".to_string(),
            },
        };
        let packet = encode_lifecycle_compare_and_set_response(&result);
        let defer_shutdown = self.startup_lease_terminal_cause.is_some_and(|cause| {
            cause != StartupLeaseTerminalCause::Exit && !self.startup_lease_deadline_notified
        });
        if let Some(client) = self.clients.get(&id) {
            if defer_shutdown {
                let _ = client
                    .tx
                    .send(Out::BytesThenRelease(packet, self.actor_tx.clone()));
                let actor = self.actor_tx.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(STARTUP_RESPONSE_BACKSTOP);
                    let _ = actor.send(Msg::StartupLeaseResponseReleased);
                });
            } else {
                client.send(packet);
            }
        }
    }

    fn compare_and_set_lifecycle(
        &mut self,
        request: &LifecycleCompareAndSetRequest,
    ) -> LifecycleCompareAndSetResult {
        if request.expected_generation != self.generation {
            return LifecycleCompareAndSetResult::GenerationMismatch;
        }
        let Some(current) = registry::read_metadata(&self.name) else {
            return LifecycleCompareAndSetResult::Missing;
        };
        if current.generation.as_deref() != Some(&self.generation) {
            return LifecycleCompareAndSetResult::GenerationMismatch;
        }
        if self.exited {
            let value = self
                .settle_startup_lease(StartupLeaseTerminalCause::Exit, true)
                .unwrap_or_else(|| {
                    terminal_startup_lease_value(&self.generation, StartupLeaseTerminalCause::Exit)
                });
            return match registry::read_metadata(&self.name) {
                None => LifecycleCompareAndSetResult::Missing,
                Some(metadata) if metadata.generation.as_deref() != Some(&self.generation) => {
                    LifecycleCompareAndSetResult::GenerationMismatch
                }
                Some(_) => LifecycleCompareAndSetResult::Terminal { value },
            };
        }
        let expired = self.startup_lease.as_ref().is_some_and(|lease| {
            !self.startup_lease_disarmed
                && monotonic_now_ns().is_none_or(|now| now >= lease.deadline_monotonic_ns)
        });
        if expired {
            self.settle_startup_lease_deadline(false);
            let Some(after) = registry::read_metadata(&self.name) else {
                return LifecycleCompareAndSetResult::Missing;
            };
            if after.generation.as_deref() != Some(&self.generation) {
                return LifecycleCompareAndSetResult::GenerationMismatch;
            }
            let value = self
                .startup_lease
                .as_ref()
                .and_then(|lease| after.tags.as_ref()?.get(&lease.lifecycle_tag))
                .cloned();
            return match value {
                Some(value) => LifecycleCompareAndSetResult::DeadlineExpired { value },
                None => LifecycleCompareAndSetResult::ValueMismatch { value: None },
            };
        }

        let result = registry::compare_and_set_tag_value(
            &self.name,
            &request.expected_generation,
            &request.tag,
            &request.expected_value,
            &request.value,
        );
        let changed = matches!(&result, TagCompareAndSetResult::Changed { .. });
        match result {
            TagCompareAndSetResult::Changed { value }
            | TagCompareAndSetResult::Unchanged { value } => {
                if let Some(lease) = &self.startup_lease {
                    if request.tag == lease.lifecycle_tag
                        && request.expected_value == lease.starting_value
                        && request.value != lease.starting_value
                    {
                        self.startup_lease_disarmed = true;
                        self.startup_lease_timer = None;
                        self.startup_lease_retry = None;
                    }
                    if request.tag == lease.lifecycle_tag
                        && serde_json::from_str::<serde_json::Value>(&request.value)
                            .ok()
                            .is_some_and(|lifecycle| {
                                lifecycle.get("_tag").and_then(|tag| tag.as_str())
                                    == Some("terminal")
                                    && lifecycle
                                        .get("generation")
                                        .and_then(|generation| generation.as_str())
                                        == Some(self.generation.as_str())
                            })
                    {
                        self.startup_lease_terminal_value = Some(request.value.clone());
                    }
                }
                if changed {
                    LifecycleCompareAndSetResult::Changed { value }
                } else {
                    LifecycleCompareAndSetResult::Unchanged { value }
                }
            }
            TagCompareAndSetResult::ValueMismatch { value } => {
                LifecycleCompareAndSetResult::ValueMismatch { value }
            }
            TagCompareAndSetResult::Missing => LifecycleCompareAndSetResult::Missing,
            TagCompareAndSetResult::GenerationMismatch => {
                LifecycleCompareAndSetResult::GenerationMismatch
            }
            TagCompareAndSetResult::Busy | TagCompareAndSetResult::Stale => {
                LifecycleCompareAndSetResult::Busy
            }
        }
    }

    fn settle_startup_lease_deadline(&mut self, notify: bool) -> Option<String> {
        if !self.shutdown_tree_snapshotted {
            let snapshot = self.child_identity.as_ref().map_or_else(
                || CompleteProcessTreeSnapshot::Unavailable {
                    reason: "root-identity-unavailable".to_string(),
                    identities: Vec::new(),
                },
                freeze_descendant_processes,
            );
            self.shutdown_tree_snapshotted = true;
            self.shutdown_descendants = snapshot.identities().to_vec();
            if let CompleteProcessTreeSnapshot::Unavailable { reason, .. } = snapshot {
                self.shutdown_tree_unavailable = Some(reason.clone());
                daemon_warn!(
                    "pty daemon \"{}\": complete child containment unavailable: {reason}; publishing teardown-unavailable and exact-signalling {} observed descendant(s) plus process-group fallback",
                    self.name,
                    self.shutdown_descendants.len()
                );
            }
        }
        // Persistence can be held busy by a lock owner that the freeze just
        // stopped. Arm the independent hard deadline before attempting it.
        self.arm_backstop(124);
        self.settle_startup_lease(
            startup_lease_deadline_cause(self.shutdown_tree_unavailable.is_none()),
            notify,
        )
    }

    fn settle_startup_lease(
        &mut self,
        cause: StartupLeaseTerminalCause,
        notify: bool,
    ) -> Option<String> {
        let effective_cause = if cause == StartupLeaseTerminalCause::Exit {
            self.startup_lease_terminal_cause.unwrap_or(cause)
        } else {
            cause
        };
        self.startup_lease_terminal_cause = Some(effective_cause);
        let lease = self.startup_lease.as_ref()?;
        if let Some(value) = &self.startup_lease_terminal_value {
            return Some(value.clone());
        }
        let terminal = terminal_startup_lease_value(&self.generation, effective_cause);
        let tag = lease.lifecycle_tag.clone();
        let expected_generation = self.generation.clone();
        let terminal_for_write = terminal.clone();
        let result = registry::mutate_metadata_under_lock(
            &self.name,
            move |metadata| {
                if metadata.tags.as_ref().and_then(|tags| tags.get(&tag))
                    == Some(&terminal_for_write)
                {
                    return false;
                }
                metadata
                    .tags
                    .get_or_insert_with(TagMap::new)
                    .insert(tag, terminal_for_write);
                true
            },
            &MutateOptions {
                expected_generation: Some(expected_generation),
                expected_metadata: None,
            },
        );
        match result {
            MutateStatus::Changed(_) | MutateStatus::Unchanged(_) => {
                self.startup_lease_disarmed = true;
                self.startup_lease_timer = None;
                self.startup_lease_retry = None;
                self.startup_lease_terminal_value = Some(terminal.clone());
                if effective_cause != StartupLeaseTerminalCause::Exit {
                    self.startup_lease_deadline_notification_pending = true;
                    if notify || self.startup_lease_deadline_response_released {
                        self.notify_startup_lease_deadline();
                    }
                }
            }
            MutateStatus::Busy | MutateStatus::Stale | MutateStatus::Missing => {
                self.startup_lease_timer = None;
                self.startup_lease_retry = Some((Instant::now() + STARTUP_LEASE_RETRY, notify));
            }
            MutateStatus::GenerationMismatch => {
                // A replacement owns the registry, so fence all further writes
                // and cleanup against it. A deadline still owns this daemon's
                // already-frozen exact child tree and must finish terminating it.
                self.startup_lease_disarmed = true;
                self.startup_lease_timer = None;
                self.startup_lease_retry = None;
                self.startup_lease_terminal_value = Some(terminal.clone());
                if effective_cause != StartupLeaseTerminalCause::Exit {
                    self.startup_lease_deadline_notification_pending = true;
                    if notify || self.startup_lease_deadline_response_released {
                        self.notify_startup_lease_deadline();
                    }
                }
            }
        }
        Some(terminal)
    }

    /// A daemon-initiated close must not remove the socket or signal the
    /// child while its startup tag still advertises `starting`. The launcher
    /// may briefly own the creation lock while releasing daemon readiness, so
    /// retry without a timeout until this generation is terminal or no longer
    /// owns the stable id.
    fn settle_startup_lease_before_close(&mut self) {
        if self.startup_lease.is_none() || self.startup_lease_terminal_value.is_some() {
            return;
        }
        loop {
            self.settle_startup_lease(StartupLeaseTerminalCause::Exit, true);
            if self.startup_lease_terminal_value.is_some() {
                return;
            }
            match registry::read_metadata(&self.name) {
                None => return,
                Some(metadata)
                    if metadata.generation.as_deref() != Some(self.generation.as_str()) =>
                {
                    return;
                }
                Some(_) => std::thread::sleep(STARTUP_LEASE_RETRY),
            }
        }
    }

    fn notify_startup_lease_deadline(&mut self) {
        if !self.startup_lease_deadline_notification_pending || self.startup_lease_deadline_notified
        {
            return;
        }
        self.startup_lease_deadline_notification_pending = false;
        self.startup_lease_deadline_notified = true;
        self.external_kill = true;
        if self.shutdown_code.is_none() {
            self.shutdown_code = Some(124);
        }
    }

    fn service_timers(&mut self, now: Instant) {
        self.service_cuts(now);
        if let Some(at) = self.activity_persist_at
            && at <= now
        {
            self.persist_output_activity();
        }
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
        if let Some(at) = self.startup_lease_timer
            && at <= now
        {
            if self.startup_lease_disarmed {
                self.startup_lease_timer = None;
            } else if let Some(lease) = &self.startup_lease {
                let remaining = remaining_lease_delay(lease.deadline_monotonic_ns);
                if remaining.is_zero() {
                    self.startup_lease_timer = None;
                    self.settle_startup_lease_deadline(true);
                } else {
                    self.startup_lease_timer = Some(now + remaining);
                }
            }
        }
        if let Some((at, notify)) = self.startup_lease_retry
            && at <= now
            && let Some(cause) = self.startup_lease_terminal_cause
        {
            self.startup_lease_retry = None;
            self.settle_startup_lease(cause, notify);
        }
        if let Some(at) = self.exit_shutdown_at
            && at <= now
        {
            if self.startup_lease.is_some() && self.startup_lease_terminal_value.is_none() {
                self.exit_shutdown_at = Some(now + STARTUP_LEASE_RETRY);
            } else {
                self.exit_shutdown_at = None;
                if self.shutdown_code.is_none() {
                    self.shutdown_code = Some(self.exit_code);
                }
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
        self.arm_backstop(code);
        self.settle_startup_lease(StartupLeaseTerminalCause::Exit, true);
        self.broadcast(&encode_exit(code));
        self.events
            .append(Event::session_exit(&self.name, code, signal));
        if matches!(
            self.save_exit_metadata(),
            MutateStatus::Busy | MutateStatus::Stale
        ) {
            let now = Instant::now();
            self.exit_meta_retry =
                Some((now + Duration::from_millis(10), now + EXIT_METADATA_RETRY));
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
        // Carry the newest output stamp even when its own write was still
        // waiting out the debounce, so the last thing the child printed is
        // never lost to the exit.
        let last_output = self.last_output_at_ms;
        registry::mutate_metadata_under_lock(
            &self.name,
            move |m| {
                m.exit_code = Some(code);
                m.exited_at = Some(registry::now_iso8601());
                m.last_lines = Some(last_lines);
                if last_output.is_some() {
                    m.last_output_at_ms = last_output;
                }
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
        if self.cfg.startup_lease.is_some() {
            return false;
        }
        reap_at_exit(
            &self.name,
            &self.generation,
            self.external_kill,
            self.cfg.ephemeral,
            self.cfg.tags(),
        )
    }

    fn arm_backstop(&mut self, code: i32) -> Arc<Mutex<Vec<ProcessIdentity>>> {
        if let Some(descendants) = &self.shutdown_backstop_descendants {
            if let Ok(mut shared) = descendants.lock() {
                shared.clone_from(&self.shutdown_descendants);
            }
            return descendants.clone();
        }
        let descendants = Arc::new(Mutex::new(self.shutdown_descendants.clone()));
        self.start_backstop(code, descendants.clone());
        self.shutdown_backstop_descendants = Some(descendants.clone());
        descendants
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
        let startup_lease = self.cfg.startup_lease.is_some();
        let child_identity = self.child_identity.clone();
        let owner = self.owner();
        std::thread::spawn(move || {
            std::thread::sleep(deadline);
            crate::daemon::daemon_warn!(
                "pty daemon \"{name}\": graceful shutdown exceeded {}ms — forcing exit (child reaped)",
                deadline.as_millis()
            );
            signal_child(child_identity.as_ref(), libc::SIGKILL);
            let descendants = descendants.lock().map(|d| d.clone()).unwrap_or_default();
            signal_process_identities(&descendants, libc::SIGKILL);
            if !startup_lease
                && reap_at_exit(&name, &generation, external, ephemeral, tags.as_ref())
            {
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
        // Arm first: startup-lease settlement takes the creation lock and is
        // deliberately unbounded, so it must not consume the hard deadline.
        let descendants = self.arm_backstop(code);
        if self.external_kill && !self.shutdown_tree_snapshotted {
            let snapshot = self.child_identity.as_ref().map_or_else(
                || CompleteProcessTreeSnapshot::Unavailable {
                    reason: "root-identity-unavailable".to_string(),
                    identities: Vec::new(),
                },
                snapshot_descendant_processes_complete_for,
            );
            self.shutdown_tree_snapshotted = true;
            self.shutdown_descendants = snapshot.identities().to_vec();
            if let Ok(mut shared) = descendants.lock() {
                shared.clone_from(&self.shutdown_descendants);
            }
            if let CompleteProcessTreeSnapshot::Unavailable { reason, .. } = snapshot {
                self.shutdown_tree_unavailable = Some(reason.clone());
                daemon_warn!(
                    "pty daemon \"{}\": complete child snapshot unavailable: {reason}; exact-signalling {} observed descendant(s) plus process-group fallback",
                    self.name,
                    self.shutdown_descendants.len()
                );
            }
        }
        self.settle_startup_lease_before_close();
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
            signal_child(
                self.child_identity.as_ref(),
                if self.startup_lease_deadline_notified {
                    libc::SIGKILL
                } else {
                    libc::SIGHUP
                },
            );
        }
        let term_wait = if self.startup_lease_deadline_notified {
            Duration::ZERO
        } else {
            TERM_WAIT
        };
        let descendant_wait = self.external_kill.then(|| {
            let ids = descendants.lock().map(|d| d.clone()).unwrap_or_default();
            std::thread::spawn(move || terminate_process_identities(&ids, term_wait, KILL_WAIT))
        });
        let needs_group_fallback = self.external_kill && self.shutdown_tree_unavailable.is_some();
        let group_wait = needs_group_fallback
            .then(|| self.child_identity.clone())
            .flatten()
            .map(|root| {
                std::thread::spawn(move || terminate_process_group(&root, term_wait, KILL_WAIT))
            });
        if !self.wait_child_exit(CHILD_HUP_WAIT) {
            signal_child(self.child_identity.as_ref(), libc::SIGKILL);
            self.wait_child_exit(CHILD_KILL_WAIT);
        }
        let survivors = descendant_wait
            .and_then(|t| t.join().ok())
            .unwrap_or_default();
        let process_group_gone = if needs_group_fallback {
            group_wait
                .and_then(|thread| thread.join().ok())
                .unwrap_or(false)
        } else {
            true
        };
        if !process_group_gone {
            daemon_warn!(
                "pty daemon \"{}\": process-group fallback could not verify teardown after incomplete snapshot ({})",
                self.name,
                self.shutdown_tree_unavailable
                    .as_deref()
                    .unwrap_or("unknown")
            );
        }
        if !survivors.is_empty() {
            let pids: Vec<i32> = survivors.iter().map(|s| s.pid).collect();
            crate::daemon::daemon_warn!(
                "pty daemon \"{}\": {} child process(es) did not exit after exact TERM and KILL signals: {pids:?}",
                self.name,
                survivors.len()
            );
            // And somewhere a person can find it. The warning above goes to
            // this daemon's standard error, which has had no reader since
            // the command that launched it stopped listening — so the one
            // moment it has something worth saying is the one moment nobody
            // is there. `pty kill` reports on the daemon and cannot see a
            // surviving child, so without this line the fact reaches no one.
            self.events
                .append(Event::session_descendants_survived(&self.name, &pids));
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

    #[test]
    fn readiness_capability_descriptor_is_cloexec_before_child_spawn() {
        let (stream, _peer) = UnixStream::pair().expect("socketpair");
        let fd = stream.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert!(flags >= 0);
        assert_eq!(unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) }, 0);
        assert!(set_fd_cloexec(fd));
        let final_flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert_ne!(final_flags & libc::FD_CLOEXEC, 0);
    }

    #[test]
    fn unauthorized_capability_peers_are_not_challenged() {
        assert!(!should_challenge_capability(true, true, false));
        assert!(!should_challenge_capability(true, false, true));
        assert!(should_challenge_capability(true, true, true));
    }
}

#[cfg(test)]
mod accept_failure_tests {
    use super::accept_failure_passes;
    use std::io::{Error, ErrorKind};

    /// The old loop left on ANY error, which took the daemon deaf while it
    /// went on reporting itself as running. These are the failures that end
    /// on their own, so leaving on them is the wrong answer.
    ///
    /// This covers the decision, not the loop. Nothing here proves the
    /// acceptor keeps serving after a real descriptor shortage; that needs a
    /// daemon under a lowered descriptor limit and is not written.
    #[test]
    fn failures_that_pass_are_not_fatal() {
        for kind in [
            ErrorKind::Interrupted,
            ErrorKind::ConnectionAborted,
            ErrorKind::WouldBlock,
            ErrorKind::TimedOut,
        ] {
            assert!(
                accept_failure_passes(&Error::new(kind, "x")),
                "{kind:?} should not end the acceptor"
            );
        }
        for errno in [libc::EMFILE, libc::ENFILE] {
            assert!(
                accept_failure_passes(&Error::from_raw_os_error(errno)),
                "errno {errno} should not end the acceptor"
            );
        }
    }

    /// A listener that cannot work is a different answer, and the daemon
    /// shuts down rather than sitting deaf while the registry says running.
    #[test]
    fn a_broken_listener_is_fatal() {
        for errno in [libc::EBADF, libc::EINVAL, libc::ENOTSOCK] {
            assert!(
                !accept_failure_passes(&Error::from_raw_os_error(errno)),
                "errno {errno} should end the acceptor"
            );
        }
    }
}
