//! Client-side operations against a session daemon: peek, send, status, and the
//! interactive attach loop.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use crate::protocol::{
    decode_exit, encode_attach, encode_data, encode_detach, encode_peek, encode_resize,
    encode_status, read_packet, MessageType, PacketReader,
};
use crate::registry;

/// Connect to a session's unix socket.
pub fn connect(name: &str) -> std::io::Result<UnixStream> {
    UnixStream::connect(registry::socket_path(name))
}

/// Peek: request the current screen and return it as a string (plain text or
/// full ANSI). One-shot. When the socket is gone the connect error is
/// returned; the post-exit view comes from `lastLines` in the metadata.
pub fn peek(name: &str, plain: bool, full: bool) -> std::io::Result<String> {
    let mut stream = connect(name)?;
    stream.write_all(&encode_peek(plain, full))?;
    stream.flush()?;
    // Read until we get a SCREEN packet (or EOF).
    let mut parser = PacketReader::new();
    let mut buf = [0u8; 8192];
    let deadline = Instant::now() + Duration::from_secs(5);
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for p in parser.feed(&buf[..n]).unwrap_or_default() {
                    if p.type_ == MessageType::Screen {
                        return Ok(String::from_utf8_lossy(&p.payload).into_owned());
                    }
                }
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                if Instant::now() > deadline {
                    break;
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(String::new())
}

/// Peek repeatedly until `needle` appears on screen or `timeout` elapses.
pub fn peek_wait(name: &str, needle: &str, timeout: Duration) -> std::io::Result<Option<String>> {
    let start = Instant::now();
    loop {
        let screen = peek(name, true, false)?;
        if screen.contains(needle) {
            return Ok(Some(screen));
        }
        if start.elapsed() >= timeout {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Follow a session read-only: replay the current screen, then stream live
/// output to stdout until the process exits or the caller interrupts (Ctrl-C).
/// Attaches geometry-neutral so following never resizes the shared PTY, and
/// never forwards stdin. Returns the exit code if the session ended.
pub fn follow(name: &str) -> std::io::Result<Option<i32>> {
    let stream = connect(name)?;
    let (rows, cols) = terminal_size();
    {
        let mut w = stream.try_clone()?;
        w.write_all(&encode_attach(rows, cols, true))?;
        w.flush()?;
    }
    let mut stream = stream;
    let mut parser = PacketReader::new();
    let mut buf = [0u8; 8192];
    let mut out = std::io::stdout();
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for p in parser.feed(&buf[..n]).unwrap_or_default() {
                    match p.type_ {
                        MessageType::Data | MessageType::Screen => {
                            out.write_all(&p.payload)?;
                            out.flush()?;
                        }
                        MessageType::Exit => return Ok(Some(decode_exit(&p.payload))),
                        _ => {}
                    }
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// Send raw bytes as terminal input to the session.
pub fn send(name: &str, data: &[u8]) -> std::io::Result<()> {
    let mut stream = connect(name)?;
    stream.write_all(&encode_data(data))?;
    stream.flush()?;
    // Give the daemon a moment to consume before we close.
    std::thread::sleep(Duration::from_millis(50));
    Ok(())
}

/// Default gap between `--seq` items (node's `DEFAULT_SEQ_DELAY_MS`). The gap
/// exists so a trailing `key:return` fired with zero delay doesn't land before
/// the program has parsed the typed text.
pub const DEFAULT_SEQ_DELAY_MS: u64 = 300;

/// Resolve the `--seq` inter-item delay in ms, matching node's
/// `resolveSeqDelayMs`: `None` → 300 (default); `Some(0)` → 0 (straight stream);
/// `Some(n)` → `n * 1000`.
pub fn resolve_seq_delay_ms(delay_secs: Option<f64>) -> u64 {
    match delay_secs {
        None => DEFAULT_SEQ_DELAY_MS,
        // node uses Math.round (not truncation): 0.4285s -> 429ms.
        Some(n) => (n * 1000.0).round() as u64,
    }
}

/// Send an ordered sequence of items over one connection, sleeping `delay_ms`
/// BETWEEN items (not before the first, not after the last) — matching node's
/// `send --seq` pacing.
pub fn send_seq(name: &str, items: &[Vec<u8>], delay_ms: u64) -> std::io::Result<()> {
    let mut stream = connect(name)?;
    let last = items.len().saturating_sub(1);
    for (idx, item) in items.iter().enumerate() {
        stream.write_all(&encode_data(item))?;
        stream.flush()?;
        if idx != last && delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
    }
    // Give the daemon a moment to consume before we close.
    std::thread::sleep(Duration::from_millis(50));
    Ok(())
}

/// Request STATUS JSON from the session.
pub fn status(name: &str) -> std::io::Result<String> {
    let mut stream = connect(name)?;
    stream.write_all(&encode_status())?;
    stream.flush()?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    if let Some(p) = read_packet(&mut stream)?
        && p.type_ == MessageType::Status {
            return Ok(String::from_utf8_lossy(&p.payload).into_owned());
        }
    Ok(String::new())
}

/// Query the controlling terminal size of the current process (rows, cols).
pub fn terminal_size() -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_row > 0 {
            (ws.ws_row, ws.ws_col)
        } else {
            (24, 80)
        }
    }
}

/// RAII guard that puts a tty fd into raw mode and restores it on drop.
struct RawMode {
    fd: i32,
    original: libc::termios,
}

impl RawMode {
    fn enable(fd: i32) -> Option<RawMode> {
        unsafe {
            let mut termios: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut termios) != 0 {
                return None;
            }
            let original = termios;
            libc::cfmakeraw(&mut termios);
            if libc::tcsetattr(fd, libc::TCSANOW, &termios) != 0 {
                return None;
            }
            Some(RawMode { fd, original })
        }
    }
    fn restore(&self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Detach key: Ctrl+\ (0x1c), matching the original pty. Press it once to
/// detach; press it twice in quick succession to send a literal Ctrl+\ to the
/// child.
pub const DETACH_KEY: u8 = 0x1c;

/// Kitty keyboard-protocol encoding of Ctrl+\, normalized to [`DETACH_KEY`] so
/// the detach logic works with a single representation (mirrors the TS).
const DETACH_KEY_KITTY: &[u8] = b"\x1b[92;5u";

/// Double-tap window: a second Ctrl+\ within this sends the literal byte to the
/// child instead of detaching. Matches the TS `DOUBLE_TAP_MS`.
const DOUBLE_TAP_MS: u128 = 300;

/// Reset terminal modes a program may have enabled, so the terminal isn't left
/// "poisoned" after detach (alt screen, mouse tracking, hidden cursor, …).
/// Ported from the TS `TERMINAL_SANITIZE`. Does not clear screen content.
pub const TERMINAL_SANITIZE: &str = concat!(
    "\x1b[?1049l", // leave alternate screen buffer
    "\x1b[?1l",    // reset cursor keys to normal (DECCKM)
    "\x1b[?7h",    // re-enable autowrap (DECAWM)
    "\x1b[?6l",    // reset origin mode (DECOM)
    "\x1b[?1000l", // disable mouse click tracking
    "\x1b[?1002l", // disable mouse button-event tracking
    "\x1b[?1003l", // disable mouse any-event tracking
    "\x1b[?1004l", // disable focus event reporting
    "\x1b[?1006l", // disable SGR mouse mode
    "\x1b[?25h",   // show cursor
    "\x1b[?2004l", // disable bracketed paste
    "\x1b[4l",     // reset insert mode (IRM) to replace
    "\x1b[r",      // reset scroll region (DECSTBM)
    "\x1b[0m",     // reset SGR attributes
    "\x1b[0 q",    // reset cursor style
    "\x1b>",       // reset application keypad mode (DECKPNM)
    "\x1b(B",      // reset G0 charset to ASCII
    "\x1b[<99u",   // pop all Kitty keyboard protocol levels
);

/// Move the cursor to the bottom so "[detached]" lands below session content.
const CURSOR_TO_BOTTOM: &str = "\x1b[999;1H";

/// Replace the Kitty encoding of Ctrl+\ with the legacy byte.
fn normalize_detach_key(data: &[u8]) -> Vec<u8> {
    if !data.windows(DETACH_KEY_KITTY.len()).any(|w| w == DETACH_KEY_KITTY) {
        return data.to_vec();
    }
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i..].starts_with(DETACH_KEY_KITTY) {
            out.push(DETACH_KEY);
            i += DETACH_KEY_KITTY.len();
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

enum PollRead {
    Timeout,
    Eof,
    Data(usize),
}

/// Poll `fd` for readability up to `timeout_ms` (-1 = block), then read once.
fn poll_read(fd: i32, buf: &mut [u8], timeout_ms: i32) -> std::io::Result<PollRead> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let r = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if r == 0 {
        return Ok(PollRead::Timeout);
    }
    let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
    if n <= 0 {
        return Ok(PollRead::Eof);
    }
    Ok(PollRead::Data(n as usize))
}

/// Attach interactively to a session. Forwards stdin→session and
/// session→stdout, replays the screen, and returns when the child exits or the
/// user presses Ctrl+\ to detach (twice in quick succession sends a literal
/// Ctrl+\ to the child). Returns the child's exit code (or `None` on detach).
pub fn attach(name: &str) -> std::io::Result<Option<i32>> {
    let stream = connect(name)?;
    let (rows, cols) = terminal_size();

    let stdin_fd = libc::STDIN_FILENO;
    let raw = RawMode::enable(stdin_fd);
    // Original termios for the reader thread to restore before a hard exit.
    let saved = raw.as_ref().map(|r| r.original);

    // ATTACH with our geometry.
    {
        let mut w = stream.try_clone()?;
        w.write_all(&encode_attach(rows, cols, false))?;
        w.flush()?;
    }

    // Reader thread: session packets → stdout. On EXIT, restore tty and exit.
    let read_stream = stream.try_clone()?;
    let _reader = std::thread::spawn(move || {
        let mut stream = read_stream;
        let mut parser = PacketReader::new();
        let mut buf = [0u8; 8192];
        let mut out = std::io::stdout();
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for p in parser.feed(&buf[..n]).unwrap_or_default() {
                        match p.type_ {
                            MessageType::Data | MessageType::Screen => {
                                let _ = out.write_all(&p.payload);
                                let _ = out.flush();
                            }
                            MessageType::Exit => {
                                let code = decode_exit(&p.payload);
                                if let Some(t) = saved {
                                    unsafe {
                                        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
                                    }
                                }
                                // Newline so the shell prompt lands cleanly.
                                let _ = out.write_all(b"\r\n");
                                let _ = out.flush();
                                std::process::exit(if code < 0 { 0 } else { code });
                            }
                            _ => {}
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Main: stdin → session (DATA), with Ctrl+\ to detach (double-tap → literal).
    let mut wstream = stream.try_clone()?;
    let mut buf = [0u8; 4096];
    // When Some, a detach is armed and fires when the double-tap window closes
    // with no second Ctrl+\.
    let mut armed_at: Option<Instant> = None;
    let mut detached = false;

    loop {
        // If a detach is armed, only wait out the remainder of the window.
        let timeout_ms: i32 = match armed_at {
            Some(t) => {
                let elapsed = t.elapsed().as_millis();
                if elapsed >= DOUBLE_TAP_MS {
                    detached = true;
                    break; // window closed, no second tap → detach
                }
                (DOUBLE_TAP_MS - elapsed) as i32
            }
            None => -1,
        };

        match poll_read(stdin_fd, &mut buf, timeout_ms)? {
            PollRead::Eof => break,
            PollRead::Timeout => {
                // Window elapsed with no further input → detach.
                detached = true;
                break;
            }
            PollRead::Data(n) => {
                let data = normalize_detach_key(&buf[..n]);
                let mut forward: Vec<u8> = Vec::with_capacity(data.len());
                for &b in &data {
                    if b == DETACH_KEY {
                        match armed_at {
                            Some(t) if t.elapsed().as_millis() < DOUBLE_TAP_MS => {
                                // Double-tap: send a literal Ctrl+\ to the child.
                                armed_at = None;
                                forward.push(DETACH_KEY);
                            }
                            _ => {
                                // First tap: arm the detach.
                                armed_at = Some(Instant::now());
                            }
                        }
                    } else {
                        forward.push(b);
                    }
                }
                if !forward.is_empty()
                    && (wstream.write_all(&encode_data(&forward)).is_err()
                        || wstream.flush().is_err())
                {
                    break;
                }
            }
        }
    }

    if detached {
        let _ = wstream.write_all(&encode_detach());
        let _ = wstream.flush();
    }
    drop(raw); // restore tty
    if detached {
        let mut out = std::io::stdout();
        let _ = out.write_all(TERMINAL_SANITIZE.as_bytes());
        let _ = out.write_all(CURSOR_TO_BOTTOM.as_bytes());
        let _ = out.write_all(b"\r\n[detached]\r\n");
        let _ = out.flush();
    }
    Ok(None)
}

/// Send a resize to the session (used by scripted clients / tests).
pub fn resize(name: &str, rows: u16, cols: u16) -> std::io::Result<()> {
    let mut stream = connect(name)?;
    stream.write_all(&encode_resize(rows, cols))?;
    stream.flush()?;
    Ok(())
}

/// Is a session's socket connectable right now?
pub fn is_alive(name: &str) -> bool {
    connect(name).is_ok()
}
