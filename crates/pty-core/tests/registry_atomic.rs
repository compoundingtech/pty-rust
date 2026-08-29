//! Atomic publication: a reader never sees a torn file across rewrites,
//! concurrent writers cannot corrupt each other, and no `*.tmp.*` is left
//! behind.

mod registry_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pty_core::registry;
use registry_support::root;

/// node: tests/atomic-writes.test.ts:96-134
#[test]
fn reader_never_sees_unparseable_json_across_200_rewrites() {
    let root = root();
    let target = root.join("atomic-test.json");
    registry::atomic_write(&target, br#"{"version":0}"#).unwrap();

    let done = Arc::new(AtomicBool::new(false));
    let reader_done = done.clone();
    let reader_target = target.clone();
    let reader = std::thread::spawn(move || {
        let mut errors = Vec::new();
        let mut reads = 0usize;
        while !reader_done.load(Ordering::Relaxed) {
            match std::fs::read(&reader_target) {
                Ok(bytes) => {
                    if serde_json::from_slice::<serde_json::Value>(&bytes).is_err() {
                        errors.push(String::from_utf8_lossy(&bytes).into_owned());
                    }
                }
                Err(e) => errors.push(e.to_string()),
            }
            reads += 1;
            std::thread::yield_now();
        }
        (errors, reads)
    });

    for i in 0..200 {
        registry::atomic_write(&target, format!("{{\"version\":{}}}", i + 1).as_bytes()).unwrap();
        std::thread::yield_now();
    }
    done.store(true, Ordering::Relaxed);
    let (errors, reads) = reader.join().unwrap();
    assert!(errors.is_empty(), "reader saw torn content: {errors:?}");
    assert!(reads > 0);
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "{\"version\":200}"
    );
}

/// node: tests/atomic-writes.test.ts:136-164
#[test]
fn concurrent_writers_leave_valid_json() {
    let root = root();
    let target = root.join("atomic-async.json");
    registry::atomic_write(&target, br#"{"version":0}"#).unwrap();

    let done = Arc::new(AtomicBool::new(false));
    let reader_done = done.clone();
    let reader_target = target.clone();
    let reader = std::thread::spawn(move || {
        let mut errors = Vec::new();
        while !reader_done.load(Ordering::Relaxed) {
            if let Ok(bytes) = std::fs::read(&reader_target)
                && serde_json::from_slice::<serde_json::Value>(&bytes).is_err()
            {
                errors.push(String::from_utf8_lossy(&bytes).into_owned());
            }
            std::thread::yield_now();
        }
        errors
    });

    for batch in 0..50 {
        let writers: Vec<_> = (0..4)
            .map(|k| {
                let target = target.clone();
                std::thread::spawn(move || {
                    registry::atomic_write(
                        &target,
                        format!("{{\"batch\":{batch},\"writer\":{k}}}").as_bytes(),
                    )
                    .unwrap();
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
    }
    done.store(true, Ordering::Relaxed);
    let errors = reader.join().unwrap();
    assert!(errors.is_empty(), "reader saw torn content: {errors:?}");
    let last: serde_json::Value = serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
    assert_eq!(last["batch"], 49);
}

/// node: tests/atomic-writes.test.ts:166-174
#[test]
fn no_tmp_leftovers_after_successful_writes() {
    let root = root();
    let target = root.join("atomic-clean.json");
    for i in 0..50 {
        registry::atomic_write(&target, format!("{{\"i\":{i}}}").as_bytes()).unwrap();
    }
    let leftovers: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("atomic-clean.json") && n.contains(".tmp."))
        .collect();
    assert!(leftovers.is_empty(), "leftover temporaries: {leftovers:?}");
}

/// The temporary is `<target>.tmp.<pid>.<16 hex>`, so Node readers ignore
/// it, and a failed write unlinks it.
///
/// node: src/sessions.ts:251-264
#[test]
fn tmp_name_shape_and_failure_cleanup() {
    let root = root();
    let tmp = registry::atomic::tmp_path_for(&root.join("x.json"));
    let name = tmp.file_name().unwrap().to_string_lossy().into_owned();
    let rest = name.strip_prefix("x.json.tmp.").expect("prefix");
    let (pid, hex) = rest.split_once('.').expect("pid.hex");
    assert_eq!(pid, std::process::id().to_string());
    assert_eq!(hex.len(), 16);
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    assert!(registry::is_tmp_name(&name));
    assert!(!registry::is_tmp_name("x.json"));

    // A write into a missing directory fails and leaves nothing behind.
    let missing = root.join("no-such-dir").join("y.json");
    assert!(registry::atomic_write(&missing, b"{}").is_err());
    assert!(!root.join("no-such-dir").exists());
}
