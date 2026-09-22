//! Typed two-plane PTY substrate primitives.
//!
//! [`SessionRef`] is shareable identity only. A [`SessionOwner`] is the sole
//! reader of a generation's PTY master and the sole reaper of its child. Each
//! [`SessionClient`] is an equal attachment: it can submit the same typed data
//! and lifecycle operations, but it never receives the master or child handle.
//! The actor serializes every operation and fences attachments by generation.

use std::fmt;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use portable_pty::{Child, ExitStatus as PortableExitStatus, MasterPty, PtyPair, PtySize};

/// Immutable identity for one exact PTY generation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionRef {
    root: PathBuf,
    id: String,
    generation: String,
}

impl SessionRef {
    /// Creates an identity. The generation must be non-empty and opaque.
    pub fn new(root: impl Into<PathBuf>, id: impl Into<String>, generation: impl Into<String>) -> Self {
        Self { root: root.into(), id: id.into(), generation: generation.into() }
    }

    pub fn root(&self) -> &Path { &self.root }
    pub fn id(&self) -> &str { &self.id }
    pub fn generation(&self) -> &str { &self.generation }
    pub fn socket_path(&self) -> PathBuf { self.root.join(format!("{}.sock", self.id)) }
}

/// The only lifecycle outcomes a client can observe from the substrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

impl ExitStatus {
    fn from_portable(status: PortableExitStatus) -> Self {
        if status.success() {
            return Self { code: Some(0), signal: None };
        }
        let signal = status.signal().and_then(signal_number);
        Self {
            code: signal.is_none().then_some(status.exit_code() as i32),
            signal,
        }
    }

    #[cfg(unix)]
    fn from_wait_status(status: i32) -> Self {
        if libc::WIFSIGNALED(status) {
            Self { code: None, signal: Some(libc::WTERMSIG(status)) }
        } else if libc::WIFEXITED(status) {
            Self { code: Some(libc::WEXITSTATUS(status)), signal: None }
        } else {
            Self { code: None, signal: None }
        }
    }
}

fn signal_number(name: &str) -> Option<i32> {
    let name = name.strip_prefix("SIG").unwrap_or(name);
    Some(match name {
        "ABRT" | "Aborted" => libc::SIGABRT,
        "ALRM" | "Alarm clock" => libc::SIGALRM,
        "BUS" | "Bus error" => libc::SIGBUS,
        "CHLD" => libc::SIGCHLD,
        "CONT" => libc::SIGCONT,
        "FPE" | "Floating point exception" => libc::SIGFPE,
        "HUP" | "Hangup" => libc::SIGHUP,
        "ILL" | "Illegal instruction" => libc::SIGILL,
        "INT" | "Interrupt" => libc::SIGINT,
        "KILL" | "Killed" => libc::SIGKILL,
        "PIPE" | "Broken pipe" => libc::SIGPIPE,
        "QUIT" | "Quit" => libc::SIGQUIT,
        "SEGV" | "Segmentation fault" => libc::SIGSEGV,
        "STOP" | "Stopped" => libc::SIGSTOP,
        "TERM" | "Terminated" => libc::SIGTERM,
        "TRAP" | "Trace/breakpoint trap" => libc::SIGTRAP,
        "TSTP" | "Stopped (signal)" => libc::SIGTSTP,
        "TTIN" => libc::SIGTTIN,
        "TTOU" => libc::SIGTTOU,
        "USR1" | "User defined signal 1" => libc::SIGUSR1,
        "USR2" | "User defined signal 2" => libc::SIGUSR2,
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Running,
    Exited(ExitStatus),
    /// The owner actor disappeared before an authoritative child result.
    OwnerLost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    Data(Vec<u8>),
    Lifecycle(Lifecycle),
    Geometry(PtySize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    StaleGeneration { expected: String, actual: String },
    OwnerLost,
    Exited(ExitStatus),
    Closed,
    Io(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleGeneration { expected, actual } => write!(f, "stale PTY generation (expected {expected}, current {actual})"),
            Self::OwnerLost => f.write_str("PTY owner was lost"),
            Self::Exited(status) => write!(f, "PTY child exited ({status:?})"),
            Self::Closed => f.write_str("PTY session is closed"),
            Self::Io(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SessionError {}

/// The read side of one attachment. It contains no PTY descriptor and cannot
/// become a second master reader.
pub struct AttachStream {
    rx: Receiver<SessionEvent>,
}

impl AttachStream {
    pub fn recv(&self) -> Result<SessionEvent, SessionError> {
        self.rx.recv().map_err(|_| SessionError::Closed)
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<SessionEvent, SessionError> {
        match self.rx.recv_timeout(timeout) {
            Ok(event) => Ok(event),
            Err(RecvTimeoutError::Timeout) => Err(SessionError::Io("attachment event timed out".into())),
            Err(RecvTimeoutError::Disconnected) => Err(SessionError::Closed),
        }
    }
}

/// A per-client attachment. Cloning it does not create lifecycle authority or
/// another reader; all commands still pass through the owner actor.
#[derive(Clone)]
pub struct SessionClient {
    session: SessionRef,
    tx: Sender<Command>,
}

impl SessionClient {
    pub fn session(&self) -> &SessionRef { &self.session }

    pub fn input(&self, bytes: impl Into<Vec<u8>>) -> Result<(), SessionError> {
        self.request(CommandKind::Input(bytes.into()))
    }

    pub fn resize(&self, size: PtySize) -> Result<(), SessionError> {
        self.request(CommandKind::Resize(size))
    }

    pub fn terminate(&self) -> Result<(), SessionError> {
        self.request(CommandKind::Terminate)
    }

    /// Ask the owner actor to send the daemon's traditional hangup.
    pub fn hangup(&self) -> Result<(), SessionError> {
        self.request(CommandKind::Hangup)
    }

    pub fn lifecycle(&self) -> Result<Lifecycle, SessionError> {
        let (reply, rx) = mpsc::channel();
        self.tx.send(Command::Query { session: self.session.clone(), reply }).map_err(|_| SessionError::Closed)?;
        rx.recv().map_err(|_| SessionError::Closed)?
    }

    fn request(&self, kind: CommandKind) -> Result<(), SessionError> {
        let (reply, rx) = reply();
        self.tx.send(Command::Request { session: self.session.clone(), kind, reply }).map_err(|_| SessionError::Closed)?;
        rx.recv().map_err(|_| SessionError::Closed)?
    }
}

/// The sole owner/serializer for one externally-owned PTY generation.
pub struct SessionOwner {
    inner: Arc<OwnerInner>,
}

impl Clone for SessionOwner {
    fn clone(&self) -> Self { Self { inner: Arc::clone(&self.inner) } }
}

struct OwnerInner {
    tx: Sender<Command>,
}

impl Drop for OwnerInner {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::OwnerLost);
    }
}

/// A PTY master and child already created by an external consumer. Ownership
/// moves into the substrate; no raw descriptor is exposed or duplicated.
pub fn external_owned(
    session: SessionRef,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
) -> io::Result<SessionOwner> {
    start_owner(session, master, child)
}

/// Convenience constructor for callers that already opened a pair and spawned
/// its child on the slave. Dropping the slave here is part of the ownership
/// transfer, ensuring this owner is the only component that reads the master.
pub fn external_owned_pair(
    session: SessionRef,
    pair: PtyPair,
    child: Box<dyn Child + Send + Sync>,
) -> io::Result<SessionOwner> {
    drop(pair.slave);
    start_owner(session, pair.master, child)
}

fn start_owner(
    session: SessionRef,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
) -> io::Result<SessionOwner> {
    let reader = master.try_clone_reader().map_err(io::Error::other)?;
    let writer = master.take_writer().map_err(io::Error::other)?;
    let child_pid = child.process_id();
    let (tx, rx) = mpsc::channel();
    let reader_tx = tx.clone();
    thread::Builder::new().name("pty-substrate-reader".into()).spawn(move || read_master(reader, reader_tx)).map_err(io::Error::other)?;
    let reaper_tx = tx.clone();
    thread::Builder::new().name("pty-substrate-reaper".into()).spawn(move || reap_child(child, reaper_tx)).map_err(io::Error::other)?;
    let actor_tx = tx.clone();
    let actor_session = session.clone();
    thread::Builder::new().name("pty-substrate-owner".into()).spawn(move || actor(actor_session, master, writer, child_pid, rx)).map_err(io::Error::other)?;
    drop(actor_tx);
    Ok(SessionOwner { inner: Arc::new(OwnerInner { tx }) })
}

fn reply() -> (Sender<Result<(), SessionError>>, Receiver<Result<(), SessionError>>) { mpsc::channel() }

enum Command {
    Request { session: SessionRef, kind: CommandKind, reply: Sender<Result<(), SessionError>> },
    Query { session: SessionRef, reply: Sender<Result<Lifecycle, SessionError>> },
    Attach { session: SessionRef, events: Sender<SessionEvent>, reply: Sender<Result<(), SessionError>> },
    Output(Vec<u8>),
    Reaped(ExitStatus),
    ReaderClosed,
    OwnerLost,
}

enum CommandKind { Input(Vec<u8>), Resize(PtySize), Terminate, Hangup }

impl SessionOwner {
    /// Attach an equal client and its independent event stream.
    pub fn attach(&self, session: &SessionRef) -> Result<(SessionClient, AttachStream), SessionError> {
        let (events, rx) = mpsc::channel();
        let (reply, result) = reply();
        self.inner.tx.send(Command::Attach { session: session.clone(), events, reply }).map_err(|_| SessionError::Closed)?;
        result.recv().map_err(|_| SessionError::Closed)??;
        Ok((SessionClient { session: session.clone(), tx: self.inner.tx.clone() }, AttachStream { rx }))
    }

    /// Explicitly revoke this generation without granting authority to a client.
    pub fn lose_ownership(&self) { let _ = self.inner.tx.send(Command::OwnerLost); }
}

fn check(session: &SessionRef, current: &SessionRef) -> Result<(), SessionError> {
    if session != current {
        return Err(SessionError::StaleGeneration { expected: session.generation.clone(), actual: current.generation.clone() });
    }
    Ok(())
}

fn actor(
    session: SessionRef,
    master: Box<dyn MasterPty + Send>,
    mut writer: Box<dyn Write + Send>,
    child_pid: Option<u32>,
    rx: Receiver<Command>,
) {
    let mut lifecycle = Lifecycle::Running;
    let mut attachments: Vec<Sender<SessionEvent>> = Vec::new();
    while let Ok(command) = rx.recv() {
        match command {
            Command::Attach { session: requested, events, reply } => {
                let result = check(&requested, &session).and_then(|_| {
                    let _ = events.send(SessionEvent::Lifecycle(lifecycle));
                    attachments.push(events);
                    Ok(())
                });
                let _ = reply.send(result);
            }
            Command::Request { session: requested, kind, reply } => {
                let result = check(&requested, &session).and_then(|_| match lifecycle {
                    Lifecycle::Running => match kind {
                        CommandKind::Input(bytes) => writer.write_all(&bytes).and_then(|_| writer.flush()).map_err(|e| SessionError::Io(e.to_string())),
                        CommandKind::Resize(size) => master.resize(size).map_err(|e| SessionError::Io(e.to_string())),
                        CommandKind::Terminate => kill_child(child_pid, libc::SIGTERM).map_err(|e| SessionError::Io(e.to_string())),
                        CommandKind::Hangup => kill_child(child_pid, libc::SIGHUP).map_err(|e| SessionError::Io(e.to_string())),
                    },
                    Lifecycle::Exited(status) => Err(SessionError::Exited(status)),
                    Lifecycle::OwnerLost => Err(SessionError::OwnerLost),
                });
                let _ = reply.send(result);
            }
            Command::Query { session: requested, reply } => {
                let result = check(&requested, &session).map(|_| lifecycle);
                let _ = reply.send(result);
            }
            Command::Output(bytes) if matches!(lifecycle, Lifecycle::Running) => broadcast(&mut attachments, SessionEvent::Data(bytes)),
            Command::Reaped(status) if matches!(lifecycle, Lifecycle::Running) => {
                lifecycle = Lifecycle::Exited(status);
                broadcast(&mut attachments, SessionEvent::Lifecycle(lifecycle));
            }
            Command::ReaderClosed => {}
            Command::OwnerLost => {
                if matches!(lifecycle, Lifecycle::Running) {
                    lifecycle = Lifecycle::OwnerLost;
                    broadcast(&mut attachments, SessionEvent::Lifecycle(lifecycle));
                    let _ = kill_child(child_pid, libc::SIGTERM);
                }
            }
            _ => {}
        }
    }
}

fn broadcast(attachments: &mut Vec<Sender<SessionEvent>>, event: SessionEvent) {
    attachments.retain(|sender| sender.send(event.clone()).is_ok());
}

fn read_master(mut reader: Box<dyn Read + Send>, tx: Sender<Command>) {
    let mut buf = [0_u8; 16 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) if tx.send(Command::Output(buf[..n].to_vec())).is_err() => return,
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let _ = tx.send(Command::ReaderClosed);
}
fn kill_child(pid: Option<u32>, signal: libc::c_int) -> io::Result<()> {
    let Some(pid) = pid else { return Ok(()) };
    // SAFETY: the pid came from the child owned by this session.
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn reap_child(mut child: Box<dyn Child + Send + Sync>, tx: Sender<Command>) {
    #[cfg(unix)]
    let status = child
        .process_id()
        .and_then(wait_unix)
        .or_else(|| child.wait().ok().map(ExitStatus::from_portable))
        .unwrap_or(ExitStatus { code: None, signal: None });
    #[cfg(not(unix))]
    let status = child
        .wait()
        .map(ExitStatus::from_portable)
        .unwrap_or(ExitStatus { code: None, signal: None });
    let _ = tx.send(Command::Reaped(status));
}

#[cfg(unix)]
fn wait_unix(pid: u32) -> Option<ExitStatus> {
    let mut raw = 0;
    loop {
        // SAFETY: pid came from the child owned by this substrate actor.
        let result = unsafe { libc::waitpid(pid as libc::pid_t, &mut raw, 0) };
        if result == pid as libc::pid_t {
            return Some(ExitStatus::from_wait_status(raw));
        }
        if result < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return None;
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::{open, shell_exec};
    use std::time::Duration;

    fn owner(id: &str, command: &str) -> (SessionRef, SessionOwner) {
        let pair = open(24, 80).unwrap();
        let child = pair.slave.spawn_command(shell_exec(command, &[])).unwrap();
        let session = SessionRef::new("/private", id, "generation-a");
        (session.clone(), external_owned_pair(session, pair, child).unwrap())
    }

    #[test]
    fn equal_clients_receive_the_same_typed_output() {
        let (session, owner) = owner("equal", "cat");
        let (a, a_stream) = owner.attach(&session).unwrap();
        let (b, b_stream) = owner.attach(&session).unwrap();
        assert_eq!(a.session(), b.session());
        assert_eq!(a_stream.recv_timeout(Duration::from_secs(2)).unwrap(), SessionEvent::Lifecycle(Lifecycle::Running));
        assert_eq!(b_stream.recv_timeout(Duration::from_secs(2)).unwrap(), SessionEvent::Lifecycle(Lifecycle::Running));
        a.input(b"equal\n".to_vec()).unwrap();
        let a_event = a_stream.recv_timeout(Duration::from_secs(2)).unwrap();
        let b_event = b_stream.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(a_event, b_event);
        assert!(matches!(a_event, SessionEvent::Data(bytes) if !bytes.is_empty()));
    }

    #[test]
    fn one_reaper_publishes_the_child_exit() {
        let (session, owner) = owner("reaper", "true");
        let (_, stream) = owner.attach(&session).unwrap();
        assert_eq!(stream.recv_timeout(Duration::from_secs(2)).unwrap(), SessionEvent::Lifecycle(Lifecycle::Running));
        assert!(matches!(
            stream.recv_timeout(Duration::from_secs(2)).unwrap(),
            SessionEvent::Lifecycle(Lifecycle::Exited(ExitStatus { code: Some(0), signal: None }))
        ));
    }

    #[test]
    fn generation_is_fenced_before_attach() {
        let (session, owner) = owner("fence", "true");
        let stale = SessionRef::new("/private", "fence", "generation-b");
        assert!(matches!(owner.attach(&stale), Err(SessionError::StaleGeneration { .. })));
        let _ = owner.attach(&session).unwrap();
    }

    #[test]
    fn owner_loss_invalidates_all_clients() {
        let (session, owner) = owner("loss", "sleep");
        let (client, stream) = owner.attach(&session).unwrap();
        owner.lose_ownership();
        assert_eq!(stream.recv_timeout(Duration::from_secs(2)).unwrap(), SessionEvent::Lifecycle(Lifecycle::Running));
        assert_eq!(stream.recv_timeout(Duration::from_secs(2)).unwrap(), SessionEvent::Lifecycle(Lifecycle::OwnerLost));
        assert_eq!(client.input(Vec::new()), Err(SessionError::OwnerLost));
    }

    #[test]
    fn a_new_generation_does_not_adopt_an_old_owner() {
        let (old, owner) = owner("restart", "sleep");
        let stale = SessionRef::new("/private", "restart", "generation-b");
        assert!(matches!(owner.attach(&stale), Err(SessionError::StaleGeneration { .. })));
        drop(owner);
        assert_eq!(old.generation(), "generation-a");
    }
}
