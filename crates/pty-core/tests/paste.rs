//! Port of the bracketed-paste helper coverage from the pty project's
//! `send-paste.test.ts` (the wrapping logic; the CLI wiring is exercised by
//! `cli_e2e`).

use pty_core::paste::{wrap_bracketed_paste, BRACKETED_PASTE_END, BRACKETED_PASTE_START};

#[test]
fn markers_are_csi_200_and_201() {
    assert_eq!(BRACKETED_PASTE_START, "\x1b[200~");
    assert_eq!(BRACKETED_PASTE_END, "\x1b[201~");
}

#[test]
fn wraps_payload_in_start_and_end() {
    let wrapped = wrap_bracketed_paste(b"hello world");
    assert_eq!(wrapped, b"\x1b[200~hello world\x1b[201~");
}

#[test]
fn wraps_multiline_payload_as_one_block() {
    let payload = "line 1\nline 2\nline 3";
    let wrapped = wrap_bracketed_paste(payload.as_bytes());
    let s = String::from_utf8(wrapped).unwrap();
    assert!(s.starts_with(BRACKETED_PASTE_START));
    assert!(s.ends_with(BRACKETED_PASTE_END));
    assert!(s.contains("line 1\nline 2\nline 3"));
}

#[test]
fn wraps_empty_payload() {
    assert_eq!(wrap_bracketed_paste(b""), b"\x1b[200~\x1b[201~");
}
