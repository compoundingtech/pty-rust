//! Loaders for the Rust-owned protocol fixtures in `fixtures/*.json`
//! (issue #4 in the parity plan): bytes and escape sequences split across
//! reads, raw (non-UTF-8) DATA, attach identity across a replacement, late
//! frames from an old socket, framing limits, and a slow reader. These talk
//! to the daemon directly over its socket through `Rig::connect`.
//!
//! node: (no Node test file; the fixtures are new and shared with the TS
//! testing package)

use pty_conformance::*;
use pty_core::protocol::{MessageType, PacketReader, encode_data};
use serde_json::Value;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fixture(name: &str) -> Value {
    let raw = std::fs::read_to_string(fixtures_dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// A `sh` script that prints `bytes` one byte at a time with a pause, then
/// `after`, then keeps the session alive with `cat`. The leading sleep gives
/// a client time to attach before the first byte.
fn byte_by_byte_script(bytes: &[u8], after: &str) -> String {
    let mut s = String::from("sleep 0.3; ");
    for b in bytes {
        s.push_str(&format!("printf '\\{b:03o}'; sleep 0.05; "));
    }
    if !after.is_empty() {
        let mut lit = String::new();
        for b in after.bytes() {
            lit.push_str(&format!("\\{b:03o}"));
        }
        s.push_str(&format!("printf '{lit}'; "));
    }
    s.push_str("exec cat");
    s
}

fn start_dump_session(rig: &Rig, id: &str) -> PathBuf {
    let dump = rig.root().join(format!("{id}.dump.bin"));
    let script = format!("stty raw -echo; cat > '{}'", dump.display());
    rig.daemon(id, &["sh", "-c", &script], DaemonOpts::no_display_name());
    std::thread::sleep(Duration::from_millis(150));
    dump
}

fn wait_for_dump(dump: &std::path::Path, min_len: usize, timeout: Duration) -> Vec<u8> {
    let _ = poll_for(timeout, || std::fs::read(dump).map(|b| b.len() >= min_len).unwrap_or(false));
    std::fs::read(dump).unwrap_or_default()
}

fn plain_screen(rig: &Rig, id: &str) -> String {
    let out = rig.pty(&["peek", "--plain", id]);
    expect_status(&out, 0);
    out.stdout()
}

// ── bytes-split ──

/// Output direction: the child prints each byte separately; the terminal
/// reassembles the scalar and attached clients see the same bytes.
#[test]
fn bytes_split_output_reassembles_every_scalar() {
    let doc = fixture("bytes-split.json");
    for case in doc["cases"].as_array().unwrap() {
        if case["direction"] != "output" {
            continue;
        }
        let id = case["id"].as_str().unwrap();
        let sample = case["sample"].as_str().unwrap();
        let expect_plain = case["expectPlain"].as_str().unwrap();
        let rig = Rig::new();
        let script = byte_by_byte_script(sample.as_bytes(), "");
        rig.daemon(id, &["sh", "-c", &script], DaemonOpts::no_display_name());
        let mut conn = rig.connect(id);
        conn.attach(24, 80);
        let packets = conn.drain(Duration::from_millis(1500));
        let data = data_bytes(&packets);
        assert!(
            data.windows(sample.len()).any(|w| w == sample.as_bytes()),
            "[{id}] DATA frames do not concatenate to the sample: {:?}",
            String::from_utf8_lossy(&data)
        );
        let screen = plain_screen(&rig, id);
        assert_eq!(screen.trim_end(), expect_plain, "[{id}] plain screen");
    }
}

/// Input direction, Rust half: each byte of the sample in its own DATA
/// frame reaches the child intact (docs/decisions/0001-raw-data-bytes.md).
#[test]
fn bytes_split_input_reassembles_every_scalar_rust() {
    if !is_rust() {
        return;
    }
    let doc = fixture("bytes-split.json");
    for case in doc["cases"].as_array().unwrap() {
        if case["direction"] != "input" {
            continue;
        }
        let id = case["id"].as_str().unwrap();
        let sample = case["sample"].as_str().unwrap();
        let expect_plain = case["expectPlain"].as_str().unwrap();
        let rig = Rig::new();
        rig.daemon(id, &["cat"], DaemonOpts::no_display_name());
        let mut conn = rig.connect(id);
        conn.attach(24, 80);
        let _ = conn.drain(Duration::from_millis(300));
        for b in sample.as_bytes() {
            conn.data(&[*b]);
            std::thread::sleep(Duration::from_millis(30));
        }
        let expected = expect_plain.to_string();
        wait_until(&format!("[{id}] echoed sample on screen"), || {
            plain_screen(&rig, id).contains(&expected)
        });
    }
}

/// Input direction, Node half: Node decodes each DATA payload as its own
/// UTF-8 string, so a scalar split across frames never reaches the child
/// intact (docs/decisions/0001-raw-data-bytes.md).
#[test]
fn bytes_split_input_is_mangled_node() {
    if !is_node() {
        return;
    }
    let doc = fixture("bytes-split.json");
    for case in doc["cases"].as_array().unwrap() {
        if case["direction"] != "input" {
            continue;
        }
        assert_eq!(case["node"], "mangled", "fixture records the Node behavior");
        let id = case["id"].as_str().unwrap();
        let sample = case["sample"].as_str().unwrap();
        let rig = Rig::new();
        rig.daemon(id, &["cat"], DaemonOpts::no_display_name());
        let mut conn = rig.connect(id);
        conn.attach(24, 80);
        let _ = conn.drain(Duration::from_millis(300));
        for b in sample.as_bytes() {
            conn.data(&[*b]);
            std::thread::sleep(Duration::from_millis(30));
        }
        // Sentinel so the screen has settled.
        conn.data(b"|END");
        wait_until(&format!("[{id}] sentinel on screen"), || plain_screen(&rig, id).contains("|END"));
        let screen = plain_screen(&rig, id);
        assert!(!screen.contains(sample), "[{id}] Node delivered a split scalar intact: {screen:?}");
    }
}

// ── escape-split ──

#[test]
fn escape_split_sequences_parse_across_reads() {
    let doc = fixture("escape-split.json");
    for case in doc["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let mut seq = vec![0x1bu8];
        seq.extend_from_slice(case["sequence"].as_str().unwrap().as_bytes());
        if case["sequence"].as_str().unwrap().starts_with(']') {
            seq.push(0x07);
        }
        let after = case["after"].as_str().unwrap();
        let expect_plain = case["expectPlain"].as_str().unwrap();
        let rig = Rig::new();
        let script = byte_by_byte_script(&seq, after);
        rig.daemon(id, &["sh", "-c", &script], DaemonOpts::no_display_name());
        let expected = after.to_string();
        wait_until(&format!("[{id}] text after the sequence"), || plain_screen(&rig, id).contains(&expected));
        let screen = plain_screen(&rig, id);
        assert_eq!(screen.trim_end(), expect_plain, "[{id}] plain screen");
        if case["expectAltScreen"].as_bool() == Some(true) {
            let mut conn = rig.connect(id);
            conn.attach(24, 80);
            let screen = conn.wait_for(MessageType::Screen, deadline()).expect("SCREEN");
            assert!(
                screen.payload.starts_with(b"\x1b[?1049h"),
                "[{id}] SCREEN does not start with the alt-screen prefix: {:?}",
                String::from_utf8_lossy(&screen.payload[..screen.payload.len().min(40)])
            );
        }
    }
}

// ── raw-bytes ──

fn raw_bytes_dump(rig: &Rig) -> (Vec<u8>, Value) {
    let doc = fixture("raw-bytes.json");
    let payload = parse_hex(doc["payloadHex"].as_str().unwrap());
    let dump = start_dump_session(rig, "raw");
    let mut conn = rig.connect("raw");
    conn.data(&payload);
    conn.shutdown_write();
    let got = wait_for_dump(&dump, payload.len(), Duration::from_secs(3));
    (got, doc)
}

/// Node half: invalid UTF-8 is re-encoded as U+FFFD per byte
/// (docs/decisions/0001-raw-data-bytes.md).
#[test]
fn raw_data_bytes_node() {
    if !is_node() {
        return;
    }
    let rig = Rig::new();
    let (got, doc) = raw_bytes_dump(&rig);
    assert_eq!(to_hex(&got), doc["expect"]["node"]["dumpHex"].as_str().unwrap());
}

/// Rust half: the bytes are written through unchanged
/// (docs/decisions/0001-raw-data-bytes.md).
#[test]
fn raw_data_bytes_rust() {
    if !is_rust() {
        return;
    }
    let rig = Rig::new();
    let (got, doc) = raw_bytes_dump(&rig);
    assert_eq!(to_hex(&got), doc["expect"]["rust"]["dumpHex"].as_str().unwrap());
}

// ── attach-identity ──

#[test]
fn attach_reaches_the_replacement_daemon() {
    let doc = fixture("attach-identity.json");
    let id = doc["id"].as_str().unwrap();
    let cmd: Vec<&str> = doc["command"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    let rig = Rig::new();
    rig.daemon(id, &cmd, DaemonOpts::no_display_name());
    let mut c1 = rig.connect(id);
    c1.attach(24, 80);
    c1.wait_for(MessageType::Screen, deadline()).expect("first SCREEN");
    let pid1 = c1.status_json(deadline())["daemon"]["pid"].as_i64().unwrap();
    drop(c1);
    expect_status(&rig.pty(&["kill", id]), 0);
    expect_status(&rig.pty(&["rm", id]), 0);
    rig.daemon(id, &cmd, DaemonOpts::no_display_name());
    let mut c2 = rig.connect(id);
    c2.attach(24, 80);
    c2.wait_for(MessageType::Screen, deadline()).expect("second SCREEN");
    let starts: Vec<&str> = doc["expect"]["secondSequenceStartsWith"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let seq = sequence_names(&c2.sequence());
    assert_eq!(&seq[..starts.len()], &starts[..], "second attach sequence {seq:?}");
    let pid2 = c2.status_json(deadline())["daemon"]["pid"].as_i64().unwrap();
    assert_ne!(pid1, pid2, "replacement daemon pid");
    assert!(pid_alive(pid2 as i32));
}

// ── late-events ──

#[test]
fn old_socket_frames_do_not_leak_into_a_new_attach() {
    let doc = fixture("late-events.json");
    let id = doc["id"].as_str().unwrap();
    let marker = doc["marker"].as_str().unwrap();
    let cmd: Vec<&str> = doc["command"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    let rig = Rig::new();
    rig.daemon(id, &cmd, DaemonOpts::no_display_name());
    let mut old = rig.connect(id);
    old.attach(24, 80);
    old.wait_for(MessageType::Screen, deadline()).expect("old SCREEN");
    let _ = old.drain(Duration::from_millis(200));

    expect_status(&rig.pty(&["kill", id]), 0);
    expect_status(&rig.pty(&["rm", id]), 0);
    let script = format!("echo {marker}; exec cat");
    rig.daemon(id, &["sh", "-c", &script], DaemonOpts::no_display_name());

    let mut new = rig.connect(id);
    new.attach(24, 80);
    let screen = new.wait_for(MessageType::Screen, deadline()).expect("new SCREEN");
    let starts: Vec<&str> = doc["expect"]["newSequenceStartsWith"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let seq = sequence_names(&new.sequence());
    assert_eq!(&seq[..starts.len()], &starts[..], "new attach sequence {seq:?}");
    assert!(
        String::from_utf8_lossy(&screen.payload).contains(marker),
        "replacement SCREEN lacks the marker"
    );

    // The old socket only ever sees its own daemon's end.
    let tail = old.drain(Duration::from_millis(500));
    for p in &tail {
        assert!(
            !String::from_utf8_lossy(&p.payload).contains(marker),
            "old socket received a replacement frame: {:?}",
            type_name(p.type_)
        );
    }
    assert!(
        tail.iter().any(|p| p.type_ == MessageType::Exit) || old.is_eof(),
        "old socket neither got EXIT nor closed: {:?}",
        sequence_names(&old.sequence())
    );
}

// ── frame-limits ──

#[test]
fn a_large_data_frame_arrives_intact() {
    let doc = fixture("frame-limits.json");
    let n = doc["intactBytes"].as_u64().unwrap() as usize;
    let payload: Vec<u8> = (0..n).map(|i| b'a' + (i % 26) as u8).collect();
    let rig = Rig::new();
    let dump = start_dump_session(&rig, "big");
    let mut conn = rig.connect("big");
    conn.data(&payload);
    conn.shutdown_write();
    let got = wait_for_dump(&dump, n, Duration::from_secs(10));
    assert_eq!(got.len(), n, "dump length");
    assert!(got == payload, "dump content differs");
}

#[test]
fn an_oversized_declared_length_drops_the_connection() {
    let doc = fixture("frame-limits.json");
    let declared = doc["oversizedDeclaredLength"].as_u64().unwrap() as u32;
    let mut header = vec![MessageType::Data.as_u8()];
    header.extend_from_slice(&declared.to_be_bytes());

    // Client side: the reader refuses the header outright.
    let mut reader = PacketReader::new();
    let err = reader.feed(&header).expect_err("oversized header is rejected");
    assert_eq!(err.kind(), ErrorKind::InvalidData);

    // Server side: the daemon closes the socket.
    let rig = Rig::new();
    rig.daemon("over", &["cat"], DaemonOpts::no_display_name());
    let mut conn = rig.connect("over");
    conn.attach(24, 80);
    conn.wait_for(MessageType::Screen, deadline()).expect("SCREEN");
    conn.write_raw(&header).expect("write header");
    let start = Instant::now();
    let mut closed = false;
    while start.elapsed() < deadline() {
        match conn.next_packet_result(Duration::from_millis(200)) {
            Ok(None) if conn.is_eof() => {
                closed = true;
                break;
            }
            Ok(_) => {}
            Err(_) => {
                closed = true;
                break;
            }
        }
        // A write failing with EPIPE/ECONNRESET also proves the drop.
        if conn.write_raw(b"").is_err() {
            closed = true;
            break;
        }
    }
    assert!(closed, "daemon kept the connection open after an oversized header");
    // The daemon itself is unharmed.
    let mut again = rig.connect("over");
    again.attach(24, 80);
    again.wait_for(MessageType::Screen, deadline()).expect("SCREEN after the drop");
}

#[test]
fn three_frames_in_one_write_arrive_in_order() {
    let doc = fixture("frame-limits.json");
    let parts: Vec<&str> = doc["coalesced"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    let expected = doc["expect"]["coalescedDump"].as_str().unwrap();
    let rig = Rig::new();
    let dump = start_dump_session(&rig, "coal");
    let mut conn = rig.connect("coal");
    let mut buf = Vec::new();
    for p in &parts {
        buf.extend(encode_data(p.as_bytes()));
    }
    conn.write_raw(&buf).unwrap();
    conn.shutdown_write();
    let got = wait_for_dump(&dump, expected.len(), Duration::from_secs(3));
    assert_eq!(String::from_utf8_lossy(&got), expected);
}

// ── slow-reader ──

#[test]
fn a_client_that_never_reads_does_not_starve_the_others() {
    let doc = fixture("slow-reader.json");
    let id = doc["id"].as_str().unwrap();
    let total = doc["outputBytes"].as_u64().unwrap() as usize;
    let within = Duration::from_millis(doc["deliveryWithinMs"].as_u64().unwrap());
    let total_within = Duration::from_millis(doc["totalWithinMs"].as_u64().unwrap());
    let rig = Rig::new();
    let script = format!("sleep 0.5; head -c {total} /dev/zero | tr '\\0' x; exec cat");
    rig.daemon(id, &["sh", "-c", &script], DaemonOpts::no_display_name());

    // The slow reader: attach and never read again.
    let slow = std::os::unix::net::UnixStream::connect(rig.socket_path(id)).unwrap();
    (&slow).write_all(&pty_core::protocol::encode_attach(24, 80)).unwrap();

    let mut fast = rig.connect(id);
    fast.attach(24, 80);
    fast.wait_for(MessageType::Screen, deadline()).expect("SCREEN");
    let start = Instant::now();
    let first = fast.wait_for(MessageType::Data, Duration::from_millis(500) + within);
    assert!(first.is_some(), "no DATA reached the reading client within {within:?}");
    let mut received = first.unwrap().payload.len();
    while received < total && start.elapsed() < total_within {
        match fast.next_packet(Duration::from_secs(2)) {
            Some(p) if p.type_ == MessageType::Data => received += p.payload.len(),
            Some(_) => {}
            None => break,
        }
    }
    assert!(received >= total, "reading client got {received} of {total} bytes in {:?}", start.elapsed());
    // Keep the slow socket alive until here so its buffer stayed full.
    let mut probe = [0u8; 1];
    let _ = (&slow).set_read_timeout(Some(Duration::from_millis(10)));
    let _ = (&slow).read(&mut probe);
    drop(slow);
}
