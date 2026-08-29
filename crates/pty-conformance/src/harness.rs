//! The conformance rig: an isolated `PTY_ROOT`, a scrubbed environment, and
//! helpers to run the binary under test, host daemons, and speak the wire
//! protocol to them directly.
//!
//! Everything here is black-box: the rig only knows the path of a `pty`
//! binary and the on-disk registry layout (`<root>/<id>.{sock,pid,json}`).

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pty_core::protocol::{self, MessageType, Packet, PacketReader};
use pty_testkit::{Session, SpawnOptions};
use serde_json::Value;

/// Environment variables scrubbed from every process the rig starts, so the
/// suite behaves the same whether or not it runs inside a pty session.
pub const SCRUBBED_ENV: &[&str] = &[
    "PTY_SESSION",
    "PTY_SESSION_GENERATION",
    "PTY_SESSION_DIR",
    "PTY_REAP_ON_EXIT",
    "PTY_SERVER_CONFIG",
    "ST_AGENT",
    "ST_ROOT",
    "NO_COLOR",
    "PTY_ROOT",
];

/// Per-invocation timeout for a CLI call.
pub const CLI_TIMEOUT: Duration = Duration::from_secs(30);

/// Result of one CLI invocation.
#[derive(Debug, Clone)]
pub struct Out {
    /// Exit status; `-1` when the process was killed by the timeout or died
    /// from a signal.
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Set when the 30 s timeout fired.
    pub timed_out: bool,
}

impl Out {
    /// stdout as (lossy) UTF-8.
    pub fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// stderr as (lossy) UTF-8.
    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// A one-line summary for assertion messages.
    pub fn summary(&self) -> String {
        format!(
            "status={} timed_out={}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            self.status,
            self.timed_out,
            self.stdout(),
            self.stderr()
        )
    }
}

/// Where the binary under test comes from.
pub fn pty_bin() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Ok(p) = std::env::var("PTY_TEST_BIN")
            && !p.is_empty()
        {
            let p = PathBuf::from(p);
            assert!(p.is_absolute(), "PTY_TEST_BIN must be an absolute path: {}", p.display());
            assert!(p.exists(), "PTY_TEST_BIN does not exist: {}", p.display());
            return p;
        }
        let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(profile)
            .join("pty");
        let p = p.canonicalize().unwrap_or(p);
        assert!(
            p.exists(),
            "no pty binary at {} — run `cargo build -p pty` or set PTY_TEST_BIN",
            p.display()
        );
        p
    })
}

/// The version string printed by the binary under test.
pub fn pty_version() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| {
        let out = Command::new(pty_bin())
            .arg("--version")
            .env("PTY_ROOT_LEGACY_SILENT", "1")
            .output()
            .expect("run pty --version");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    })
}

/// True when the binary under test is the Rust port (`-rust` version marker).
pub fn is_rust() -> bool {
    pty_version().contains("-rust")
}

/// True when the binary under test is the Node reference implementation.
pub fn is_node() -> bool {
    !is_rust()
}

/// Default deadline for polls: 10 s, doubled by `PC_SLOW=1`.
pub fn deadline() -> Duration {
    let base = Duration::from_secs(10);
    if std::env::var("PC_SLOW").map(|v| v == "1").unwrap_or(false) {
        base * 2
    } else {
        base
    }
}

/// Poll `cond` every 10 ms until it returns true or the default deadline
/// passes. Panics with `what` on timeout.
pub fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    wait_until_for(what, deadline(), &mut cond);
}

/// Poll `cond` until true or `timeout` passes.
pub fn wait_until_for(what: &str, timeout: Duration, cond: &mut dyn FnMut() -> bool) {
    let start = Instant::now();
    loop {
        if cond() {
            return;
        }
        if start.elapsed() > timeout {
            panic!("timed out after {timeout:?} waiting for: {what}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Poll `cond` until true or `timeout` passes; returns whether it became true.
pub fn poll_for(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if cond() {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// True if `pid` is alive (signal 0).
pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: kill(2) with signal 0 only checks for existence/permission.
    let r = unsafe { libc::kill(pid, 0) };
    if r == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Send `sig` to `pid`, ignoring errors.
pub fn kill_pid(pid: i32, sig: i32) {
    if pid > 0 {
        // SAFETY: plain kill(2).
        unsafe {
            libc::kill(pid, sig);
        }
    }
}

fn mkdtemp(prefix: &str) -> PathBuf {
    let template = CString::new(format!("{prefix}XXXXXX")).unwrap();
    let mut buf = template.into_bytes_with_nul();
    // SAFETY: buf is a NUL-terminated template ending in XXXXXX.
    let p = unsafe { libc::mkdtemp(buf.as_mut_ptr() as *mut libc::c_char) };
    assert!(!p.is_null(), "mkdtemp failed: {}", std::io::Error::last_os_error());
    // SAFETY: mkdtemp rewrote the template in place; it is still NUL-terminated.
    let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
    PathBuf::from(s)
}

/// Options for [`Rig::daemon`].
#[derive(Debug, Clone, Default)]
pub struct DaemonOpts {
    pub no_display_name: bool,
    pub display_name: Option<String>,
    pub tags: Vec<(String, String)>,
    pub ephemeral: bool,
    /// `--env K=V` (persisted overlay).
    pub env: Vec<(String, String)>,
    /// `--unset-env K`.
    pub unset_env: Vec<String>,
    pub isolate_env: bool,
    pub cwd: Option<PathBuf>,
    /// Shortcut for `--tag keep=true`.
    pub keep: bool,
    pub rows: Option<u16>,
    pub cols: Option<u16>,
    /// Extra process environment for the `pty run` invocation itself
    /// (not the persisted overlay).
    pub invoke_env: Vec<(String, String)>,
    /// Environment variables removed from the `pty run` invocation.
    pub invoke_unset: Vec<String>,
}

impl DaemonOpts {
    pub fn no_display_name() -> Self {
        Self {
            no_display_name: true,
            ..Default::default()
        }
    }

    pub fn keep() -> Self {
        Self {
            no_display_name: true,
            keep: true,
            ..Default::default()
        }
    }

    pub fn tag(mut self, k: &str, v: &str) -> Self {
        self.tags.push((k.to_string(), v.to_string()));
        self
    }

    pub fn ephemeral(mut self) -> Self {
        self.ephemeral = true;
        self
    }

    pub fn with_env(mut self, k: &str, v: &str) -> Self {
        self.env.push((k.to_string(), v.to_string()));
        self
    }

    pub fn invoke_env(mut self, k: &str, v: &str) -> Self {
        self.invoke_env.push((k.to_string(), v.to_string()));
        self
    }
}

/// A daemon started by [`Rig::daemon`].
#[derive(Debug, Clone)]
pub struct Daemon {
    pub root: PathBuf,
    pub id: String,
    /// The `pty run -d` invocation's output.
    pub launch: Out,
}

impl Daemon {
    pub fn socket_path(&self) -> PathBuf {
        self.root.join(format!("{}.sock", self.id))
    }

    pub fn meta_path(&self) -> PathBuf {
        self.root.join(format!("{}.json", self.id))
    }

    pub fn pid_path(&self) -> PathBuf {
        self.root.join(format!("{}.pid", self.id))
    }

    /// Parsed `<id>.json` (untyped so unknown fields stay visible).
    pub fn meta(&self) -> Value {
        read_json(&self.meta_path()).unwrap_or_else(|| panic!("no metadata for {}", self.id))
    }

    /// The daemon pid from `<id>.pid`.
    pub fn pid(&self) -> i32 {
        read_pid_file(&self.pid_path()).unwrap_or_else(|| panic!("no pid for {}", self.id))
    }
}

/// Read and parse a JSON file; `None` if it is missing or not (yet) valid.
pub fn read_json(path: &Path) -> Option<Value> {
    let raw = std::fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// Read a pid file.
pub fn read_pid_file(path: &Path) -> Option<i32> {
    let raw = std::fs::read_to_string(path).ok()?;
    raw.trim().parse().ok()
}

/// One isolated registry (a temp `PTY_ROOT`) plus the environment policy.
pub struct Rig {
    tmp: PathBuf,
    root: PathBuf,
    home: PathBuf,
    extra_roots: std::sync::Mutex<Vec<PathBuf>>,
    keep_tmp: bool,
}

impl Rig {
    /// Create a fresh rig under `/tmp/pc-XXXXXX`.
    pub fn new() -> Rig {
        let tmp = mkdtemp("/tmp/pc-");
        let root = tmp.clone();
        let home = tmp.join("home");
        std::fs::create_dir_all(&home).unwrap();
        Rig {
            tmp,
            root,
            home,
            extra_roots: std::sync::Mutex::new(Vec::new()),
            keep_tmp: std::env::var("PC_KEEP").map(|v| v == "1").unwrap_or(false),
        }
    }

    /// The `PTY_ROOT` this rig runs in.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The temp `HOME`.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The temp directory everything lives under.
    pub fn tmp(&self) -> &Path {
        &self.tmp
    }

    /// A fresh empty directory under the rig (short path); torn down with the
    /// rig, and scanned for pid files at teardown.
    pub fn make_root(&self) -> PathBuf {
        let dir = mkdtemp(&format!("{}/r-", self.tmp.display()));
        self.extra_roots.lock().unwrap().push(dir.clone());
        dir
    }

    /// A fresh empty directory under the rig (for cwd tests and the like).
    pub fn make_dir(&self, name: &str) -> PathBuf {
        let dir = self.tmp.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The rig's base environment: the ambient env minus [`SCRUBBED_ENV`],
    /// plus `PTY_ROOT`, `PTY_ROOT_LEGACY_SILENT=1`, `TERM`, `HOME`.
    pub fn base_env(&self) -> BTreeMap<String, String> {
        let mut env: BTreeMap<String, String> = std::env::vars().collect();
        for k in SCRUBBED_ENV {
            env.remove(*k);
        }
        env.insert("PTY_ROOT".into(), self.root.to_string_lossy().into_owned());
        env.insert("PTY_ROOT_LEGACY_SILENT".into(), "1".into());
        env.insert("TERM".into(), "xterm-256color".into());
        env.insert("HOME".into(), self.home.to_string_lossy().into_owned());
        env
    }

    /// A `Command` for the binary under test with the rig's base environment.
    /// The cwd is the rig's temp dir.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(pty_bin());
        cmd.args(args);
        cmd.env_clear();
        for (k, v) in self.base_env() {
            cmd.env(k, v);
        }
        cmd.current_dir(&self.tmp);
        cmd
    }

    /// A `Command` with an environment built from scratch: only `PATH`,
    /// `HOME` (the rig's), and `extra`. No `PTY_ROOT` unless `extra` sets it.
    pub fn command_clean(&self, extra: &[(&str, &str)], args: &[&str]) -> Command {
        let mut cmd = Command::new(pty_bin());
        cmd.args(args);
        cmd.env_clear();
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.env("HOME", &self.home);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.current_dir(&self.tmp);
        cmd
    }

    /// Run the binary under test with the rig's base environment.
    pub fn pty(&self, args: &[&str]) -> Out {
        self.run(self.command(args), None)
    }

    /// Run with extra environment merged over the base environment.
    pub fn pty_env(&self, extra: &[(&str, &str)], args: &[&str]) -> Out {
        let mut cmd = self.command(args);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        self.run(cmd, None)
    }

    /// Run with some variables removed from the base environment (and
    /// `extra` merged in).
    pub fn pty_env_unset(&self, unset: &[&str], extra: &[(&str, &str)], args: &[&str]) -> Out {
        let mut cmd = self.command(args);
        for k in unset {
            cmd.env_remove(k);
        }
        for (k, v) in extra {
            cmd.env(k, v);
        }
        self.run(cmd, None)
    }

    /// Run with a from-scratch environment (see [`Rig::command_clean`]).
    pub fn pty_clean(&self, extra: &[(&str, &str)], args: &[&str]) -> Out {
        self.run(self.command_clean(extra, args), None)
    }

    /// Run with `stdin` fed from `bytes`.
    pub fn pty_stdin(&self, bytes: &[u8], args: &[&str]) -> Out {
        self.run(self.command(args), Some(bytes.to_vec()))
    }

    /// Run with `stdin` fed from `bytes` and extra environment.
    pub fn pty_stdin_env(&self, bytes: &[u8], extra: &[(&str, &str)], args: &[&str]) -> Out {
        let mut cmd = self.command(args);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        self.run(cmd, Some(bytes.to_vec()))
    }

    /// Run with the current directory set to `cwd`.
    pub fn pty_in(&self, cwd: &Path, args: &[&str]) -> Out {
        let mut cmd = self.command(args);
        cmd.current_dir(cwd);
        self.run(cmd, None)
    }

    /// Run an arbitrary prepared command with the 30 s timeout, collecting
    /// stdout and stderr. Detached daemons that inherit the pipes do not
    /// block collection: after the process exits the readers get two more
    /// seconds and whatever arrived by then is returned.
    pub fn run(&self, mut cmd: Command, stdin: Option<Vec<u8>>) -> Out {
        cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let mut child: Child = cmd.spawn().expect("spawn pty binary");
        if let Some(bytes) = stdin {
            let mut si = child.stdin.take().unwrap();
            std::thread::spawn(move || {
                let _ = si.write_all(&bytes);
            });
        }
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (tx_out, rx_out) = mpsc::channel();
        let (tx_err, rx_err) = mpsc::channel();
        std::thread::spawn(move || {
            let mut r = stdout;
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            let _ = tx_out.send(buf);
        });
        std::thread::spawn(move || {
            let mut r = stderr;
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            let _ = tx_err.send(buf);
        });
        let start = Instant::now();
        let mut timed_out = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break st.code().unwrap_or(-1),
                Ok(None) => {}
                Err(e) => panic!("wait on pty: {e}"),
            }
            if start.elapsed() > CLI_TIMEOUT {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break -1;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let grace = Duration::from_secs(2);
        let stdout = rx_out.recv_timeout(grace).unwrap_or_default();
        let stderr = rx_err.recv_timeout(grace).unwrap_or_default();
        Out {
            status,
            stdout,
            stderr,
            timed_out,
        }
    }

    /// Run the CLI inside a real terminal (for `attach`, `peek -f`, prompts).
    /// The environment is the rig's base environment; `extra` is merged in.
    pub fn pty_tty(&self, args: &[&str], rows: u16, cols: u16) -> Session {
        self.pty_tty_env(&[], args, rows, cols)
    }

    /// [`Rig::pty_tty`] with extra environment.
    pub fn pty_tty_env(&self, extra: &[(&str, &str)], args: &[&str], rows: u16, cols: u16) -> Session {
        // `env -u` scrubs the ambient variables the testkit would otherwise
        // pass through; the rig's fixed variables are set on the same line.
        let mut env_args: Vec<String> = Vec::new();
        for k in SCRUBBED_ENV {
            env_args.push("-u".into());
            env_args.push((*k).into());
        }
        let mut merged = self.base_env();
        for (k, v) in extra {
            merged.insert((*k).into(), (*v).into());
        }
        for k in ["PTY_ROOT", "PTY_ROOT_LEGACY_SILENT", "TERM", "HOME"] {
            if let Some(v) = merged.get(k) {
                env_args.push(format!("{k}={v}"));
            }
        }
        for (k, v) in extra {
            env_args.push(format!("{k}={v}"));
        }
        env_args.push(pty_bin().to_string_lossy().into_owned());
        for a in args {
            env_args.push((*a).into());
        }
        let refs: Vec<&str> = env_args.iter().map(String::as_str).collect();
        Session::spawn(
            "env",
            &refs,
            SpawnOptions {
                rows: Some(rows),
                cols: Some(cols),
                cwd: Some(self.tmp.clone()),
                env: Vec::new(),
            },
        )
        .expect("spawn pty in a tty")
    }

    /// Start a detached session with `pty run -d --id <id> ... -- cmd` and
    /// wait for its socket and metadata to appear.
    pub fn daemon(&self, id: &str, cmd: &[&str], opts: DaemonOpts) -> Daemon {
        let d = self.daemon_try(id, cmd, opts);
        assert_eq!(d.launch.status, 0, "pty run -d --id {id} failed: {}", d.launch.summary());
        self.wait_for_daemon(&d);
        d
    }

    /// Like [`Rig::daemon`] but returns whatever `pty run -d` did without
    /// asserting or waiting (for tests about failed launches).
    pub fn daemon_try(&self, id: &str, cmd: &[&str], opts: DaemonOpts) -> Daemon {
        let mut args: Vec<String> = vec!["run".into(), "-d".into(), "--id".into(), id.into()];
        if opts.no_display_name {
            args.push("--no-display-name".into());
        }
        if let Some(dn) = &opts.display_name {
            args.push("--name".into());
            args.push(dn.clone());
        }
        for (k, v) in &opts.tags {
            args.push("--tag".into());
            args.push(format!("{k}={v}"));
        }
        if opts.keep {
            args.push("--tag".into());
            args.push("keep=true".into());
        }
        if opts.ephemeral {
            args.push("-e".into());
        }
        for (k, v) in &opts.env {
            args.push("--env".into());
            args.push(format!("{k}={v}"));
        }
        for k in &opts.unset_env {
            args.push("--unset-env".into());
            args.push(k.clone());
        }
        if opts.isolate_env {
            args.push("--isolate-env".into());
        }
        if let Some(cwd) = &opts.cwd {
            args.push("--cwd".into());
            args.push(cwd.to_string_lossy().into_owned());
        }
        if let Some(r) = opts.rows {
            args.push("--rows".into());
            args.push(r.to_string());
        }
        if let Some(c) = opts.cols {
            args.push("--cols".into());
            args.push(c.to_string());
        }
        args.push("--".into());
        for c in cmd {
            args.push((*c).into());
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut command = self.command(&refs);
        for k in &opts.invoke_unset {
            command.env_remove(k);
        }
        for (k, v) in &opts.invoke_env {
            command.env(k, v);
        }
        let launch = self.run(command, None);
        Daemon {
            root: self.root.clone(),
            id: id.to_string(),
            launch,
        }
    }

    /// Wait (≤ 30 s) for `<id>.sock` and `<id>.json`.
    pub fn wait_for_daemon(&self, d: &Daemon) {
        let sock = d.socket_path();
        let meta = d.meta_path();
        wait_until_for(
            &format!("{} socket and metadata", d.id),
            Duration::from_secs(30),
            &mut || sock.exists() && read_json(&meta).is_some(),
        );
    }

    /// Path helpers for a session id in this rig's root.
    pub fn socket_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.sock"))
    }

    pub fn meta_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    pub fn pid_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.pid"))
    }

    /// Parsed metadata for `id`, if present.
    pub fn meta(&self, id: &str) -> Option<Value> {
        read_json(&self.meta_path(id))
    }

    /// The daemon pid for `id`, if present.
    pub fn pid(&self, id: &str) -> Option<i32> {
        read_pid_file(&self.pid_path(id))
    }

    /// `pty list --json` parsed (empty stdout → `[]`).
    pub fn list_json(&self) -> Vec<Value> {
        let out = self.pty(&["list", "--json"]);
        assert_eq!(out.status, 0, "list --json failed: {}", out.summary());
        let s = out.stdout();
        if s.trim().is_empty() {
            return Vec::new();
        }
        serde_json::from_str::<Value>(&s)
            .unwrap_or_else(|e| panic!("list --json is not JSON ({e}): {s}"))
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// The `list --json` entry named `id`, if any.
    pub fn list_entry(&self, id: &str) -> Option<Value> {
        self.list_json().into_iter().find(|e| e["name"] == id)
    }

    /// Wait until `<id>.json` carries an `exitCode` (the session exited and
    /// was preserved).
    pub fn wait_for_exit(&self, id: &str) {
        wait_until(&format!("{id} to exit"), || {
            self.meta(id).map(|m| m.get("exitCode").is_some()).unwrap_or(false)
        });
    }

    /// Wait until the session's registry files are gone.
    pub fn wait_for_gone(&self, id: &str) {
        wait_until(&format!("{id} to be removed"), || {
            !self.meta_path(id).exists() && !self.socket_path(id).exists()
        });
    }

    /// Wait until `list --json` reports `id` with `status`.
    pub fn wait_for_status(&self, id: &str, status: &str) {
        wait_until(&format!("{id} to be {status}"), || {
            self.list_entry(id).map(|e| e["status"] == status).unwrap_or(false)
        });
    }

    /// Open a raw protocol connection to `id`'s socket.
    pub fn connect(&self, id: &str) -> Conn {
        Conn::open(&self.socket_path(id))
    }

    /// Every pid recorded in a `*.pid` file under the rig (root and extra
    /// roots).
    pub fn recorded_pids(&self) -> Vec<i32> {
        let mut dirs = vec![self.root.clone()];
        dirs.extend(self.extra_roots.lock().unwrap().iter().cloned());
        let mut pids = Vec::new();
        let mut stack = dirs;
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().map(|x| x == "pid").unwrap_or(false)
                    && let Some(pid) = read_pid_file(&p)
                {
                    pids.push(pid);
                }
            }
        }
        pids.sort_unstable();
        pids.dedup();
        pids
    }

    /// SIGTERM every recorded pid, wait up to 2 s, SIGKILL survivors.
    pub fn teardown_daemons(&self) {
        let pids = self.recorded_pids();
        for &pid in &pids {
            kill_pid(pid, libc::SIGTERM);
        }
        let _ = poll_for(Duration::from_secs(2), || pids.iter().all(|&p| !pid_alive(p)));
        for &pid in &pids {
            if pid_alive(pid) {
                kill_pid(pid, libc::SIGKILL);
            }
        }
    }
}

impl Default for Rig {
    fn default() -> Self {
        Rig::new()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.teardown_daemons();
        if !self.keep_tmp {
            let _ = std::fs::remove_dir_all(&self.tmp);
        }
    }
}

/// A raw socket client speaking the daemon wire protocol.
pub struct Conn {
    stream: UnixStream,
    reader: PacketReader,
    queue: std::collections::VecDeque<Packet>,
    seen: Vec<MessageType>,
    eof: bool,
}

impl Conn {
    /// Connect to a daemon socket.
    pub fn open(path: &Path) -> Conn {
        let stream = UnixStream::connect(path)
            .unwrap_or_else(|e| panic!("connect {}: {e}", path.display()));
        Conn {
            stream,
            reader: PacketReader::new(),
            queue: Default::default(),
            seen: Vec::new(),
            eof: false,
        }
    }

    /// Try to connect; `None` on failure.
    pub fn try_open(path: &Path) -> Option<Conn> {
        let stream = UnixStream::connect(path).ok()?;
        Some(Conn {
            stream,
            reader: PacketReader::new(),
            queue: Default::default(),
            seen: Vec::new(),
            eof: false,
        })
    }

    /// The underlying stream.
    pub fn stream(&self) -> &UnixStream {
        &self.stream
    }

    /// Write raw bytes to the socket.
    pub fn write_raw(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.stream.write_all(bytes)?;
        self.stream.flush()
    }

    /// Send an ATTACH with a terminal size.
    pub fn attach(&mut self, rows: u16, cols: u16) {
        self.write_raw(&protocol::encode_attach(rows, cols, false)).expect("send ATTACH");
    }

    /// Send a geometry-neutral ATTACH.
    pub fn attach_neutral(&mut self, rows: u16, cols: u16) {
        self.write_raw(&protocol::encode_attach(rows, cols, true)).expect("send ATTACH");
    }

    /// Send a PEEK request.
    pub fn peek(&mut self, plain: bool, full: bool) {
        self.write_raw(&protocol::encode_peek(plain, full)).expect("send PEEK");
    }

    /// Send a RESIZE.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.write_raw(&protocol::encode_resize(rows, cols)).expect("send RESIZE");
    }

    /// Send DATA.
    pub fn data(&mut self, bytes: &[u8]) {
        self.write_raw(&protocol::encode_data(bytes)).expect("send DATA");
    }

    /// Send a DETACH.
    pub fn detach(&mut self) {
        let _ = self.write_raw(&protocol::encode_detach());
    }

    /// Send a STATUS request.
    pub fn status(&mut self) {
        self.write_raw(&protocol::encode_status()).expect("send STATUS");
    }

    /// Shut down the write half (signals EOF to the daemon).
    pub fn shutdown_write(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Write);
    }

    /// True once the daemon closed the connection.
    pub fn is_eof(&self) -> bool {
        self.eof
    }

    /// The next packet, waiting up to `timeout`. `None` on timeout or EOF.
    /// A framing error (declared length over the cap) panics with the error.
    pub fn next_packet(&mut self, timeout: Duration) -> Option<Packet> {
        match self.next_packet_result(timeout) {
            Ok(p) => p,
            Err(e) => panic!("protocol read error: {e}"),
        }
    }

    /// Like [`Conn::next_packet`] but surfaces framing errors.
    pub fn next_packet_result(&mut self, timeout: Duration) -> std::io::Result<Option<Packet>> {
        let start = Instant::now();
        loop {
            if let Some(p) = self.queue.pop_front() {
                self.seen.push(p.type_);
                return Ok(Some(p));
            }
            if self.eof {
                return Ok(None);
            }
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Ok(None);
            }
            self.stream
                .set_read_timeout(Some(remaining.max(Duration::from_millis(1))))
                .ok();
            let mut buf = [0u8; 65536];
            match self.stream.read(&mut buf) {
                Ok(0) => {
                    self.eof = true;
                }
                Ok(n) => {
                    let packets = self.reader.feed(&buf[..n])?;
                    self.queue.extend(packets);
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return Ok(None);
                }
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                    self.eof = true;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Wait for the next packet of `type_`, skipping others (which are still
    /// recorded in [`Conn::sequence`]).
    pub fn wait_for(&mut self, type_: MessageType, timeout: Duration) -> Option<Packet> {
        let start = Instant::now();
        loop {
            let remaining = timeout.saturating_sub(start.elapsed());
            let p = self.next_packet(remaining)?;
            if p.type_ == type_ {
                return Some(p);
            }
        }
    }

    /// Collect packets until an EXIT arrives, the daemon closes the socket,
    /// or `timeout` passes.
    pub fn collect_until_exit(&mut self, timeout: Duration) -> Vec<Packet> {
        let start = Instant::now();
        let mut out = Vec::new();
        loop {
            let remaining = timeout.saturating_sub(start.elapsed());
            match self.next_packet(remaining) {
                Some(p) => {
                    let is_exit = p.type_ == MessageType::Exit;
                    out.push(p);
                    if is_exit {
                        break;
                    }
                }
                None => break,
            }
        }
        out
    }

    /// Collect everything that arrives within `quiet` of silence.
    pub fn drain(&mut self, quiet: Duration) -> Vec<Packet> {
        let mut out = Vec::new();
        while let Some(p) = self.next_packet(quiet) {
            out.push(p);
        }
        out
    }

    /// Every packet type received so far, in order.
    pub fn sequence(&self) -> Vec<MessageType> {
        self.seen.clone()
    }

    /// Send STATUS and return the parsed JSON response.
    pub fn status_json(&mut self, timeout: Duration) -> Value {
        self.status();
        let p = self
            .wait_for(MessageType::Status, timeout)
            .expect("STATUS response");
        serde_json::from_slice(&p.payload).expect("STATUS payload is JSON")
    }
}

/// Concatenate the payloads of all DATA packets.
pub fn data_bytes(packets: &[Packet]) -> Vec<u8> {
    let mut v = Vec::new();
    for p in packets {
        if p.type_ == MessageType::Data {
            v.extend_from_slice(&p.payload);
        }
    }
    v
}

// ── Assertion helpers ──

/// Assert the exit status.
#[track_caller]
pub fn expect_status(out: &Out, status: i32) {
    assert_eq!(out.status, status, "unexpected exit status: {}", out.summary());
}

/// Assert a non-zero exit status.
#[track_caller]
pub fn expect_failure(out: &Out) {
    assert_ne!(out.status, 0, "expected failure: {}", out.summary());
}

/// stdout as a string (lossy).
pub fn stdout_str(out: &Out) -> String {
    out.stdout()
}

/// stderr as a string (lossy).
pub fn stderr_str(out: &Out) -> String {
    out.stderr()
}

/// Assert `hay` contains `needle`.
#[track_caller]
pub fn expect_contains(hay: &str, needle: &str) {
    assert!(hay.contains(needle), "expected {needle:?} in:\n{hay}");
}

/// Assert `hay` does not contain `needle`.
#[track_caller]
pub fn expect_not_contains(hay: &str, needle: &str) {
    assert!(!hay.contains(needle), "did not expect {needle:?} in:\n{hay}");
}

/// Assert `hay` matches the regex `re`.
#[track_caller]
pub fn expect_regex(hay: &str, re: &str) {
    let r = regex::Regex::new(re).expect("valid regex");
    assert!(r.is_match(hay), "expected /{re}/ to match:\n{hay}");
}

/// Assert `hay` does not match the regex `re`.
#[track_caller]
pub fn expect_not_regex(hay: &str, re: &str) {
    let r = regex::Regex::new(re).expect("valid regex");
    assert!(!r.is_match(hay), "did not expect /{re}/ to match:\n{hay}");
}

/// Number of non-overlapping matches of `re` in `hay`.
pub fn count_regex(hay: &str, re: &str) -> usize {
    regex::Regex::new(re).expect("valid regex").find_iter(hay).count()
}

/// Assert status 0 and parse stdout as JSON.
#[track_caller]
pub fn expect_json(out: &Out) -> Value {
    assert_eq!(out.status, 0, "expected success: {}", out.summary());
    let s = out.stdout();
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", out.summary()))
}

/// Parse stdout as JSON without checking the status.
#[track_caller]
pub fn parse_json(out: &Out) -> Value {
    let s = out.stdout();
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", out.summary()))
}

/// stdout split into lines (a trailing newline does not add an empty line).
pub fn expect_lines(out: &Out) -> Vec<String> {
    out.stdout().lines().map(|l| l.to_string()).collect()
}

/// Generate a short unique session id with `prefix`.
pub fn unique_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()
        % 100_000;
    format!("{prefix}{t}{n}")
}

/// Path of the Node reference checkout, when `PTY_NODE_CHECKOUT` is set.
pub fn node_checkout() -> Option<PathBuf> {
    std::env::var("PTY_NODE_CHECKOUT").ok().filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// The workspace root (two levels above this crate).
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// This crate's `fixtures/` directory.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}
