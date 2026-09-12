//! Positive counterpart to the socket-path overflow cases: a real daemon can
//! bind and serve the longest combined `<PTY_ROOT>/<id>.sock` path supported
//! by the current platform.

use pty_conformance::*;
use pty_core::protocol::MessageType;
use std::time::Duration;

const SOCKET_PATH_LIMIT: usize = pty_core::registry::SUN_PATH_MAX;

fn operation_budget() -> Duration {
    if unoptimized_binary_under_test() {
        deadline() * 2
    } else {
        deadline()
    }
}

#[test]
fn longest_platform_socket_path_starts_and_serves_a_session() {
    let rig = Rig::new();
    let overhead = rig.root().as_os_str().as_encoded_bytes().len() + 1 + ".sock".len();
    assert!(
        overhead < SOCKET_PATH_LIMIT,
        "test PTY_ROOT is too long: {}",
        rig.root().display()
    );
    let id = "b".repeat(SOCKET_PATH_LIMIT - overhead);
    let socket = rig.socket_path(&id);
    assert_eq!(
        socket.as_os_str().as_encoded_bytes().len(),
        SOCKET_PATH_LIMIT,
        "{}",
        socket.display()
    );

    let daemon = rig.daemon(&id, &["cat"], DaemonOpts::no_display_name().ephemeral());
    let pid = daemon.pid();
    let mut client = rig.connect(&id);
    client.attach(24, 80);
    client
        .wait_for(MessageType::Screen, operation_budget())
        .unwrap_or_else(|| panic!("maximal socket {} did not serve ATTACH", socket.display()));
    client.detach();
    wait_until_for(
        "maximal-path client detach",
        operation_budget(),
        &mut || {
            let _ = client.next_packet(Duration::from_millis(20));
            client.is_eof()
        },
    );
    drop(client);

    let stopped = rig.pty(&["kill", &id]);
    expect_status(&stopped, 0);
    wait_for_process_gone(pid);
    wait_until_for(
        "maximal-path session files to be removed",
        operation_budget(),
        &mut || !rig.meta_path(&id).exists() && !socket.exists() && !rig.pid_path(&id).exists(),
    );
}
