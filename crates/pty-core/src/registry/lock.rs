//! Node's file-lock protocol, implemented exactly so Rust and Node writers
//! can share one `$PTY_ROOT`: `open(O_CREAT|O_EXCL, 0600)`, holder pid in
//! the file, a dead or garbage holder is stolen with exactly one retry, and
//! release is `unlink`.
//!
//! Two locks per session: `<name>.lock` (creation/metadata) and
//! `<name>.events.lock` (event log). Whenever both are taken the order is
//! event lock first, then creation lock (`with_both_locks`).
//!
//! node: src/sessions.ts:2273-2336, 2374-2386; src/events.ts:224-249

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::list::pid_alive;
use super::root::{ensure_session_dir, event_lock_path, lock_path};

/// The text Node throws when the event lock is held (`events.ts:229-295`).
pub fn event_busy_message(name: &str) -> String {
    format!("Session id \"{name}\" event log is busy. Retry the operation.")
}

/// The text Node throws when the creation/metadata lock is held.
pub fn metadata_busy_message(name: &str) -> String {
    format!("Session id \"{name}\" metadata is busy. Retry the operation.")
}

/// A held file lock; dropping it unlinks the lock file.
#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
    armed: bool,
}

impl LockGuard {
    /// The lock file this guard owns.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Release now (idempotent; dropping does the same).
    pub fn release(mut self) {
        self.armed = false;
        release_file_lock(&self.path);
    }

    /// Keep the lock file on disk when this guard drops (the caller takes
    /// over the release).
    pub fn forget(mut self) {
        self.armed = false;
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if self.armed {
            release_file_lock(&self.path);
        }
    }
}

fn try_create(lock_path: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(lock_path)
    {
        Ok(mut f) => {
            f.write_all(std::process::id().to_string().as_bytes())?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

/// Acquire an exclusive file lock at `lock_path`. `Some(guard)` when
/// acquired, `None` when another live process holds it. A lock whose holder
/// pid is dead (or unreadable/garbage) is stolen: unlink, then retry the
/// exclusive create exactly once — a racing stealer gets `None`.
///
/// I/O errors other than `EEXIST` are surfaced as `Err`, as Node rethrows
/// them.
///
/// node: src/sessions.ts:2293-2336
pub fn try_acquire_file_lock(lock_path: &Path) -> std::io::Result<Option<LockGuard>> {
    ensure_session_dir()?;
    let guard = |path: &Path| LockGuard {
        path: path.to_path_buf(),
        armed: true,
    };
    if try_create(lock_path)? {
        return Ok(Some(guard(lock_path)));
    }
    let holder_alive = std::fs::read_to_string(lock_path)
        .ok()
        .and_then(|s| parse_leading_int(s.trim()))
        .is_some_and(pid_alive);
    if holder_alive {
        return Ok(None);
    }
    match std::fs::remove_file(lock_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Ok(None),
    }
    Ok(try_create(lock_path)?.then(|| guard(lock_path)))
}

/// [`try_acquire_file_lock`] with I/O errors folded into `None`.
pub fn acquire_file_lock(lock_path: &Path) -> Option<LockGuard> {
    try_acquire_file_lock(lock_path).ok().flatten()
}

/// Release a lock by path (unlink; missing is fine).
///
/// node: src/sessions.ts:2374-2378
pub fn release_file_lock(lock_path: &Path) {
    let _ = std::fs::remove_file(lock_path);
}

/// `parseInt(s, 10)`: leading integer, `None` when there is none.
pub(crate) fn parse_leading_int(s: &str) -> Option<i32> {
    let s = s.trim_start();
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let end = digits.bytes().take_while(u8::is_ascii_digit).count();
    if end == 0 {
        return None;
    }
    let value: i64 = digits[..end].parse().ok()?;
    let value = if neg { -value } else { value };
    i32::try_from(value).ok()
}

/// Acquire the creation/metadata lock `<name>.lock`.
pub fn acquire_lock(name: &str) -> Option<LockGuard> {
    acquire_file_lock(&lock_path(name))
}

/// Release `<name>.lock` regardless of holder (Node's `releaseLock`).
pub fn release_lock(name: &str) {
    release_file_lock(&lock_path(name));
}

/// Acquire the event lock `<name>.events.lock` without waiting.
///
/// node: src/events.ts:228-230
pub fn acquire_event_lock(name: &str) -> Option<LockGuard> {
    acquire_file_lock(&event_lock_path(name))
}

/// Release `<name>.events.lock` regardless of holder.
pub fn release_event_lock(name: &str) {
    release_file_lock(&event_lock_path(name));
}

/// How long async writers wait for the event lock (`EVENT_LOCK_WAIT_MS`).
pub const EVENT_LOCK_WAIT: Duration = Duration::from_millis(5_000);

/// Acquire the event lock, polling every 10 ms for up to `wait`. Fails with
/// Node's busy text when the deadline passes.
///
/// node: src/events.ts:237-249
pub fn wait_for_event_lock(name: &str, wait: Duration) -> Result<LockGuard, String> {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(guard) = acquire_event_lock(name) {
            return Ok(guard);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(event_busy_message(name));
        }
        std::thread::sleep(Duration::from_millis(10).min(deadline - now));
    }
}

/// Which of the two locks refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockBusy {
    /// `<name>.events.lock` is held by a live process.
    Events,
    /// `<name>.lock` is held by a live process.
    Metadata,
}

impl LockBusy {
    /// Node's error text for the refusing lock.
    pub fn message(&self, name: &str) -> String {
        match self {
            LockBusy::Events => event_busy_message(name),
            LockBusy::Metadata => metadata_busy_message(name),
        }
    }
}

/// Run `f` holding both locks, taken in Node's order: event lock, then the
/// creation/metadata lock. Neither waits.
///
/// node: src/sessions.ts:2188-2202
pub fn with_both_locks<T>(name: &str, f: impl FnOnce() -> T) -> Result<T, LockBusy> {
    let events = acquire_event_lock(name).ok_or(LockBusy::Events)?;
    let metadata = acquire_lock(name).ok_or(LockBusy::Metadata)?;
    let out = f();
    drop(metadata);
    drop(events);
    Ok(out)
}

/// Verify an explicitly delegated creation lock (`PTY_CREATION_LOCK_OWNER_PID`)
/// without acquiring it: the file holds `owner_pid` and that process lives.
///
/// node: src/sessions.ts:2273-2281
pub fn is_lock_owned_by_pid(name: &str, owner_pid: i32) -> bool {
    if owner_pid <= 0 {
        return false;
    }
    std::fs::read_to_string(lock_path(name))
        .ok()
        .and_then(|s| parse_leading_int(s.trim()))
        .is_some_and(|pid| pid == owner_pid && pid_alive(owner_pid))
}
