//! Public contracts for observing a registry selected by path instead of the
//! process environment.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use pty_core::client::{
    PeekScreenOptions, peek_screen_bytes_in, peek_screen_in, query_stats_in,
    query_stats_in_with_timeout,
};
use pty_core::events::{read_all_events_in, read_recent_events_in};
use pty_core::protocol::{
    MessageType, Packet, PacketReader, decode_peek, encode_screen, encode_status_response,
};
use pty_core::registry::{ListOptions, list_sessions_in};
use serde_json::json;

const T: Duration = Duration::from_secs(5);
static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let serial = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("ptyobs-{}-{serial}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create explicit test root");
        TestRoot(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn session_file(&self, name: &str, suffix: &str) -> PathBuf {
        self.0.join(format!("{name}.{suffix}"))
    }

    fn listen(&self, name: &str) -> UnixListener {
        UnixListener::bind(self.session_file(name, "sock")).expect("bind test session socket")
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read_packet(stream: &mut UnixStream) -> Packet {
    let mut reader = PacketReader::new();
    let mut buf = [0_u8; 1024];
    loop {
        let n = stream.read(&mut buf).expect("read request packet");
        assert!(n > 0, "client closed before sending a packet");
        let mut packets = reader.feed(&buf[..n]).expect("valid request packet");
        if !packets.is_empty() {
            return packets.remove(0);
        }
    }
}

fn serve_once(listener: UnixListener, response: Vec<u8>) -> JoinHandle<Packet> {
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept client");
        let packet = read_packet(&mut stream);
        stream.write_all(&response).expect("write response");
        packet
    })
}

fn metadata(name: &str) -> String {
    json!({
        "command": "cat",
        "args": [],
        "displayCommand": "cat",
        "cwd": "/tmp",
        "createdAt": "2026-09-13T00:00:00.000Z",
        "displayName": name,
    })
    .to_string()
}

fn stats_body(name: &str) -> String {
    json!({
        "name": name,
        "terminal": {
            "cols": 80,
            "rows": 24,
            "cursorX": 0,
            "cursorY": 0,
            "scrollbackUsed": 0,
            "scrollbackCapacity": 10000,
        },
        "process": {
            "alive": true,
            "exitCode": null,
            "pid": 100,
            "resources": null,
        },
        "daemon": {
            "pid": 200,
            "resources": null,
        },
        "clients": {
            "total": 0,
            "attached": 0,
            "readOnly": 0,
        },
        "modes": {
            "sgrMouse": false,
            "cursorHidden": false,
            "kittyKeyboard": false,
            "kittyKeyboardFlags": [],
        },
        "uptimeSeconds": 1,
        "createdAt": "2026-09-13T00:00:00.000Z",
    })
    .to_string()
}

#[test]
fn listing_keeps_explicit_roots_independent() {
    let left = TestRoot::new();
    let right = TestRoot::new();
    std::fs::write(left.session_file("left", "json"), metadata("left")).unwrap();
    std::fs::write(right.session_file("right", "json"), metadata("right")).unwrap();

    let left_sessions = list_sessions_in(left.path(), &ListOptions::default());
    let right_sessions = list_sessions_in(right.path(), &ListOptions::default());

    assert_eq!(
        left_sessions
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["left"]
    );
    assert_eq!(
        left_sessions[0].socket_path,
        left.session_file("left", "sock")
    );
    assert_eq!(
        right_sessions
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["right"]
    );
    assert_eq!(
        right_sessions[0].socket_path,
        right.session_file("right", "sock")
    );
}

#[test]
fn stats_resolve_the_session_socket_in_the_supplied_root() {
    let left = TestRoot::new();
    let right = TestRoot::new();
    let name = "same-name";
    let left_server = serve_once(
        left.listen(name),
        encode_status_response(&stats_body("from-left")),
    );
    let right_server = serve_once(
        right.listen(name),
        encode_status_response(&stats_body("from-right")),
    );

    let left_stats = query_stats_in(left.path(), name).unwrap();
    let right_stats = query_stats_in_with_timeout(right.path(), name, T).unwrap();

    assert_eq!(left_stats.name, "from-left");
    assert_eq!(right_stats.name, "from-right");
    assert_eq!(left_server.join().unwrap().type_, MessageType::Status);
    assert_eq!(right_server.join().unwrap().type_, MessageType::Status);
}

#[test]
fn explicit_peek_has_a_text_api_and_a_lossless_byte_api() {
    let text_root = TestRoot::new();
    let bytes_root = TestRoot::new();
    let name = "peek";
    let text_server = serve_once(text_root.listen(name), encode_screen(b"text screen"));
    let raw = b"before\xff\x80after\x00".to_vec();
    let bytes_server = serve_once(bytes_root.listen(name), encode_screen(&raw));
    let options = PeekScreenOptions {
        plain: false,
        full: true,
    };

    assert_eq!(
        peek_screen_in(text_root.path(), name, options).unwrap(),
        "text screen"
    );
    assert_eq!(
        peek_screen_bytes_in(bytes_root.path(), name, options).unwrap(),
        raw
    );

    let text_request = text_server.join().unwrap();
    let bytes_request = bytes_server.join().unwrap();
    assert_eq!(text_request.type_, MessageType::Peek);
    assert_eq!(bytes_request.type_, MessageType::Peek);
    assert_eq!(decode_peek(&text_request.payload), (false, true));
    assert_eq!(decode_peek(&bytes_request.payload), (false, true));
}

#[test]
fn full_peek_uses_retained_last_lines_when_the_socket_is_unavailable() {
    let root = TestRoot::new();
    let name = "retained";
    std::fs::write(
        root.session_file(name, "json"),
        json!({
            "command": "cat",
            "args": [],
            "displayCommand": "cat",
            "cwd": "/tmp",
            "createdAt": "2026-09-13T00:00:00.000Z",
            "exitedAt": "2026-09-13T00:00:01.000Z",
            "lastLines": ["café", "snowman ☃ and \"quoted\""],
        })
        .to_string(),
    )
    .unwrap();
    let options = PeekScreenOptions {
        plain: false,
        full: true,
    };
    let expected = b"caf\xc3\xa9\nsnowman \xe2\x98\x83 and \"quoted\"\n";

    assert_eq!(
        peek_screen_bytes_in(root.path(), name, options).unwrap(),
        expected
    );
    assert_eq!(
        peek_screen_in(root.path(), name, options)
            .unwrap()
            .as_bytes(),
        expected
    );
}

#[test]
fn full_peek_uses_retained_last_lines_when_the_socket_closes_before_screen() {
    let root = TestRoot::new();
    let name = "retained-race";
    std::fs::write(
        root.session_file(name, "json"),
        json!({
            "command": "cat",
            "args": [],
            "displayCommand": "cat",
            "cwd": "/tmp",
            "createdAt": "2026-09-13T00:00:00.000Z",
            "lastLines": ["saved output"],
        })
        .to_string(),
    )
    .unwrap();
    let server = std::thread::spawn({
        let listener = root.listen(name);
        move || {
            let (mut stream, _) = listener.accept().expect("accept client");
            assert_eq!(read_packet(&mut stream).type_, MessageType::Peek);
        }
    });

    assert_eq!(
        peek_screen_bytes_in(
            root.path(),
            name,
            PeekScreenOptions {
                plain: false,
                full: true,
            },
        )
        .unwrap(),
        b"saved output\n"
    );
    server.join().unwrap();
}

#[test]
fn event_readers_use_only_the_supplied_root() {
    let left = TestRoot::new();
    let right = TestRoot::new();
    let name = "same-name";
    let left_events = [
        json!({
            "session": name,
            "type": "user.left-first",
            "ts": "2026-09-13T00:00:00.000Z",
        }),
        json!({
            "session": name,
            "type": "user.left-last",
            "ts": "2026-09-13T00:00:01.000Z",
        }),
    ]
    .map(|event| event.to_string())
    .join("\n")
        + "\n";
    let right_events = json!({
        "session": name,
        "type": "user.right",
        "ts": "2026-09-13T00:00:02.000Z",
    })
    .to_string()
        + "\n";
    std::fs::write(left.session_file(name, "events.jsonl"), left_events).unwrap();
    std::fs::write(right.session_file(name, "events.jsonl"), right_events).unwrap();

    let recent = read_recent_events_in(left.path(), name, 1);
    let all = read_all_events_in(right.path(), name);

    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].r#type, "user.left-last");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].r#type, "user.right");
}
