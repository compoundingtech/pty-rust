//! Port of tests/seq-delay.test.ts, the end-to-end half: the `--seq` delay
//! (default 0.3 s, `--with-delay 0` = straight stream, `--with-delay N`)
//! is really applied by `pty send`. Timing is compared as a delta against
//! the straight-stream baseline so process startup cancels out. The
//! `resolveSeqDelayMs` unit tests live in pty-core.
//!
//! The bytes are also checked: the daemon runs `sh -c 'stty raw -echo; cat > dump'`
//! and the dump must hold every item, in order, once.

use pty_conformance::*;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const ITEMS: &[&str] = &["a", "b", "c", "d", "e", "f", "g", "h", "i"];

fn seq_args(with_delay: Option<&str>) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(d) = with_delay {
        v.push("--with-delay".into());
        v.push(d.into());
    }
    for it in ITEMS {
        v.push("--seq".into());
        v.push((*it).into());
    }
    v
}

fn start_dump(rig: &Rig, name: &str) -> PathBuf {
    let dump = rig.root().join(format!("{name}.dump.bin"));
    let script = format!("stty raw -echo; cat > '{}'", dump.display());
    rig.daemon(name, &["sh", "-c", &script], DaemonOpts::no_display_name());
    std::thread::sleep(Duration::from_millis(150));
    dump
}

/// Wall-clock ms of one `pty send` invocation.
fn time_send(rig: &Rig, name: &str, with_delay: Option<&str>) -> u128 {
    let extra = seq_args(with_delay);
    let mut argv = vec!["send", name];
    argv.extend(extra.iter().map(String::as_str));
    let start = Instant::now();
    let out = rig.pty(&argv);
    expect_status(&out, 0);
    start.elapsed().as_millis()
}

/// node: tests/seq-delay.test.ts:82
#[test]
fn default_spaces_items_but_with_delay_zero_does_not() {
    let rig = Rig::new();
    let name = unique_id("sq");
    let dump = start_dump(&rig, &name);
    let straight = time_send(&rig, &name, Some("0"));
    let dflt = time_send(&rig, &name, None);
    // 8 gaps × 0.3 s ≈ 2.4 s over the straight stream; assert ≥ 1.2 s so a
    // loaded machine cannot false-fail.
    assert!(dflt > straight + 1200, "default={dflt}ms straight={straight}ms");
    // Both runs delivered every item, in order.
    let _ = poll_for(Duration::from_secs(3), || {
        std::fs::read(&dump).map(|b| b.len() >= 18).unwrap_or(false)
    });
    let bytes = std::fs::read(&dump).unwrap_or_default();
    assert_eq!(String::from_utf8_lossy(&bytes), "abcdefghiabcdefghi");
}

/// node: tests/seq-delay.test.ts:97
#[test]
fn with_delay_scales_the_spacing() {
    let rig = Rig::new();
    let name = unique_id("sq");
    let _dump = start_dump(&rig, &name);
    let straight = time_send(&rig, &name, Some("0"));
    let slow = time_send(&rig, &name, Some("0.2"));
    // 8 gaps × 0.2 s ≈ 1.6 s over the baseline; assert ≥ 0.9 s.
    assert!(slow > straight + 900, "slow={slow}ms straight={straight}ms");
}
