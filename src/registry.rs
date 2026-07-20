//! Session registry: on-disk layout for sessions, mirroring the pty project's
//! `src/sessions.ts` (paths + `SessionMetadata` shape).
//!
//! Session state lives under `$PTY_ROOT` (default `~/.local/state/pty`), with
//! per-session files `<name>.sock`, `<name>.pid`, `<name>.json`.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// On-disk metadata for a session (`<name>.json`). Field names match the TS
/// `SessionMetadata` (camelCase on the wire).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadata {
    pub command: String,
    pub args: Vec<String>,
    /// Original command as the user typed it.
    pub display_command: String,
    pub cwd: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exited_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_lines: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tags: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_attach_at: Option<String>,
}

/// Resolve the session registry directory: `$PTY_ROOT`, else the deprecated
/// `$PTY_SESSION_DIR`, else `~/.local/state/pty`.
pub fn session_dir() -> PathBuf {
    if let Ok(root) = std::env::var("PTY_ROOT")
        && !root.is_empty() {
            return PathBuf::from(root);
        }
    if let Ok(legacy) = std::env::var("PTY_SESSION_DIR")
        && !legacy.is_empty() {
            return PathBuf::from(legacy);
        }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".local/state/pty")
}

/// Create the session dir (mode 0700) if missing.
pub fn ensure_session_dir() -> io::Result<PathBuf> {
    let dir = session_dir();
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

/// Path to a session's unix socket.
pub fn socket_path(name: &str) -> PathBuf {
    session_dir().join(format!("{name}.sock"))
}

/// Path to a session's pid file.
pub fn pid_path(name: &str) -> PathBuf {
    session_dir().join(format!("{name}.pid"))
}

/// Path to a session's metadata JSON.
pub fn metadata_path(name: &str) -> PathBuf {
    session_dir().join(format!("{name}.json"))
}

/// Write session metadata atomically (write to a temp file, then rename).
pub fn write_metadata(name: &str, meta: &SessionMetadata) -> io::Result<()> {
    ensure_session_dir()?;
    let path = metadata_path(name);
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Read session metadata, or `None` if it doesn't exist / can't be parsed.
pub fn read_metadata(name: &str) -> Option<SessionMetadata> {
    let path = metadata_path(name);
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

/// A session as seen by `pty ls`.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub name: String,
    pub meta: SessionMetadata,
    pub alive: bool,
}

/// Is a process with `pid` alive? (`kill(pid, 0)`.)
pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: kill with signal 0 only checks for existence/permission.
    unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

/// Read the pid recorded for a session, if any.
pub fn read_pid(name: &str) -> Option<i32> {
    let data = std::fs::read_to_string(pid_path(name)).ok()?;
    data.trim().parse().ok()
}

/// Enumerate all sessions in the registry (by `<name>.json` files).
pub fn list_sessions() -> Vec<SessionInfo> {
    let dir = session_dir();
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        // Skip the atomic-write temp files (`<name>.json.tmp`).
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".json.tmp"))
            .unwrap_or(false)
        {
            continue;
        }
        let name = match path.file_stem().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if let Some(meta) = read_metadata(&name) {
            let alive = read_pid(&name).map(pid_alive).unwrap_or(false) && meta.exit_code.is_none();
            out.push(SessionInfo { name, meta, alive });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Remove all on-disk files for a session.
pub fn cleanup(name: &str) {
    for p in [socket_path(name), pid_path(name), metadata_path(name)] {
        let _ = std::fs::remove_file(p);
    }
    let dir = session_dir();
    let _ = std::fs::remove_file(dir.join(format!("{name}.events.jsonl")));
}

/// Does a session with this name have live metadata?
pub fn session_exists(name: &str) -> bool {
    metadata_path(name).exists()
}

/// Validate a session name for use in a socket path (no slashes / control
/// chars; length-bounded so the socket path fits `sockaddr_un`).
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("session name cannot be empty".into());
    }
    if name.len() > 200 {
        return Err("session name too long".into());
    }
    if name
        .chars()
        .any(|c| c == '/' || c == '\\' || c.is_control())
    {
        return Err("session name may not contain slashes or control characters".into());
    }
    let socket = socket_path(name);
    if socket.as_os_str().len() > 103 {
        return Err(format!(
            "socket path for {name:?} exceeds the sockaddr_un limit; use a shorter name or PTY_ROOT"
        ));
    }
    Ok(())
}

/// Best-effort helper: a short, filesystem-safe unique id.
pub fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let mixed = nanos ^ (pid.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    to_base36(mixed).chars().take(8).collect()
}

fn to_base36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_string();
    }
    let mut s = Vec::new();
    while n > 0 {
        s.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

/// Resolve a user-provided reference (name or displayName) to a session name.
pub fn resolve_ref(reference: &str) -> Option<String> {
    if session_exists(reference) {
        return Some(reference.to_string());
    }
    // Fall back to a displayName match.
    list_sessions()
        .into_iter()
        .find(|s| s.meta.display_name.as_deref() == Some(reference))
        .map(|s| s.name)
}

/// Path helper used by tests / callers that want the dir as a `&Path`.
pub fn session_dir_path() -> impl AsRef<Path> {
    session_dir()
}
