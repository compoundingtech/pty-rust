//! Liveness semantics of `registry::list_sessions().alive`, locking in the
//! node #117 read-layer contract: a session is reported NOT alive ONLY on
//! POSITIVE proof of death — a readable pid whose process is gone AND an
//! unreachable control socket. A live pid ⇒ alive; a reachable socket ⇒ alive
//! (even when the pid looks dead); an unreadable/absent pid with no reachable
//! socket is INDETERMINATE (e.g. mid-launch, before the pid file lands) and is
//! reported running, never reaped. This is the destroy-a-live-daemon race guard
//! on the read side — see the heads-up that prompted it.
//!
//! One test per binary on purpose: it sets `PTY_ROOT` for THIS process, and a
//! single test means no intra-binary parallelism can race that env. Other test
//! files compile to separate processes, so their `PTY_ROOT` is unaffected.

use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use pty_core::registry::{self, SessionMetadata};

/// A pid far above Linux's `pid_max`, so no live process can own it → `kill(pid,
/// 0)` yields ESRCH → treated as dead. Asserted at runtime below so the test
/// fails loudly rather than silently if that assumption ever breaks.
const DEAD_PID: i32 = i32::MAX;

fn meta(exit_code: Option<i32>) -> SessionMetadata {
    SessionMetadata {
        command: "sh".into(),
        args: vec![],
        display_command: "sh".into(),
        cwd: "/".into(),
        created_at: "1970-01-01T00:00:00.000Z".into(),
        exit_code,
        exited_at: None,
        last_lines: None,
        tags: None,
        display_name: None,
        last_attach_at: None,
    }
}

fn plant(name: &str, m: &SessionMetadata) {
    registry::write_metadata(name, m).expect("write metadata");
}

fn alive_of(name: &str) -> bool {
    registry::list_sessions()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("session {name} not listed"))
        .alive
}

#[test]
fn liveness_follows_node_117_positive_death_proof() {
    // Isolated registry root for this process only.
    let root: PathBuf = std::env::temp_dir().join(format!("pty-liveness-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    // SAFETY: single test in this binary → no other thread reads/writes env
    // concurrently; other test binaries are separate processes.
    unsafe {
        std::env::set_var("PTY_ROOT", &root);
    }

    // Preconditions: our chosen pids really are dead / alive as assumed.
    assert!(
        !registry::pid_alive(DEAD_PID),
        "DEAD_PID unexpectedly looks alive; pick a higher pid"
    );
    assert!(
        registry::pid_alive(std::process::id() as i32),
        "our own pid should look alive"
    );

    // Case: mid-launch — metadata exists, pid file not yet written, no socket.
    // INDETERMINATE ⇒ reported running (this is the transient window node #117
    // fixed; the old pid-only rule wrongly reported this dead).
    plant("launching", &meta(None));
    assert!(
        alive_of("launching"),
        "a launching session (no pid yet, no socket) must report alive"
    );

    // Case: live pid, no socket bound yet — process is up ⇒ alive.
    plant("live-pid", &meta(None));
    std::fs::write(registry::pid_path("live-pid"), std::process::id().to_string()).unwrap();
    assert!(alive_of("live-pid"), "a live daemon pid must report alive");

    // Case: pid LOOKS dead, but the control socket is reachable ⇒ alive. This is
    // the #117 rescue: a reachable socket overrides a dead-looking/stale pid.
    plant("socket-rescue", &meta(None));
    std::fs::write(registry::pid_path("socket-rescue"), DEAD_PID.to_string()).unwrap();
    let _listener =
        UnixListener::bind(registry::socket_path("socket-rescue")).expect("bind control socket");
    assert!(
        alive_of("socket-rescue"),
        "a reachable socket must report alive even when the pid looks dead"
    );

    // Case: positive proof of death — readable pid whose process is gone AND no
    // reachable socket ⇒ dead.
    plant("truly-dead", &meta(None));
    std::fs::write(registry::pid_path("truly-dead"), DEAD_PID.to_string()).unwrap();
    assert!(
        !alive_of("truly-dead"),
        "a dead pid with no reachable socket must report NOT alive"
    );

    // Case: preserved exited session — exit metadata recorded ⇒ not alive,
    // regardless of pid/socket.
    plant("exited", &meta(Some(0)));
    assert!(
        !alive_of("exited"),
        "an exited (exit_code recorded) session must report NOT alive"
    );

    let _ = std::fs::remove_dir_all(&root);
}
