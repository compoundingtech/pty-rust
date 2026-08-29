//! Shared scaffolding for the registry and events tests: one isolated
//! `PTY_ROOT` per test binary (set once, before any registry read, so the
//! process-wide environment is never mutated while another thread reads
//! it), unique session names, and the Node `pty` oracle when it is on PATH.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The registry root for this test process. The first call sets `PTY_ROOT`
/// (and silences the legacy notices); every registry call must go through
/// this first so the environment is settled before any thread reads it.
pub fn root() -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "t".to_string());
        let short: String = exe.chars().take(12).collect();
        let dir = std::env::temp_dir().join(format!("pty-{short}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test root");
        // Private like Node's `ensureSessionDir`, so a Node daemon under it
        // advertises its recovery capability.
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        // SAFETY: the OnceLock guarantees this runs exactly once and every
        // test enters through `root()` before touching the registry, so no
        // other thread is reading the environment concurrently.
        unsafe {
            std::env::set_var("PTY_ROOT", &dir);
            std::env::set_var("PTY_ROOT_LEGACY_SILENT", "1");
            std::env::remove_var("PTY_SESSION_DIR");
            std::env::remove_var("PTY_SESSION");
        }
        dir
    })
    .clone()
}

/// A session name unique within this process.
pub fn unique_name(prefix: &str) -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}{n}-{:x}", std::process::id() % 0xffff)
}

/// Every registry file of `name` (ignores missing).
pub fn remove_session_files(name: &str) {
    let root = root();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for e in entries.flatten() {
            let fname = e.file_name().to_string_lossy().into_owned();
            if fname.starts_with(&format!("{name}.")) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// A pid no live process can own (above Linux's `pid_max`).
pub const DEAD_PID: i32 = 2_147_483_646;

/// Parsed lines of `<name>.events.jsonl` (empty when missing).
pub fn read_events(name: &str) -> Vec<serde_json::Value> {
    let path = root().join(format!("{name}.events.jsonl"));
    match std::fs::read_to_string(path) {
        Ok(s) => s
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).expect("valid JSONL line"))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// The Node `pty` binary (the parity oracle) when it is on PATH and is the
/// pinned version; `None` skips oracle tests.
pub fn node_pty() -> Option<PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        let out = Command::new("pty").arg("--version").output().ok()?;
        let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !version.starts_with("0.12.") {
            return None;
        }
        let which = Command::new("sh")
            .args(["-c", "command -v pty"])
            .output()
            .ok()?;
        let path = String::from_utf8_lossy(&which.stdout).trim().to_string();
        (!path.is_empty()).then(|| PathBuf::from(path))
    })
    .clone()
}

/// Run the Node `pty` against a root.
pub fn run_node_pty(bin: &Path, root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(bin)
        .args(args)
        .env("PTY_ROOT", root)
        .env("PTY_ROOT_LEGACY_SILENT", "1")
        .env_remove("PTY_SESSION")
        .env_remove("PTY_SESSION_DIR")
        .output()
        .expect("spawn node pty")
}

/// Wait until `pred` holds, polling every 20 ms for up to `ms`.
pub fn wait_for(ms: u64, mut pred: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    while std::time::Instant::now() < deadline {
        if pred() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    pred()
}
