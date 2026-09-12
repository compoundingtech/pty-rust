//! Concurrent cross-binary resize arbitration through two writable clients
//! and one read-only observer. A child reports the actual terminal geometry
//! after SIGWINCH, making GEOMETRY-before-DATA ordering externally visible.

use pty_conformance::*;
use pty_core::protocol::{MessageType, Packet, decode_geometry};
use serde_json::Value;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

const ROUNDS: usize = 8;
const REPORTER: &str = r#"
report_size() {
  set -- $(stty size)
  printf '\nSTORM_SIZE:%sx%s\n' "$1" "$2"
}
printf 'STORM_READY\n'
while IFS= read -r command; do
  if test "$command" = REPORT; then report_size; fi
done
"#;

fn operation_budget() -> Duration {
    if unoptimized_binary_under_test() {
        deadline() * 2
    } else {
        deadline()
    }
}

fn marker_data_index(packets: &[Packet], marker: &[u8]) -> Option<usize> {
    let mut data = Vec::new();
    let mut packet_for_byte = Vec::new();
    for (index, packet) in packets.iter().enumerate() {
        if packet.type_ == MessageType::Data {
            data.extend_from_slice(&packet.payload);
            packet_for_byte.extend(std::iter::repeat_n(index, packet.payload.len()));
        }
    }
    let start = data
        .windows(marker.len())
        .position(|window| window == marker)?;
    packet_for_byte.get(start).copied()
}

fn geometry_index(packets: &[Packet], rows: u16, cols: u16) -> Option<usize> {
    packets.iter().position(|packet| {
        packet.type_ == MessageType::Geometry && decode_geometry(&packet.payload) == (rows, cols)
    })
}

fn collect_affected_output(observer: &mut Conn, rows: u16, cols: u16) {
    let marker = format!("STORM_SIZE:{rows}x{cols}");
    let mut packets = Vec::new();
    let started = Instant::now();
    loop {
        let remaining = operation_budget().saturating_sub(started.elapsed());
        assert!(!remaining.is_zero(), "timed out waiting for {marker}");
        if let Some(packet) = observer.next_packet(remaining) {
            packets.push(packet);
        }
        let Some(data_index) = marker_data_index(&packets, marker.as_bytes()) else {
            continue;
        };
        let geometry_index = geometry_index(&packets, rows, cols).unwrap_or_else(|| {
            panic!(
                "affected DATA arrived without GEOMETRY({rows},{cols}): {:?}",
                sequence_names(
                    &packets
                        .iter()
                        .map(|packet| packet.type_)
                        .collect::<Vec<_>>()
                )
            )
        });
        assert!(
            geometry_index < data_index,
            "GEOMETRY({rows},{cols}) at {geometry_index} did not precede affected DATA at {data_index}: {:?}",
            sequence_names(
                &packets
                    .iter()
                    .map(|packet| packet.type_)
                    .collect::<Vec<_>>()
            )
        );
        return;
    }
}

fn has_writable_request(status: &Value, rows: u16, cols: u16) -> bool {
    status["clients"]["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|connection| {
            connection["role"] == "writable"
                && connection["rows"] == u64::from(rows)
                && connection["cols"] == u64::from(cols)
        })
}

fn wait_for_arbitrated_status(
    rig: &Rig,
    id: &str,
    a: (u16, u16),
    b: (u16, u16),
    effective: (u16, u16),
) -> Value {
    let started = Instant::now();
    loop {
        let remaining = operation_budget().saturating_sub(started.elapsed());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for resize requests {a:?} and {b:?}"
        );
        let mut status_client = rig.connect(id);
        let status = status_client.status_json(remaining);
        if has_writable_request(&status, a.0, a.1)
            && has_writable_request(&status, b.0, b.1)
            && status["terminal"]["rows"] == u64::from(effective.0)
            && status["terminal"]["cols"] == u64::from(effective.1)
        {
            return status;
        }
    }
}

fn send_concurrently(a: &mut Conn, a_size: (u16, u16), b: &mut Conn, b_size: (u16, u16)) {
    let barrier = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        let a_barrier = Arc::clone(&barrier);
        scope.spawn(move || {
            a_barrier.wait();
            a.resize(a_size.0, a_size.1);
        });
        let b_barrier = Arc::clone(&barrier);
        scope.spawn(move || {
            b_barrier.wait();
            b.resize(b_size.0, b_size.1);
        });
        barrier.wait();
    });
}

#[test]
fn concurrent_resize_storm_orders_geometry_and_preserves_min_wins_status() {
    let rig = Rig::new();
    let id = unique_id("storm");
    let daemon = rig.daemon(
        &id,
        &["sh", "-c", REPORTER],
        DaemonOpts::no_display_name().ephemeral(),
    );
    let pid = daemon.pid();

    let mut a = rig.connect(&id);
    a.attach(60, 180);
    a.wait_for(MessageType::Screen, operation_budget())
        .expect("writer A SCREEN");
    let mut b = rig.connect(&id);
    b.attach(60, 180);
    b.wait_for(MessageType::Screen, operation_budget())
        .expect("writer B SCREEN");
    let mut observer = rig.connect(&id);
    observer.peek(false, false);
    observer
        .wait_for(MessageType::Screen, operation_budget())
        .expect("read-only observer SCREEN");

    let mut final_status = Value::Null;
    for round in 0..ROUNDS {
        let (a_size, b_size) = if round % 2 == 0 {
            ((40, 120), (30, 100))
        } else {
            ((28, 90), (36, 110))
        };
        let effective = (a_size.0.min(b_size.0), a_size.1.min(b_size.1));
        send_concurrently(&mut a, a_size, &mut b, b_size);
        final_status = wait_for_arbitrated_status(&rig, &id, a_size, b_size, effective);
        a.data(b"REPORT\n");
        collect_affected_output(&mut observer, effective.0, effective.1);
    }

    assert_eq!(final_status["terminal"]["rows"], 28, "{final_status}");
    assert_eq!(final_status["terminal"]["cols"], 90, "{final_status}");
    assert!(
        has_writable_request(&final_status, 28, 90),
        "{final_status}"
    );
    assert!(
        has_writable_request(&final_status, 36, 110),
        "{final_status}"
    );

    drop(observer);
    drop(a);
    drop(b);
    let stopped = rig.pty(&["kill", &id]);
    expect_status(&stopped, 0);
    wait_for_process_gone(pid);
    wait_until_for(
        "resize-storm session files to be removed",
        operation_budget(),
        &mut || {
            !rig.meta_path(&id).exists()
                && !rig.socket_path(&id).exists()
                && !rig.pid_path(&id).exists()
        },
    );
}
