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
/// full ANSI). One-shot.
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

/// Detach key: Ctrl-] (0x1d), telnet-style — leaves the session running.
const DETACH_KEY: u8 = 0x1d;

/// Attach interactively to a session. Forwards stdin→session and
/// session→stdout, replays the screen, and returns when the child exits or the
/// user presses Ctrl-] to detach. Returns the child's exit code (or `None` on
/// detach).
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
    let reader = std::thread::spawn(move || {
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

    // Main: stdin → session (DATA), with Ctrl-] to detach.
    let mut wstream = stream.try_clone()?;
    let mut stdin = std::io::stdin();
    let mut byte = [0u8; 1];
    let detached;
    loop {
        match stdin.read(&mut byte) {
            Ok(0) => {
                detached = false;
                break;
            }
            Ok(_) => {
                if byte[0] == DETACH_KEY {
                    let _ = wstream.write_all(&encode_detach());
                    let _ = wstream.flush();
                    detached = true;
                    break;
                }
                if wstream.write_all(&encode_data(&byte)).is_err() {
                    detached = false;
                    break;
                }
                let _ = wstream.flush();
            }
            Err(_) => {
                detached = false;
                break;
            }
        }
    }

    drop(raw); // restore tty
    if detached {
        // Detached cleanly; leave the reader thread to be reaped on exit.
        let _ = reader;
        Ok(None)
    } else {
        Ok(None)
    }
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
