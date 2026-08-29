//! Attach synchronization order: per generation `GEOMETRY → SCREEN →
//! DATA* → EXIT?`, output produced during synchronization folded into the
//! SCREEN, supersession by a newer ATTACH/PEEK on the same socket.
//!
//! node: tests/integration.test.ts:423-852

mod daemon_support;

use std::time::Duration;

use daemon_support::*;
use pty_core::protocol::MessageType::{self, *};

const T: Duration = Duration::from_secs(5);

/// A wide settle window so the child's output lands during synchronization.
const WIDE_SETTLE: &[(&str, &str)] = &[("PTY_REDRAW_SETTLE_MS", "1500")];

fn cat_daemon(settle: &[(&str, &str)]) -> Daemon {
    let root = short_root();
    Daemon::start_env(&root, config(&unique_name("ord"), "cat", &[]), settle)
}

/// node: tests/integration.test.ts:423-471
#[test]
fn live_data_during_attach_sync_is_folded_into_screen() {
    let _s = serial();
    let d = cat_daemon(WIDE_SETTLE);
    let mut live = d.connect();
    live.attach(24, 80);
    assert!(live.wait_type(Screen, T));

    let mut attaching = d.connect();
    attaching.attach(20, 70);
    assert!(attaching.wait_type(Geometry, T));

    live.data("during-initial-sync\n");
    assert!(live.wait_text("during-initial-sync", T));

    assert!(attaching.wait_type(Screen, T));
    assert_eq!(attaching.types(), vec![Geometry, Screen]);
    assert!(attaching.screen().unwrap().contains("during-initial-sync"));
}

/// node: tests/integration.test.ts:473-529
#[test]
fn pre_cut_data_is_not_lost() {
    let _s = serial();
    let root = short_root();
    let d = Daemon::start_env(
        &root,
        config(&unique_name("ord"), "sh", &["-c", "stty -echo; cat"]),
        WIDE_SETTLE,
    );
    std::thread::sleep(Duration::from_millis(100));
    let mut live = d.connect();
    live.attach(24, 80);
    assert!(live.wait_type(Screen, T));

    let mut attaching = d.connect();
    attaching.attach(20, 70);
    assert!(attaching.wait_type(Geometry, T));
    live.data("parser-backlog\n");
    assert!(live.wait_text("parser-backlog", T));

    assert!(attaching.wait_type(Screen, T));
    assert!(attaching.output().contains("parser-backlog"));
}

/// node: tests/integration.test.ts:531-616
#[test]
fn post_cut_data_precedes_post_cut_exit_with_exactly_one_exit() {
    let _s = serial();
    let root = short_root();
    let d = Daemon::start(
        &root,
        config(
            &unique_name("ord"),
            "sh",
            &["-c", "stty -echo; read value; printf 'post-cut-data'; exit 7"],
        ),
    );
    std::thread::sleep(Duration::from_millis(100));
    let mut live = d.connect();
    live.attach(24, 80);
    assert!(live.wait_type(Screen, T));

    let mut attaching = d.connect();
    attaching.attach(20, 70);
    assert!(attaching.wait_type(Geometry, T));
    attaching.resize(24, 80);
    assert!(attaching.wait_count(Geometry, 2, T));
    assert!(attaching.wait_type(Screen, T));

    live.data("go\n");
    assert!(attaching.wait_type(Exit, T));
    attaching.settle(Duration::from_millis(200));

    let types = attaching.types();
    assert_eq!(&types[..3], &[Geometry, Geometry, Screen], "{types:?}");
    assert_eq!(types.last(), Some(&Exit), "{types:?}");
    assert!(types[3..types.len() - 1].iter().all(|t| *t == Data), "{types:?}");
    assert!(attaching.output().contains("post-cut-data"));
    assert_eq!(attaching.exit_codes(), vec![7]);
}

/// After the exit, an attacher gets its baseline and one EXIT, immediately.
///
/// node: src/server.ts:992-993, 1248-1250; tests/integration.test.ts:618-699
#[test]
fn attach_after_exit_is_geometry_screen_exit() {
    let _s = serial();
    let root = short_root();
    let d = Daemon::start(
        &root,
        config(&unique_name("ord"), "sh", &["-c", "echo final-line; exit 7"]),
    );
    assert!(wait_until(T, || d.meta().and_then(|m| m["exitCode"].as_i64()) == Some(7)));
    let mut late = d.connect();
    late.attach(20, 70);
    assert!(late.wait_type(Exit, T));
    late.settle(Duration::from_millis(100));
    assert_eq!(late.types(), vec![Geometry, Screen, Exit]);
    assert!(late.screen().unwrap().contains("final-line"));
    assert_eq!(late.exit_codes(), vec![7]);
}

/// node: tests/integration.test.ts:701-762
#[test]
fn exit_during_sync_yields_one_exit_after_screen() {
    let _s = serial();
    let root = short_root();
    // The window must stay inside the 500 ms exit grace, or the daemon
    // shuts down before the cut.
    let d = Daemon::start_env(
        &root,
        config(&unique_name("ord"), "sh", &[]),
        &[("PTY_REDRAW_SETTLE_MS", "350")],
    );
    let mut live = d.connect();
    live.attach(24, 80);
    assert!(live.wait_type(Screen, T));

    let mut attaching = d.connect();
    attaching.attach(20, 70);
    assert!(attaching.wait_type(Geometry, T));
    attaching.resize(24, 80);
    assert!(attaching.wait_count(Geometry, 2, T));

    live.data("exit 7\n");
    assert!(live.wait_type(Exit, T));

    assert!(attaching.wait_type(Exit, T));
    assert_eq!(attaching.types(), vec![Geometry, Geometry, Screen, Exit]);
    assert_eq!(attaching.exit_codes(), vec![7]);
}

/// node: tests/integration.test.ts:764-800
#[test]
fn second_attach_supersedes_pending_attach() {
    let _s = serial();
    let d = cat_daemon(&[]);
    let mut live = d.connect();
    live.attach(24, 80);
    assert!(live.wait_type(Screen, T));

    let mut re = d.connect();
    re.attach(20, 70);
    assert!(re.wait_for(T, |p| !p.is_empty()));
    re.attach(18, 60);
    assert!(re.wait_for(T, |p| p.len() >= 2));
    assert!(re.wait_type(Screen, T));
    re.settle(Duration::from_millis(250));
    assert_eq!(re.types(), vec![Geometry, Geometry, Screen]);
}

/// node: tests/integration.test.ts:802-852
#[test]
fn peek_cancels_pending_attach_sync_and_stays_live() {
    let _s = serial();
    let d = cat_daemon(WIDE_SETTLE);
    let mut live = d.connect();
    live.attach(24, 80);
    assert!(live.wait_type(Screen, T));

    let mut peeker = d.connect();
    peeker.attach(20, 70);
    assert!(peeker.wait_for(T, |p| !p.is_empty()));
    peeker.peek();
    assert!(peeker.wait_type(Screen, T));
    assert_eq!(peeker.types(), vec![Geometry, Geometry, Screen]);

    live.data("peek-is-live\n");
    assert!(live.wait_text("peek-is-live", T));
    assert!(peeker.wait_for(T, |p| p
        .iter()
        .any(|x| x.type_ == Data && String::from_utf8_lossy(&x.payload).contains("peek-is-live"))));
    // And no second SCREEN for the peeker: the attach cut was cancelled.
    peeker.settle(Duration::from_millis(1600));
    assert_eq!(peeker.count(Screen), 1);
}

/// Unknown packet types and short frames are ignored; a valid ATTACH after
/// a truncated one works.
///
/// node: tests/integration.test.ts:1477-1543
#[test]
fn malformed_and_unknown_packets_are_ignored() {
    let _s = serial();
    let d = cat_daemon(&[]);
    let mut c = d.connect();
    c.send(&pty_core::protocol::encode_packet(MessageType::Attach, &[0, 24]));
    c.send(&pty_core::protocol::encode_packet(MessageType::Unknown(99), b"abc"));
    c.send(&pty_core::protocol::encode_packet(MessageType::Resize, &[1]));
    c.settle(Duration::from_millis(150));
    assert!(c.packets.is_empty(), "{:?}", c.types());
    c.attach(24, 80);
    assert!(c.wait_type(Screen, T));
    assert_eq!(c.types(), vec![Geometry, Screen]);
    c.data("still-works\n");
    assert!(c.wait_text("still-works", T));
}

/// An oversize length header drops the connection.
///
/// node: src/server.ts:918-928
#[test]
fn oversize_packet_header_drops_the_connection() {
    let _s = serial();
    let d = cat_daemon(&[]);
    let mut c = d.connect();
    let too_big = (pty_core::protocol::MAX_PACKET_LENGTH as u32 + 1).to_be_bytes();
    c.send(&[0, too_big[0], too_big[1], too_big[2], too_big[3]]);
    assert!(c.wait_closed(T));
    // The daemon is fine.
    let mut again = d.connect();
    again.attach(24, 80);
    assert!(again.wait_type(Screen, T));
}
