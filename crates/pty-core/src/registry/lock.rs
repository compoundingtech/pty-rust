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
//!
//! # These locks are not exclusive across a crash
//!
//! **A lock whose holder died is stolen, and two processes stealing the same
//! stale lock can both end up holding it.** Measured on 2026-09-02: eight
//! threads released together against one stale lock, over four hundred
//! rounds, produced more than one winner in 386 of them.
//!
//! The steal is a read, a decision and then an unlink followed by a create,
//! and nothing binds those together. A second process that made its decision
//! from the same file unlinks what the first one has already put there. **The
//! loser removes the winner's lock and then takes it**, and either one's
//! release can remove the other's file.
//!
//! **The Node tool has the identical sequence and the identical defect**
//! (`src/sessions.ts`, `acquireFileLock`), so a shared `$PTY_ROOT` is no
//! worse than either implementation alone. This is not a difference between
//! them.
//!
//! **So do not rely on these locks for correctness after a crash.** Taking
//! one still keeps two live, healthy processes apart, which is what it is for
//! in ordinary use. It does not settle a race between two processes tidying
//! up after a daemon that died holding it.
//!
//! **A correct steal needs one exclusive create that only one process can
//! win**, which means funnelling the steal through a second file — and that
//! file lives in a directory both implementations read, so it is a change to
//! a protocol they share and has to be agreed between them rather than added
//! on one side. It is left undone deliberately. See `docs/hardening.md`,
//! "Stealing a stale lock is not exclusive", for the interleaving in full.

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
///
/// **Stealing a stale lock is not exclusive.** See the [module docs](self).
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
        // Somebody else got there first, which is fine: fall through and
        // race them for the create.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // Anything else is a registry this process cannot write. Returning
        // "not acquired" here would report it as a busy lock and ask for a
        // retry that can never work, which is the whole point of returning
        // a result from this function.
        Err(e) => return Err(e),
    }
    Ok(try_create(lock_path)?.then(|| guard(lock_path)))
}

/// [`try_acquire_file_lock`] with I/O errors folded into `None`.
///
/// Use it only where "not taken" is the whole answer. Anything that reports
/// to a caller wants [`lock_or_refusal`] instead: folding an I/O error into
/// `None` turns a read-only registry into "the event log is busy, retry",
/// which is untrue and sends the caller round a loop that cannot end.
///
/// **Stealing a stale lock is not exclusive.** See the [module docs](self).
pub fn acquire_file_lock(lock_path: &Path) -> Option<LockGuard> {
    try_acquire_file_lock(lock_path).ok().flatten()
}

/// Why a lock was not taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockRefusal {
    /// A live process holds it. Retrying can work.
    Busy,
    /// The lock file could not be created at all: a read-only registry, a
    /// full disk, a directory this process may not write. Retrying cannot
    /// work, so the message must not ask for a retry. Node throws the same
    /// error out of `acquireFileLock` rather than reporting "busy".
    Unavailable(String),
}

/// Take `lock_path` and say why when it refuses.
///
/// node: src/sessions.ts:2293-2336 (`acquireFileLock` returns false only on
/// `EEXIST` and rethrows every other error).
///
/// **Stealing a stale lock is not exclusive.** See the [module docs](self).
pub fn lock_or_refusal(lock_path: &Path) -> Result<LockGuard, LockRefusal> {
    match try_acquire_file_lock(lock_path) {
        Ok(Some(guard)) => Ok(guard),
        Ok(None) => Err(LockRefusal::Busy),
        Err(e) => Err(LockRefusal::Unavailable(format!(
            "{}: {e}",
            lock_path.display()
        ))),
    }
}

/// Take `<name>.events.lock`, with Node's busy text when a live holder has
/// it and the real cause when the file cannot be created.
///
/// **Stealing a stale lock is not exclusive.** See the [module docs](self).
pub fn take_event_lock(name: &str) -> Result<LockGuard, String> {
    lock_or_refusal(&event_lock_path(name)).map_err(|r| match r {
        LockRefusal::Busy => event_busy_message(name),
        LockRefusal::Unavailable(cause) => cause,
    })
}

/// Take `<name>.lock`, with Node's busy text when a live holder has it and
/// the real cause when the file cannot be created.
///
/// **Stealing a stale lock is not exclusive.** See the [module docs](self).
pub fn take_metadata_lock(name: &str) -> Result<LockGuard, String> {
    lock_or_refusal(&lock_path(name)).map_err(|r| match r {
        LockRefusal::Busy => metadata_busy_message(name),
        LockRefusal::Unavailable(cause) => cause,
    })
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
///
/// **Stealing a stale lock is not exclusive.** See the [module docs](self).
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
///
/// **Stealing a stale lock is not exclusive.** See the [module docs](self).
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
///
/// **Stealing a stale lock is not exclusive.** See the [module docs](self).
pub fn wait_for_event_lock(name: &str, wait: Duration) -> Result<LockGuard, String> {
    let deadline = Instant::now() + wait;
    let path = event_lock_path(name);
    loop {
        match lock_or_refusal(&path) {
            Ok(guard) => return Ok(guard),
            // The lock file cannot be made at all. Waiting five seconds to
            // say so would be five seconds spent on an answer that will not
            // change.
            Err(LockRefusal::Unavailable(cause)) => return Err(cause),
            Err(LockRefusal::Busy) => {}
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
    /// Neither lock file could be created. Carries the cause, because
    /// "busy, retry" would be untrue.
    Unavailable(String),
}

impl LockBusy {
    /// Node's error text for the refusing lock.
    pub fn message(&self, name: &str) -> String {
        match self {
            LockBusy::Events => event_busy_message(name),
            LockBusy::Metadata => metadata_busy_message(name),
            LockBusy::Unavailable(cause) => cause.clone(),
        }
    }
}

/// Run `f` holding both locks, taken in Node's order: event lock, then the
/// creation/metadata lock. Neither waits.
///
/// node: src/sessions.ts:2188-2202
///
/// **Stealing a stale lock is not exclusive.** See the [module docs](self).
pub fn with_both_locks<T>(name: &str, f: impl FnOnce() -> T) -> Result<T, LockBusy> {
    let events = lock_or_refusal(&event_lock_path(name)).map_err(|r| match r {
        LockRefusal::Busy => LockBusy::Events,
        LockRefusal::Unavailable(cause) => LockBusy::Unavailable(cause),
    })?;
    let metadata = lock_or_refusal(&lock_path(name)).map_err(|r| match r {
        LockRefusal::Busy => LockBusy::Metadata,
        LockRefusal::Unavailable(cause) => LockBusy::Unavailable(cause),
    })?;
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
