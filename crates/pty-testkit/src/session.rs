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

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use pty_core::keys::{resolve_key, KeyError};
use pty_terminal::{Screenshot, TerminalActor};

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

/// How long the `_default` waits allow, matching the Node package.
pub const DEFAULT_WAIT_MS: u64 = 10_000;

/// A spawned terminal session driving a real process through a PTY, with a
/// libghostty terminal tracking the screen state.
pub struct Session {
    /// The libghostty terminal, owned by the actor (query answers, mode
    /// tracking, and the one serializer shared with the daemon).
    actor: TerminalActor,
    writer: Box<dyn Write + Send>,
    rx: Receiver<Vec<u8>>,
    backing: Backing,
    rows: u16,
    cols: u16,
}

/// What is on the other end: a process this library started, or a session a
/// `pty` daemon owns.
enum Backing {
    Pty {
        master: Box<dyn MasterPty + Send>,
        child: Box<dyn portable_pty::Child + Send + Sync>,
    },
    Server(Box<crate::server::ServerBacking>),
}

/// How [`Session::server`] creates or finds a session.
#[derive(Debug, Clone, Default)]
pub struct ServerOptions {
    /// The session id. A random one when absent.
    pub name: Option<String>,
    pub rows: Option<u16>,
    pub cols: Option<u16>,
    pub cwd: Option<String>,
    /// Passed through as `--env KEY=VALUE`.
    pub env: Vec<(String, String)>,
    /// The registry to use. A temporary one when absent, removed on close.
    pub root: Option<std::path::PathBuf>,
}

impl Session {
    /// Spawn a process in a direct PTY. Use for CLI tools, TUI apps, or any
    /// process where you send input and check screen output.
    pub fn spawn(command: &str, args: &[&str], opts: SpawnOptions) -> std::io::Result<Session> {
        let rows = opts.rows.unwrap_or(24);
        let cols = opts.cols.unwrap_or(80);

        // The terminal actor answers queries (DA1, DSR, …) into its reply
        // buffer; `pump` flushes them back to the PTY, so programs like fish
        // that block on a DA1 reply start promptly.
        let actor = TerminalActor::new(rows, cols, pty_terminal::actor::DEFAULT_SCROLLBACK);

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
            actor,
            writer,
            rx,
            backing: Backing::Pty {
                master: pair.master,
                child,
            },
            rows,
            cols,
        })
    }

    /// Create a session through the `pty` binary and drive it over its
    /// socket. The screen arrives as frames rather than from a pty this
    /// process owns, so this is what a real client sees.
    ///
    /// The binary is `PTY_BIN`, else `pty` on PATH.
    pub fn server(command: &str, args: &[&str], opts: ServerOptions) -> std::io::Result<Session> {
        let rows = opts.rows.unwrap_or(24);
        let cols = opts.cols.unwrap_or(80);
        let name = opts.name.clone().unwrap_or_else(crate::server::random_id);
        let bin = crate::server::pty_bin();
        let owns_root = opts.root.is_none();
        let root = match opts.root.clone() {
            Some(root) => root,
            None => {
                // Short: a session socket path has to fit 104 bytes.
                let dir = std::env::temp_dir().join(format!("pt-{}", crate::server::random_id()));
                std::fs::create_dir_all(&dir)?;
                dir
            }
        };
        crate::server::spawn_daemon(
            &bin,
            &root,
            &name,
            command,
            args,
            rows,
            cols,
            opts.cwd.as_deref(),
            &opts.env,
        )?;
        Session::connect_at(&bin, root, name, rows, cols, true, owns_root)
    }

    /// Attach to a session that already exists, without owning it: closing
    /// this handle leaves the session running.
    pub fn connect(name: &str, rows: u16, cols: u16, root: std::path::PathBuf) -> std::io::Result<Session> {
        Session::connect_at(
            &crate::server::pty_bin(),
            root,
            name.to_string(),
            rows,
            cols,
            false,
            false,
        )
    }

    /// A second client on the same session, at its own size. The daemon
    /// gives every client the smallest requested geometry.
    pub fn connect_to_existing(other: &Session, rows: u16, cols: u16) -> std::io::Result<Session> {
        let Backing::Server(server) = &other.backing else {
            return Err(std::io::Error::other(
                "connect_to_existing needs a server-mode session",
            ));
        };
        Session::connect_at(
            &server.bin,
            server.root.clone(),
            server.name.clone(),
            rows,
            cols,
            false,
            false,
        )
    }

    fn connect_at(
        bin: &str,
        root: std::path::PathBuf,
        name: String,
        rows: u16,
        cols: u16,
        owned: bool,
        owns_root: bool,
    ) -> std::io::Result<Session> {
        let actor = TerminalActor::new(rows, cols, pty_terminal::actor::DEFAULT_SCROLLBACK);
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let (socket, state) = crate::server::connect(&root, &name, rows, cols, tx)?;
        let writer = Box::new(crate::server::DataFramer(socket.try_clone()?));
        Ok(Session {
            actor,
            writer,
            rx,
            backing: Backing::Server(Box::new(crate::server::ServerBacking {
                name,
                root,
                bin: bin.to_string(),
                socket,
                state,
                owned,
                owns_root,
            })),
            rows,
            cols,
        })
    }

    /// The session's id. Empty for a spawned process, which has none.
    pub fn name(&self) -> &str {
        match &self.backing {
            Backing::Server(server) => &server.name,
            Backing::Pty { .. } => "",
        }
    }

    /// The registry this session lives in, for a server-mode session.
    pub fn root(&self) -> Option<&std::path::Path> {
        match &self.backing {
            Backing::Server(server) => Some(&server.root),
            Backing::Pty { .. } => None,
        }
    }

    /// The exit status, once the session has ended.
    pub fn exit_code(&self) -> Option<i32> {
        match &self.backing {
            Backing::Server(server) => server
                .state
                .exited
                .load(std::sync::atomic::Ordering::Acquire)
                .then(|| server.state.exit_code.load(std::sync::atomic::Ordering::Relaxed)),
            Backing::Pty { .. } => None,
        }
    }

    /// Drop this client and open a new one, the way a client that lost its
    /// connection would. The screen is rebuilt from the daemon's replay.
    pub fn reconnect(&mut self) -> std::io::Result<()> {
        let Backing::Server(server) = &mut self.backing else {
            return Err(std::io::Error::other("reconnect needs a server-mode session"));
        };
        let (bin, root, name) = (server.bin.clone(), server.root.clone(), server.name.clone());
        let (owned, owns_root) = (server.owned, server.owns_root);
        let _ = server.socket.shutdown(std::net::Shutdown::Both);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let actor = TerminalActor::new(self.rows, self.cols, pty_terminal::actor::DEFAULT_SCROLLBACK);
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let (socket, state) = crate::server::connect(&root, &name, self.rows, self.cols, tx)?;
        self.writer = Box::new(crate::server::DataFramer(socket.try_clone()?));
        self.actor = actor;
        self.rx = rx;
        self.backing = Backing::Server(Box::new(crate::server::ServerBacking {
            name,
            root,
            bin,
            socket,
            state,
            owned,
            owns_root,
        }));
        Ok(())
    }

    /// Drain all currently-available PTY output into the terminal, then flush
    /// any query responses libghostty produced back to the PTY.
    fn pump(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(chunk) => {
                    self.actor.write(&chunk);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        let out = self.actor.take_pty_replies();
        if !out.is_empty() {
            let _ = self.writer.write_all(&out);
            let _ = self.writer.flush();
        }
    }

    // ── Properties ──

    /// Current terminal height in rows. In server mode this is the size the
    /// daemon settled on, which may be smaller than the one requested.
    pub fn rows(&self) -> u16 {
        match &self.backing {
            Backing::Server(server) => server.state.rows.load(std::sync::atomic::Ordering::Relaxed),
            Backing::Pty { .. } => self.rows,
        }
    }

    /// Current terminal width in columns. See [`Session::rows`].
    pub fn cols(&self) -> u16 {
        match &self.backing {
            Backing::Server(server) => server.state.cols.load(std::sync::atomic::Ordering::Relaxed),
            Backing::Pty { .. } => self.cols,
        }
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
        self.actor.screenshot()
    }

    /// The terminal actor behind this session, for typed reads
    /// (`snapshot`, `modes`, `plain`) after a [`Session::screenshot`] pump.
    pub fn actor(&self) -> &TerminalActor {
        &self.actor
    }

    /// The current terminal window title (set by the program via OSC 0/2).
    /// Drains pending output first.
    pub fn title(&mut self) -> String {
        self.pump();
        self.actor.title()
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

    /// Resize the far end and the terminal emulator together.
    ///
    /// In server mode the daemon decides the size — every client asks and
    /// the smallest wins — so `rows()` and `cols()` follow its answer rather
    /// than what was requested here.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
        match &mut self.backing {
            Backing::Pty { master, .. } => {
                let _ = master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
            Backing::Server(server) => server.resize(rows, cols),
        }
        self.actor.resize(cols, rows);
    }

    // ── Lifecycle ──

    /// True once the process on the far end has exited.
    pub fn has_exited(&mut self) -> bool {
        match &mut self.backing {
            Backing::Pty { child, .. } => matches!(child.try_wait(), Ok(Some(_))),
            Backing::Server(server) => server.state.exited.load(std::sync::atomic::Ordering::Acquire),
        }
    }

    /// Stop the session and clean up after it.
    ///
    /// A spawned process is killed. A session this handle created is killed
    /// and removed through the `pty` binary, so the daemon records its exit
    /// the way it would for any caller; a session this handle merely
    /// connected to is left running.
    pub fn close(&mut self) {
        match &mut self.backing {
            Backing::Pty { child, .. } => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Backing::Server(server) => server.close(),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        match &mut self.backing {
            Backing::Pty { child, .. } => {
                let _ = child.kill();
            }
            Backing::Server(server) => server.close(),
        }
    }
}
