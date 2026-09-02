//! Spawning a session daemon and waiting for it to publish, the way Node's
//! `spawnDaemon` does: one detached child (`<self> __daemon`), the config as
//! JSON on inherited fd 3 (never argv), stderr collected for the early-exit
//! message, then the socket, then `daemonPid == child.pid` plus a
//! `session_start` line stamped at or after `createdAt`.
//!
//! node: src/spawn.ts:129-260

use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use pty_core::registry::{self, EnvMap, TagMap};

use super::config::DaemonConfig;

/// `DEFAULT_START_TIMEOUT_MS`.
pub const DEFAULT_START_TIMEOUT: Duration = Duration::from_secs(30);

/// Node's `SpawnDaemonOptions`, minus the launcher/server-module knobs.
/// `command` is already resolved absolute (`pty_core::spawn::resolve_command`).
#[derive(Debug, Clone, Default)]
pub struct SpawnParams {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub display_command: String,
    pub cwd: String,
    pub rows: u16,
    pub cols: u16,
    pub ephemeral: bool,
    /// Written to the config only when non-empty.
    pub tags: TagMap,
    pub display_name: Option<String>,
    pub isolate_env: bool,
    /// Written only when non-empty.
    pub extra_env: EnvMap,
    /// Written only when non-empty.
    pub unset_env: Vec<String>,
    /// The verbatim replacement environment; exclusive with the three above.
    pub env: Option<EnvMap>,
    /// Variables removed from the daemon's (and so the child's) environment.
    pub scrub_env: Vec<String>,
    /// The pid that already holds `<name>.lock` for this creation; carried
    /// for the CLI-fallback path (`PTY_CREATION_LOCK_OWNER_PID`).
    pub creation_lock_owner_pid: Option<i32>,
    /// Set `PTY_SPAWNER_PID` so the daemon shuts down when this process dies.
    pub bind_to_spawner_lifetime: bool,
    /// Override of the 30 s start budget.
    pub start_timeout: Option<Duration>,
}

/// What a successful spawn learned about the daemon it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnedDaemon {
    pub pid: u32,
    pub generation: String,
}

/// A failed spawn, with Node's message as its `Display`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnError {
    /// `env` together with `isolate_env` / `extra_env` / `unset_env`.
    EnvExclusive,
    /// The daemon process died before publishing.
    DaemonExited { code: Option<i32>, stderr: String },
    /// No socket within the budget.
    SocketTimeout { name: String },
    /// A socket, but no matching metadata + `session_start` within the budget.
    PublicationTimeout { name: String },
    /// Spawning the process itself failed.
    Io(String),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnError::EnvExclusive => write!(
                f,
                "SpawnDaemonOptions.env is mutually exclusive with isolateEnv/extraEnv/unsetEnv. \
                 Use env for verbatim control, or inherited environment policy options — not both."
            ),
            SpawnError::DaemonExited { code, stderr } => {
                let code = code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".to_string());
                let msg = format!("Daemon process exited immediately (code {code}).");
                if stderr.is_empty() {
                    write!(f, "{msg} Is the command valid?")
                } else {
                    write!(f, "{msg}\n{stderr}")
                }
            }
            SpawnError::SocketTimeout { name } => {
                write!(f, "Timeout waiting for session \"{name}\" to start")
            }
            SpawnError::PublicationTimeout { name } => write!(
                f,
                "Timed out waiting for daemon publication for session \"{name}\"."
            ),
            SpawnError::Io(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for SpawnError {}

/// The config object Node's spawner serializes, in its key order.
///
/// node: src/spawn.ts:169-184
pub fn config_for(params: &SpawnParams) -> DaemonConfig {
    DaemonConfig {
        name: params.name.clone(),
        command: params.command.clone(),
        args: params.args.clone(),
        display_command: params.display_command.clone(),
        cwd: Some(params.cwd.clone()),
        rows: Some(params.rows),
        cols: Some(params.cols),
        ephemeral: params.ephemeral,
        tags: (!params.tags.is_empty()).then(|| params.tags.clone()),
        display_name: params
            .display_name
            .clone()
            .filter(|d| !d.is_empty()),
        isolate_env: params.isolate_env.then_some(true),
        extra_env: (!params.extra_env.is_empty()).then(|| params.extra_env.clone()),
        unset_env: (!params.unset_env.is_empty()).then(|| params.unset_env.clone()),
        env: params.env.clone(),
        generation: None,
    }
}

/// Name this process for `ps`, `top` and `/proc/<pid>/comm`.
///
/// **Linux only, and that is a gap rather than a decision.** On Linux this is
/// `prctl(PR_SET_NAME)`, which moves the name `ps -o comm=` reads.
///
/// **On macOS the daemon still shows the binary's whole path, where the Node
/// tool shows `pty-daemon`.** Two attempts are recorded here so nobody
/// repeats them:
///
/// - **"macOS has no equivalent" — false.** This file said so until
///   2026-09-02 and the Node tool disproves it.
/// - **`pthread_setname_np` — compiles, and does not do this job.** Apple's
///   one-argument form names the calling THREAD, for debuggers and
///   Instruments, and macOS does not surface thread names through `ps` at
///   all. Tried on 2026-09-02: `comm`, `ucomm` and `args` were all unchanged.
///
/// **What Node actually does is rewrite the process-argument region**, which
/// is where macOS `ps -o comm=` reads its answer. Its daemon's `args` field
/// is the bare string `pty-daemon` rather than a command line, which is the
/// tell. Doing the same here means overwriting the memory `argv` points at,
/// in place, within the space the kernel gave us — and getting that wrong
/// corrupts the arguments of a live process.
///
/// **So it is left undone rather than half done.** It is cosmetic: it changes
/// what `ps` and `top` display and nothing reads it. `docs/parity.md` records
/// it as absent.
pub fn set_process_title(title: &str) {
    #[cfg(target_os = "linux")]
    {
        let truncated: String = title.chars().take(15).collect();
        if let Ok(c) = std::ffi::CString::new(truncated) {
            // SAFETY: PR_SET_NAME reads a NUL-terminated string.
            unsafe {
                libc::prctl(libc::PR_SET_NAME, c.as_ptr() as libc::c_ulong, 0, 0, 0);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = title;
    }
}

/// Node's `hasPublishedSessionStart`: a `session_start` line for `name`
/// whose `ts >= created_at` (string comparison, as Node does).
///
/// node: src/spawn.ts:245-260
pub fn has_published_session_start(name: &str, created_at: &str) -> bool {
    let Ok(content) = std::fs::read_to_string(registry::events_path(name)) else {
        return false;
    };
    content.trim_end().split('\n').any(|line| {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            return false;
        };
        v.get("session").and_then(|s| s.as_str()) == Some(name)
            && v.get("type").and_then(|t| t.as_str()) == Some("session_start")
            && v.get("ts")
                .and_then(|t| t.as_str())
                .map(|ts| ts >= created_at)
                .unwrap_or(false)
    })
}

/// Has the daemon at `pid` published `name`? Metadata `daemonPid` must be
/// this pid and a `session_start` stamped at or after its `createdAt` must
/// be on disk.
///
/// node: src/spawn.ts:225-236
pub fn is_published_by(name: &str, pid: u32) -> bool {
    let Some(meta) = registry::read_metadata(name) else {
        return false;
    };
    meta.daemon_pid == Some(pid as i32) && has_published_session_start(name, &meta.created_at)
}

struct ChildWatch {
    child: std::process::Child,
    stderr: std::sync::Arc<std::sync::Mutex<String>>,
    exit_code: Option<Option<i32>>,
}

impl ChildWatch {
    fn check_early_exit(&mut self) -> Result<(), SpawnError> {
        if self.exit_code.is_none()
            && let Ok(Some(status)) = self.child.try_wait()
        {
            self.exit_code = Some(status.code());
        }
        if let Some(code) = self.exit_code {
            let stderr = self.stderr.lock().map(|s| s.trim().to_string()).unwrap_or_default();
            return Err(SpawnError::DaemonExited { code, stderr });
        }
        Ok(())
    }
}

/// Spawn `<current_exe> __daemon` detached and wait until it has published
/// the session. On any failure after the process started (and it has not
/// already exited) the daemon is sent SIGTERM.
///
/// node: src/spawn.ts:164-243
pub fn spawn_daemon(params: SpawnParams) -> Result<SpawnedDaemon, SpawnError> {
    if params.env.is_some()
        && (params.isolate_env || !params.extra_env.is_empty() || !params.unset_env.is_empty())
    {
        return Err(SpawnError::EnvExclusive);
    }
    let name = params.name.clone();
    let timeout = params.start_timeout.unwrap_or(DEFAULT_START_TIMEOUT);
    let config = serde_json::to_string(&config_for(&params))
        .map_err(|e| SpawnError::Io(format!("cannot encode daemon config: {e}")))?;
    let exe = std::env::current_exe()
        .map_err(|e| SpawnError::Io(format!("cannot find own executable: {e}")))?;

    // The config travels on a pipe the child inherits as fd 3.
    let mut fds = [0i32; 2];
    // SAFETY: pipe(2) fills two descriptors.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(SpawnError::Io(format!(
            "cannot create config pipe: {}",
            std::io::Error::last_os_error()
        )));
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);
    // SAFETY: freshly created descriptors owned here.
    let mut write_end = unsafe { std::fs::File::from_raw_fd(write_fd) };
    // The write end must not leak into the daemon (it would keep its own
    // config pipe open for ever).
    // SAFETY: fcntl on a descriptor we own.
    unsafe {
        libc::fcntl(write_fd, libc::F_SETFD, libc::FD_CLOEXEC);
    }

    let mut cmd = Command::new(exe);
    cmd.arg("__daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .env_remove("PTY_SERVER_CONFIG");
    for key in &params.scrub_env {
        cmd.env_remove(key);
    }
    if params.bind_to_spawner_lifetime {
        cmd.env("PTY_SPAWNER_PID", std::process::id().to_string());
    }
    match params.creation_lock_owner_pid {
        Some(pid) => {
            cmd.env("PTY_CREATION_LOCK_OWNER_PID", pid.to_string());
        }
        None => {
            cmd.env_remove("PTY_CREATION_LOCK_OWNER_PID");
        }
    }
    // SAFETY: the pre_exec body only calls async-signal-safe functions
    // (setsid, dup2, close).
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            libc::setsid();
            if read_fd != super::config::CONFIG_FD {
                if libc::dup2(read_fd, super::config::CONFIG_FD) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::close(read_fd);
            } else {
                // Already fd 3: clear CLOEXEC so it survives the exec.
                libc::fcntl(read_fd, libc::F_SETFD, 0);
            }
            Ok(())
        });
    }
    let spawned = cmd.spawn();
    // SAFETY: the parent's copy of the read end is closed here regardless.
    unsafe {
        libc::close(read_fd);
    }
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => return Err(SpawnError::Io(format!("failed to start daemon: {e}"))),
    };

    // Hand the config over from a helper thread so a config larger than the
    // pipe buffer cannot wedge this side if the daemon dies early.
    let config_bytes = config.into_bytes();
    std::thread::spawn(move || {
        let _ = write_end.write_all(&config_bytes);
        let _ = write_end.flush();
        drop(write_end);
    });

    let stderr_buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    if let Some(mut stderr) = child.stderr.take() {
        let buf = stderr_buf.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match stderr.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut b) = buf.lock() {
                            b.push_str(&String::from_utf8_lossy(&chunk[..n]));
                        }
                    }
                }
            }
        });
    }
    let pid = child.id();
    let mut watch = ChildWatch {
        child,
        stderr: stderr_buf,
        exit_code: None,
    };

    let outcome = wait_for_publication(&name, pid, timeout, &mut watch);
    if let Err(err) = &outcome
        && !matches!(err, SpawnError::DaemonExited { .. })
    {
        // SAFETY: SIGTERM to the pid we spawned.
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
    // Leave the daemon unreaped: it is detached (its own session) and
    // outlives this process; a dead one is reaped by init.
    std::mem::forget(watch.child);
    outcome
}

fn wait_for_publication(
    name: &str,
    pid: u32,
    timeout: Duration,
    watch: &mut ChildWatch,
) -> Result<SpawnedDaemon, SpawnError> {
    let started = Instant::now();
    wait_for_socket(name, timeout, || watch.check_early_exit())?;
    loop {
        if is_published_by(name, pid) {
            let generation = registry::read_metadata(name)
                .and_then(|m| m.generation)
                .unwrap_or_default();
            return Ok(SpawnedDaemon { pid, generation });
        }
        watch.check_early_exit()?;
        if started.elapsed() >= timeout {
            return Err(SpawnError::PublicationTimeout {
                name: name.to_string(),
            });
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Node's `waitForSocket`: stat every 50 ms, then settle 100 ms once the
/// socket exists. `early_check` runs before every probe.
///
/// node: src/spawn.ts:336-370
pub fn wait_for_socket(
    name: &str,
    timeout: Duration,
    mut early_check: impl FnMut() -> Result<(), SpawnError>,
) -> Result<(), SpawnError> {
    let socket = registry::socket_path(name);
    let start = Instant::now();
    loop {
        early_check()?;
        if start.elapsed() > timeout {
            return Err(SpawnError::SocketTimeout {
                name: name.to_string(),
            });
        }
        if std::fs::metadata(&socket).is_ok() {
            std::thread::sleep(Duration::from_millis(100));
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_texts_match_node() {
        assert_eq!(
            SpawnError::DaemonExited {
                code: Some(1),
                stderr: "boom".into()
            }
            .to_string(),
            "Daemon process exited immediately (code 1).\nboom"
        );
        assert_eq!(
            SpawnError::DaemonExited {
                code: None,
                stderr: String::new()
            }
            .to_string(),
            "Daemon process exited immediately (code unknown). Is the command valid?"
        );
        assert_eq!(
            SpawnError::SocketTimeout { name: "id".into() }.to_string(),
            "Timeout waiting for session \"id\" to start"
        );
        assert_eq!(
            SpawnError::PublicationTimeout { name: "id".into() }.to_string(),
            "Timed out waiting for daemon publication for session \"id\"."
        );
    }

    #[test]
    fn config_follows_the_spawner_shape() {
        let mut tags = TagMap::new();
        tags.insert("k".into(), "v".into());
        let params = SpawnParams {
            name: "n".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "true".into()],
            display_command: "sh -c true".into(),
            cwd: "/tmp".into(),
            rows: 24,
            cols: 80,
            tags,
            display_name: Some(String::new()),
            ..Default::default()
        };
        let json = serde_json::to_string(&config_for(&params)).unwrap();
        assert_eq!(
            json,
            r#"{"name":"n","command":"/bin/sh","args":["-c","true"],"displayCommand":"sh -c true","cwd":"/tmp","rows":24,"cols":80,"ephemeral":false,"tags":{"k":"v"}}"#
        );
    }

    #[test]
    fn env_exclusivity_is_checked_before_spawning() {
        let params = SpawnParams {
            name: "n".into(),
            env: Some(EnvMap::new()),
            isolate_env: true,
            ..Default::default()
        };
        assert_eq!(spawn_daemon(params).unwrap_err(), SpawnError::EnvExclusive);
    }
}
