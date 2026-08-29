# 0001 Raw DATA bytes

Status: accepted

Node behavior: the daemon decodes every inbound DATA payload as a UTF-8
string before writing it to the pty (`this.ptyProcess.write(packet.payload.toString())`,
src/server.ts:1024). Two consequences:

- A byte that is not valid UTF-8 (0x80–0xff outside a well-formed sequence)
  is replaced by U+FFFD, so the child receives `EF BF BD` for each such byte.
- A multi-byte scalar split across two DATA frames is decoded per frame and
  each fragment becomes U+FFFD; the child never sees the scalar.

Rust behavior: the daemon writes the DATA payload bytes to the pty
unchanged. Invalid UTF-8 reaches the child as-is; a scalar split across
frames is reassembled by the pty stream itself (the bytes are contiguous on
the pty), so the child sees the scalar.

Why: DATA is bytes on the wire (`[type][len][payload]`); re-encoding is an
artifact of Node's string-first pty API, not a protocol rule. Writing bytes
through is what every other terminal multiplexer does, it is required for
binary-clean input (mouse reports, pastes of arbitrary bytes, a child that
switches to a non-UTF-8 locale), and it removes a class of corruption for
clients that frame input at arbitrary boundaries. There is no way to hide
the difference at the CLI: `pty send` always sends valid UTF-8, so the
difference is only observable to a raw socket client.

Client effect: only a client that sends invalid UTF-8, or splits a scalar
across DATA frames, can tell the two apart. `pty send`, `pty attach`, and
the testing libraries never do either. A client that relied on Node's
sanitizing (it received `EF BF BD` for a stray byte) now gets the byte.

Test: crates/pty-conformance/tests/fixtures_protocol.rs::raw_data_bytes_node /
raw_data_bytes_rust (gated), and
bytes_split_input_is_mangled_node / bytes_split_input_reassembles_every_scalar_rust
(gated) — fixtures crates/pty-conformance/fixtures/raw-bytes.json and
crates/pty-conformance/fixtures/bytes-split.json (the `input` cases record
`node: "mangled"`).

Migration / negotiation: none. No wire change; clients keep sending DATA.
