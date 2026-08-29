//! A Playwright-style terminal test session, backed by **libghostty** as the
//! terminal emulator and a real PTY (`portable-pty`) as the process host.
//!
//! This is the Rust port of the pty project's `src/testing/session.ts`
//! (spawn mode). Instead of `@xterm/headless`, the PTY byte stream is fed into
//! a libghostty `Terminal`, and "screenshots" are produced by libghostty's
//! formatter.
//!
//! ```no_run
//! use pty_testkit::Session;
//! let mut s = Session::spawn("bash", &["--norc", "--noprofile"], Default::default()).unwrap();
//! s.wait_for_text("$", 5000).unwrap();
//! s.type_str("echo hello\r");
//! s.wait_for_text("hello", 5000).unwrap();
//! s.close();
//! ```

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use libghostty_vt::terminal::{Options, Terminal};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use pty_core::keys::{resolve_key, KeyError};
use pty_terminal::screenshot::{capture, Screenshot};

/// Options for [`Session::spawn`]. Mirrors the TS `SpawnOptions`.
#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    /// Terminal height in rows. Default: 24.
    pub rows: Option<u16>,
    /// Terminal width in columns. Default: 80.
    pub cols: Option<u16>,
    /// Working directory for the spawned process.
    pub cwd: Option<PathBuf>,
    /// Extra environment variables, merged over the inherited environment.
    pub env: Vec<(String, String)>,
}

/// Build the environment for a spawned `pty` process: the caller's base env
/// merged with `opts_env`, minus the harness's own pty-internal context.
///
/// Port of `buildSpawnEnv` from `session.ts`. Always scrubs
/// `PTY_SERVER_CONFIG` and `PTY_SESSION`; scrubs `PTY_ROOT` /
/// `PTY_SESSION_DIR` only when the caller didn't set them explicitly.
pub fn build_spawn_env(
    base: &[(String, String)],
    opts_env: &[(String, String)],
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = base.iter().cloned().collect();

    let opts_has = |k: &str| opts_env.iter().any(|(ek, _)| ek == k);
    for (k, v) in opts_env {
        env.insert(k.clone(), v.clone());
    }
    env.remove("PTY_SERVER_CONFIG");
    env.remove("PTY_SESSION");
    if !opts_has("PTY_ROOT") {
        env.remove("PTY_ROOT");
    }
    if !opts_has("PTY_SESSION_DIR") {
        env.remove("PTY_SESSION_DIR");
    }
    env
}

/// A spawned terminal session driving a real process through a PTY, with a
/// libghostty terminal tracking the screen state.
pub struct Session {
    terminal: Terminal<'static, 'static>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<Vec<u8>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    /// Bytes libghostty wants written back to the PTY (query responses etc.).
    pending: Rc<RefCell<Vec<u8>>>,
    rows: u16,
    cols: u16,
}

impl Session {
    /// Spawn a process in a direct PTY. Use for CLI tools, TUI apps, or any
    /// process where you send input and check screen output.
    pub fn spawn(command: &str, args: &[&str], opts: SpawnOptions) -> std::io::Result<Session> {
        let rows = opts.rows.unwrap_or(24);
        let cols = opts.cols.unwrap_or(80);

        // libghostty terminal. Query responses (DA1, DSR, …) are captured into
        // `pending` and flushed back to the PTY after each pump, so programs
        // like fish that block on a DA1 reply start promptly.
        let pending: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
        let mut terminal = Terminal::new(Options {
            cols,
            rows,
            max_scrollback: 10_000,
        })
        .expect("libghostty terminal");
        {
            let pending = pending.clone();
            terminal
                .on_pty_write(move |_term, data| {
                    pending.borrow_mut().extend_from_slice(data);
                })
                .expect("install on_pty_write");
        }

        // Real PTY + child process.
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(std::io::Error::other)?;

        let mut cmd = CommandBuilder::new(command);
        cmd.args(args);
        if let Some(cwd) = &opts.cwd {
            cmd.cwd(cwd);
        }
        // Apply the scrubbed, merged environment explicitly.
        let base: Vec<(String, String)> = std::env::vars().collect();
        let mut env = build_spawn_env(&base, &opts.env);
        // Default TERM to match the TS harness (node-pty used name:
        // "xterm-256color"); a caller-provided TERM in opts.env already won.
        env.entry("TERM".to_string())
            .or_insert_with(|| "xterm-256color".to_string());
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(std::io::Error::other)?;
        // Slave no longer needed in the parent; dropping it avoids a lingering
        // open fd that would keep the master from ever seeing EOF.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(std::io::Error::other)?;
        let writer = pair.master.take_writer().map_err(std::io::Error::other)?;

        // Reader thread: read PTY bytes → channel. libghostty's Terminal is
        // !Send, so it stays on this thread; the thread only ferries bytes.
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Session {
            terminal,
            master: pair.master,
            writer,
            rx,
            child,
            pending,
            rows,
            cols,
        })
    }

    /// Drain all currently-available PTY output into the terminal, then flush
    /// any query responses libghostty produced back to the PTY.
    fn pump(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(chunk) => self.terminal.vt_write(&chunk),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        let out = {
            let mut p = self.pending.borrow_mut();
            if p.is_empty() {
                Vec::new()
            } else {
                std::mem::take(&mut *p)
            }
        };
        if !out.is_empty() {
            let _ = self.writer.write_all(&out);
            let _ = self.writer.flush();
        }
    }

    // ── Properties ──

    /// Current terminal height in rows.
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Current terminal width in columns.
    pub fn cols(&self) -> u16 {
        self.cols
    }

    // ── Input ──

    /// Send raw keystrokes to the process. Use for literal text or escape
    /// sequences.
    pub fn send_keys(&mut self, keys: &str) {
        let _ = self.writer.write_all(keys.as_bytes());
        let _ = self.writer.flush();
    }

    /// Send a named key. Supports modifiers: `"ctrl+c"`, `"alt+x"`, `"shift+a"`.
    pub fn press(&mut self, key_name: &str) -> Result<(), KeyError> {
        let bytes = resolve_key(key_name)?;
        self.send_keys(&bytes);
        Ok(())
    }

    /// Send text to the process. Alias for [`Session::send_keys`].
    pub fn type_str(&mut self, text: &str) {
        self.send_keys(text);
    }

    // ── Screen ──

    /// Capture the current terminal state (drains pending output first).
    pub fn screenshot(&mut self) -> Screenshot {
        self.pump();
        capture(&self.terminal)
    }

    /// The current terminal window title (set by the program via OSC 0/2).
    /// Drains pending output first.
    pub fn title(&mut self) -> String {
        self.pump();
        self.terminal
            .title()
            .map(|t| t.to_string())
            .unwrap_or_default()
    }

    // ── Waiting ──

    /// Poll until the terminal contains `text`. Returns the matching screenshot.
    pub fn wait_for_text(&mut self, text: &str, timeout_ms: u64) -> Result<Screenshot, String> {
        self.wait_for(
            |ss| ss.text.contains(text),
            timeout_ms,
            &format!("text {text:?}"),
        )
    }

    /// Poll until the terminal no longer contains `text`.
    pub fn wait_for_absent(&mut self, text: &str, timeout_ms: u64) -> Result<Screenshot, String> {
        self.wait_for(
            |ss| !ss.text.contains(text),
            timeout_ms,
            &format!("absence of {text:?}"),
        )
    }

    /// Poll until `predicate` returns true. `description` is used in the
    /// timeout error message.
    pub fn wait_for<F>(
        &mut self,
        predicate: F,
        timeout_ms: u64,
        description: &str,
    ) -> Result<Screenshot, String>
    where
        F: Fn(&Screenshot) -> bool,
    {
        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms);
        loop {
            let ss = self.screenshot();
            if predicate(&ss) {
                return Ok(ss);
            }
            if start.elapsed() >= timeout {
                return Err(format!(
                    "Timed out after {timeout_ms}ms waiting for {description}.\nScreen:\n{}",
                    ss.text
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    // ── Resize ──

    /// Resize both the PTY and the terminal emulator.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        let _ = self.terminal.resize(cols, rows, 0, 0);
    }

    // ── Lifecycle ──

    /// True if the child process has exited.
    pub fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Kill the child process and clean up.
    pub fn close(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
