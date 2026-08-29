//! Node's file-lock protocol: `O_CREAT|O_EXCL`, holder pid, one stale steal,
//! release by unlink; the event lock's waiting variant and busy texts; the
//! event-then-metadata order of `with_both_locks` / `cleanup_all`.

mod registry_support;

use std::time::Duration;

use pty_core::registry::{self, LockBusy};
use registry_support::{DEAD_PID, root, unique_name};

/// node: tests/security-fixes.test.ts:47-52
#[test]
fn first_caller_wins_second_refused_while_holder_alive() {
    let _ = root();
    let name = unique_name("race");
    let first = registry::acquire_lock(&name).expect("first acquire");
    assert!(
        registry::acquire_lock(&name).is_none(),
        "second acquire must refuse"
    );
    first.release();
    assert!(
        registry::acquire_lock(&name).is_some(),
        "acquire after release"
    );
    registry::release_lock(&name);
}

/// node: tests/security-fixes.test.ts:54-65
#[test]
fn steals_a_stale_lock_whose_holder_is_dead() {
    let root = root();
    let name = unique_name("stale");
    let lock_path = root.join(format!("{name}.lock"));
    std::fs::write(&lock_path, "1").unwrap();
    std::fs::write(&lock_path, DEAD_PID.to_string()).unwrap();
    let guard = registry::acquire_lock(&name).expect("stale lock must be stolen");
    let holder: u32 = std::fs::read_to_string(&lock_path)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        holder,
        std::process::id(),
        "lock file must now hold our pid"
    );
    assert_eq!(guard.path(), lock_path.as_path());
    drop(guard);
    assert!(!lock_path.exists(), "drop releases (unlinks) the lock");
}

/// node: tests/security-fixes.test.ts:67-71
#[test]
fn garbage_lock_content_is_stale() {
    let root = root();
    let name = unique_name("garbage");
    std::fs::write(root.join(format!("{name}.lock")), "not a pid").unwrap();
    assert!(registry::acquire_lock(&name).is_some());
    registry::release_lock(&name);
}

/// node: tests/security-fixes.test.ts:73-75
#[test]
fn release_of_a_missing_lock_is_fine() {
    let _ = root();
    registry::release_lock("never-locked");
    registry::release_event_lock("never-locked");
}

/// node: tests/security-fixes.test.ts:77-87
#[test]
fn only_one_of_two_sequential_steals_wins() {
    let root = root();
    let name = unique_name("race4");
    std::fs::write(root.join(format!("{name}.lock")), DEAD_PID.to_string()).unwrap();
    let a = registry::acquire_lock(&name);
    let b = registry::acquire_lock(&name);
    assert_eq!(a.is_some() as u8 + b.is_some() as u8, 1);
}

/// The file is created `0600` and holds only the decimal pid.
///
/// node: src/sessions.ts:2298-2310
#[test]
fn lock_file_mode_and_content() {
    use std::os::unix::fs::PermissionsExt;
    let root = root();
    let name = unique_name("mode");
    let _guard = registry::acquire_lock(&name).unwrap();
    let path = root.join(format!("{name}.lock"));
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        std::process::id().to_string()
    );
}

/// node: src/sessions.ts:2273-2281; tests/gc-generation-guard.test.ts:181-207
#[test]
fn is_lock_owned_by_pid_checks_holder_and_liveness() {
    let root = root();
    let name = unique_name("owned");
    let me = std::process::id() as i32;
    assert!(!registry::is_lock_owned_by_pid(&name, me), "no lock file");
    let _guard = registry::acquire_lock(&name).unwrap();
    assert!(registry::is_lock_owned_by_pid(&name, me));
    assert!(!registry::is_lock_owned_by_pid(&name, me + 1));
    assert!(!registry::is_lock_owned_by_pid(&name, 0));
    assert!(!registry::is_lock_owned_by_pid(&name, -1));
    std::fs::write(root.join(format!("{name}.lock")), DEAD_PID.to_string()).unwrap();
    assert!(
        !registry::is_lock_owned_by_pid(&name, DEAD_PID),
        "dead holder is not an owner"
    );
}

/// node: tests/atomic-writes.test.ts:209-233
#[test]
fn cleanup_all_cannot_clean_through_a_live_metadata_lock_holder() {
    let root = root();
    let name = unique_name("busy");
    let metadata_path = root.join(format!("{name}.json"));
    let events_path = root.join(format!("{name}.events.jsonl"));
    std::fs::write(&metadata_path, format!("{{\"name\":\"{name}\"}}")).unwrap();
    std::fs::write(&events_path, "{\"type\":\"user.keep\"}\n").unwrap();
    let metadata_before = std::fs::read(&metadata_path).unwrap();
    let events_before = std::fs::read(&events_path).unwrap();

    let held = registry::acquire_lock(&name).expect("acquire");
    let err = registry::cleanup_all(&name).expect_err("must refuse");
    assert_eq!(err, LockBusy::Metadata);
    assert_eq!(
        err.message(&name),
        format!("Session id \"{name}\" metadata is busy. Retry the operation.")
    );
    assert_eq!(std::fs::read(&metadata_path).unwrap(), metadata_before);
    assert_eq!(std::fs::read(&events_path).unwrap(), events_before);
    assert!(registry::acquire_lock(&name).is_none());
    assert!(
        !root.join(format!("{name}.events.lock")).exists(),
        "the refused cleanup released the event lock"
    );
    held.release();

    registry::cleanup_all(&name).expect("cleanup after release");
    assert!(!metadata_path.exists());
    assert!(!events_path.exists());
}

/// node: tests/atomic-writes.test.ts:235-245
#[test]
fn reclaims_a_stale_event_lock_during_full_cleanup() {
    let root = root();
    let name = unique_name("staleev");
    std::fs::write(root.join(format!("{name}.events.jsonl")), "").unwrap();
    std::fs::write(root.join(format!("{name}.events.lock")), "2147483647").unwrap();
    registry::cleanup_all(&name).expect("stale event lock is reclaimed");
    assert!(!root.join(format!("{name}.events.jsonl")).exists());
    assert!(!root.join(format!("{name}.events.lock")).exists());
}

/// A live event-lock holder refuses `cleanup_all` before the metadata lock
/// is even tried.
///
/// node: src/sessions.ts:2188-2202
#[test]
fn cleanup_all_refuses_a_live_event_lock_holder_first() {
    let root = root();
    let name = unique_name("evbusy");
    let _held = registry::acquire_event_lock(&name).unwrap();
    let err = registry::cleanup_all(&name).expect_err("must refuse");
    assert_eq!(err, LockBusy::Events);
    assert_eq!(
        err.message(&name),
        format!("Session id \"{name}\" event log is busy. Retry the operation.")
    );
    assert!(
        !root.join(format!("{name}.lock")).exists(),
        "the metadata lock was never taken"
    );
}

/// node: tests/atomic-writes.test.ts:268-281
#[test]
fn wait_for_event_lock_with_zero_budget_reports_busy() {
    let _ = root();
    let name = unique_name("waitev");
    let _held = registry::acquire_event_lock(&name).unwrap();
    let err = registry::wait_for_event_lock(&name, Duration::ZERO).expect_err("busy");
    assert_eq!(
        err,
        format!("Session id \"{name}\" event log is busy. Retry the operation.")
    );
}

/// node: src/events.ts:237-249
#[test]
fn wait_for_event_lock_acquires_once_the_holder_releases() {
    let _ = root();
    let name = unique_name("waitok");
    let held = registry::acquire_event_lock(&name).unwrap();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        held.release();
    });
    let start = std::time::Instant::now();
    let guard = registry::wait_for_event_lock(&name, Duration::from_secs(5))
        .expect("acquired after release");
    assert!(
        start.elapsed() >= Duration::from_millis(100),
        "must have waited for the holder"
    );
    drop(guard);
    releaser.join().unwrap();
}

/// `with_both_locks` takes the event lock, then the creation lock, and
/// releases both.
#[test]
fn with_both_locks_takes_events_then_metadata_and_releases() {
    let root = root();
    let name = unique_name("both");
    let event_lock = root.join(format!("{name}.events.lock"));
    let meta_lock = root.join(format!("{name}.lock"));
    let out = registry::with_both_locks(&name, || {
        assert!(event_lock.exists());
        assert!(meta_lock.exists());
        42
    })
    .unwrap();
    assert_eq!(out, 42);
    assert!(!event_lock.exists());
    assert!(!meta_lock.exists());
}
