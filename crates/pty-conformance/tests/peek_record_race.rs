//! `pty peek` reads a gone session's record exactly once.
//!
//! It used to read it twice: a match guard asked whether the record was
//! there, and the body read it again and unwrapped it. A `pty rm`, a reap,
//! or a `gc` between the two reads left the second one with nothing, and
//! the CLI panicked instead of reporting. The panic was rare (about six
//! runs in three hundred), which is exactly why it needs a test that runs
//! the race many times.

use pty_conformance::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// How many times to run the race. The old defect showed up in roughly one
/// run in fifty, so this many attempts makes a miss unlikely.
const ATTEMPTS: usize = 300;

#[test]
fn peek_survives_the_record_going_away_under_it() {
    let rig = Rig::new();
    let id = "peek-record-race";

    // A session that has already finished. `peek` then takes the branch
    // that reads the record: the socket is gone, so it falls back to the
    // saved output.
    let d = rig.daemon_try(id, &["sh", "-c", "printf saved-output"], DaemonOpts::keep());
    assert_eq!(d.launch.status, 0, "run -d failed: {}", d.launch.summary());
    wait_until(&format!("{id} exit record"), || {
        rig.meta(id).map(|m| m.get("exitCode").is_some()).unwrap_or(false)
    });
    let meta = rig.meta_path(id);
    let saved = std::fs::read(&meta).expect("read the finished record");

    // Put the record back and take it away as fast as the filesystem will
    // let us, for the whole run. The window between two reads of the same
    // file is microseconds wide, so it is churn that finds it, not timing.
    let stop = Arc::new(AtomicBool::new(false));
    let churn = {
        let stop = Arc::clone(&stop);
        let meta = meta.clone();
        let saved = saved.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let _ = std::fs::write(&meta, &saved);
                let _ = std::fs::remove_file(&meta);
            }
        })
    };

    let mut panicked = 0usize;
    let mut reached_the_record = 0usize;
    let mut resolved_nothing = 0usize;
    for _ in 0..ATTEMPTS {
        let out = rig.pty(&["peek", id]);
        let stderr = out.stderr();
        let stdout = out.stdout();
        if stderr.contains("panicked") || out.status == 101 {
            panicked += 1;
        }
        if stdout.contains("saved-output") || stderr.contains("no saved output") {
            reached_the_record += 1;
        } else if stderr.contains("not found") {
            resolved_nothing += 1;
        }
    }
    stop.store(true, Ordering::Relaxed);
    churn.join().expect("churn thread");
    let _ = std::fs::write(&meta, &saved);

    assert_eq!(
        panicked, 0,
        "pty peek panicked in {panicked} of {ATTEMPTS} runs \
         ({reached_the_record} reached the record, {resolved_nothing} resolved nothing)"
    );
    // Without this the test could pass by never reaching the branch at all.
    assert!(
        reached_the_record > 0,
        "no run got as far as reading the record; \
         {resolved_nothing} of {ATTEMPTS} resolved nothing, so the race was not exercised"
    );
}
