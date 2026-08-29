//! `SessionConnection` / `peek_screen` against a fake daemon (port of the
//! parts of `tests/connection.test.ts` that pin the wire behavior), plus the
//! tokio flavour when the `tokio` feature is on.

mod common;

use std::time::Duration;

use common::*;
use pty_core::client::{
    ClientError, PeekScreenOptions, SessionConnection, SessionEvent, peek_screen,
};
use pty_core::protocol::{
    MessageType, PacketReader, decode_size, encode_data, encode_exit, encode_geometry,
    encode_screen,
};

const T: Duration = Duration::from_secs(5);

/// node: tests/connection.test.ts:102-202
#[test]
fn connect_attaches_resolves_on_screen_and_tracks_geometry() {
    let d = FakeDaemon::bind("conn");
    let listener = d.listener.try_clone().unwrap();
    let h = std::thread::spawn(move || {
        use std::io::Write;
        let (mut s, _) = listener.accept().unwrap();
        let mut reader = PacketReader::new();
        let first = read_until(&mut s, &mut reader, MessageType::Attach, T);
        assert_eq!(decode_size(&first[0].payload), (30, 100));
        s.write_all(&concat(&[
            encode_geometry(24, 80),
            encode_screen(b"$ "),
            encode_data(b"hello"),
        ]))
        .unwrap();
        let got = read_until(&mut s, &mut reader, MessageType::Resize, T);
        assert_eq!(got[0].type_, MessageType::Data);
        assert_eq!(got[0].payload, b"typed");
        assert_eq!(got[1].payload, b"\r");
        assert_eq!(decode_size(&got[2].payload), (40, 120));
        s.write_all(&encode_exit(9)).unwrap();
        let rest = read_until(&mut s, &mut reader, MessageType::Detach, T);
        assert_eq!(rest.last().unwrap().type_, MessageType::Detach);
    });
    let mut c = SessionConnection::connect(&d.name, 30, 100).unwrap();
    assert!(c.connected());
    assert_eq!(c.screen(), b"$ ");
    assert_eq!((c.effective_rows(), c.effective_cols()), (24, 80));
    assert_eq!((c.rows(), c.cols()), (30, 100));
    assert_eq!(
        c.next_event(Some(T)).unwrap(),
        Some(SessionEvent::Data(b"hello".to_vec()))
    );
    c.write(b"typed");
    c.press("return").unwrap();
    c.resize(40, 120);
    assert_eq!((c.rows(), c.cols()), (40, 120));
    assert_eq!(c.next_event(Some(T)).unwrap(), Some(SessionEvent::Exit(9)));
    assert_eq!(c.next_event(Some(Duration::from_millis(50))).unwrap(), None);
    c.disconnect();
    assert!(!c.connected());
    assert_eq!(c.next_event(Some(T)).unwrap(), Some(SessionEvent::Closed));
    h.join().unwrap();
}

/// node: connection.ts:157-165
#[test]
fn close_before_screen_and_missing_session() {
    let d = FakeDaemon::bind("conn-close");
    let listener = d.listener.try_clone().unwrap();
    let h = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let _ = read_chunk(&mut s);
        drop(s);
    });
    let err = SessionConnection::connect(&d.name, 24, 80).unwrap_err();
    assert_eq!(err, ClientError::ClosedBeforeScreen(d.name.clone()));
    assert_eq!(
        err.to_string(),
        format!(
            "Connection to \"{}\" closed before screen received.",
            d.name
        )
    );
    h.join().unwrap();
    test_root();
    let err = SessionConnection::connect("absent", 24, 80).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Session \"absent\" not found or not running."
    );
}

/// node: tests/connection.test.ts:318-349
#[test]
fn peek_screen_returns_the_first_screen() {
    let d = FakeDaemon::bind("ps");
    let listener = d.listener.try_clone().unwrap();
    let h = std::thread::spawn(move || {
        use std::io::Write;
        let (mut s, _) = listener.accept().unwrap();
        let first = packets(&read_chunk(&mut s));
        assert_eq!(first[0].type_, MessageType::Peek);
        assert_eq!(first[0].payload, vec![1]);
        s.write_all(&encode_screen(b"plain text")).unwrap();
        let _ = read_packets_until_eof(&mut s, T);
    });
    let got = peek_screen(
        &d.name,
        PeekScreenOptions {
            plain: true,
            full: false,
        },
    )
    .unwrap();
    assert_eq!(got, "plain text");
    assert!(!got.contains("\x1b["));
    h.join().unwrap();
}

#[cfg(feature = "tokio")]
mod tokio_flavour {
    use super::*;
    use pty_core::client::AsyncConnection;

    /// The async connection speaks the same protocol as the sync one.
    #[tokio::test]
    async fn async_connection_round_trip() {
        let d = FakeDaemon::bind("aconn");
        let listener = d.listener.try_clone().unwrap();
        let h = std::thread::spawn(move || {
            use std::io::Write;
            let (mut s, _) = listener.accept().unwrap();
            let mut reader = PacketReader::new();
            let first = read_until(&mut s, &mut reader, MessageType::Attach, T);
            assert_eq!(decode_size(&first[0].payload), (10, 20));
            s.write_all(&concat(&[encode_geometry(10, 20), encode_screen(b"async")]))
                .unwrap();
            let got = read_until(&mut s, &mut reader, MessageType::Data, T);
            assert_eq!(got.last().unwrap().payload, b"\x03");
            s.write_all(&encode_exit(1)).unwrap();
            let _ = read_until(&mut s, &mut reader, MessageType::Detach, T);
        });
        let name = d.name.clone();
        let mut c = AsyncConnection::connect(&name, 10, 20).await.unwrap();
        assert_eq!(c.screen(), b"async");
        assert_eq!((c.effective_rows(), c.effective_cols()), (10, 20));
        c.press("ctrl+c").await.unwrap();
        assert_eq!(c.next_event().await.unwrap(), SessionEvent::Exit(1));
        c.disconnect().await;
        assert_eq!(c.next_event().await.unwrap(), SessionEvent::Closed);
        tokio::task::spawn_blocking(move || h.join().unwrap())
            .await
            .unwrap();
    }
}
