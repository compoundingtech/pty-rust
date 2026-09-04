//! `pty.toml` manifest parsing, ported from the pty project's `src/ptyfile.ts`.
//!
//! A manifest declares named sessions that `pty up` starts and `pty down`
//! stops:
//!
//! ```toml
//! prefix = "myapp"
//!
//! [sessions.web]
//! command = "node server.js"
//! cwd = ".."
//!
//! [sessions.web.env]
//! PORT = "3000"
//! ```

use std::path::{Path, PathBuf};

use indexmap::IndexMap;

/// A single session definition from a `pty.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtySessionDef {
    /// Human-friendly label (`<prefix>-<name>` by default, or `display_name`).
    pub display_name: String,
    /// Name as written in the toml (the `[sessions.<shortName>]` key).
    pub short_name: String,
    /// Explicit on-disk id from `id = "..."`, if any.
    pub id: Option<String>,
    pub command: String,
    /// Working directory (absolute; relative resolves against the manifest dir).
    pub cwd: Option<String>,
    /// Tags in manifest order (`pty up` prints them in that order).
    pub tags: Option<IndexMap<String, String>>,
    pub env: Option<IndexMap<String, String>>,
}

/// A parsed `pty.toml`.
#[derive(Debug, Clone)]
pub struct PtyFile {
    pub dir: PathBuf,
    pub prefix: Option<String>,
    pub sessions: Vec<PtySessionDef>,
}

/// Read and parse a `pty.toml` from `dir` (or the current directory).
pub fn read_pty_file(dir: Option<&Path>) -> Result<PtyFile, String> {
    // Match Node's `path.resolve(dir)`: make absolute lexically (no symlink
    // resolution).
    let resolved_dir = match dir {
        Some(d) if d.is_absolute() => normalize(d),
        Some(d) => normalize(&std::env::current_dir().map_err(|e| e.to_string())?.join(d)),
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    let file_path = resolved_dir.join("pty.toml");

    let content = match std::fs::read_to_string(&file_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("No pty.toml found in {}", resolved_dir.display()));
        }
        Err(e) => return Err(e.to_string()),
    };

    // Parse a TOML *document* (not a bare value — in toml 1.x `[sessions.x]`
    // would otherwise be read as an array literal).
    let parsed: toml::Table = content
        .parse()
        .map_err(|e| format!("Invalid pty.toml in {}: {e}", resolved_dir.display()))?;

    let prefix = parsed
        .get("prefix")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let mut sessions = Vec::new();

    if let Some(sessions_tbl) = parsed.get("sessions").and_then(|v| v.as_table()) {
        for (raw_name, def) in sessions_tbl {
            let default_display = match &prefix {
                Some(p) => format!("{p}-{raw_name}"),
                None => raw_name.clone(),
            };
            let d = def
                .as_table()
                .ok_or_else(|| {
                    format!(
                        "Invalid session \"{default_display}\" in {}: expected a table",
                        file_path.display()
                    )
                })?;

            let command = d
                .get("command")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    format!(
                        "Session \"{default_display}\" in {} is missing a \"command\" field",
                        file_path.display()
                    )
                })?
                .to_string();

            // display_name override.
            let mut display_name = default_display.clone();
            if let Some(dn) = d.get("display_name") {
                let s = dn.as_str().filter(|s| !s.is_empty()).ok_or_else(|| {
                    format!(
                        "Session \"{default_display}\" in {}: \"display_name\" must be a non-empty string",
                        file_path.display()
                    )
                })?;
                display_name = s.to_string();
            }

            // id override.
            let mut id = None;
            if let Some(idv) = d.get("id") {
                let s = idv.as_str().filter(|s| !s.is_empty()).ok_or_else(|| {
                    format!(
                        "Session \"{default_display}\" in {}: \"id\" must be a non-empty string",
                        file_path.display()
                    )
                })?;
                id = Some(s.to_string());
            }

            // tags.
            let mut tags = None;
            if let Some(t) = d.get("tags").and_then(|v| v.as_table()) {
                let mut m = IndexMap::new();
                for (k, v) in t {
                    m.insert(k.clone(), value_to_string(v));
                }
                tags = Some(m);
            }

            // env — must be a table of strings.
            let mut env = None;
            if let Some(ev) = d.get("env") {
                let t = ev.as_table().ok_or_else(|| {
                    format!(
                        "Session \"{default_display}\" in {}: \"env\" must be a table of string values",
                        file_path.display()
                    )
                })?;
                let mut m = IndexMap::new();
                for (k, v) in t {
                    let s = v.as_str().ok_or_else(|| {
                        format!(
                            "Session \"{default_display}\" in {}: env.{k} must be a string",
                            file_path.display()
                        )
                    })?;
                    m.insert(k.clone(), s.to_string());
                }
                env = Some(m);
            }

            // cwd — non-empty string, resolved against the manifest dir.
            let mut cwd = None;
            if let Some(cv) = d.get("cwd") {
                let s = cv.as_str().filter(|s| !s.is_empty()).ok_or_else(|| {
                    format!(
                        "Session \"{default_display}\" in {}: \"cwd\" must be a non-empty string",
                        file_path.display()
                    )
                })?;
                let resolved = resolve_against(&resolved_dir, s);
                cwd = Some(resolved.to_string_lossy().into_owned());
            }

            sessions.push(PtySessionDef {
                display_name,
                short_name: raw_name.clone(),
                id,
                command,
                cwd,
                tags,
                env,
            });
        }
    }

    if sessions.is_empty() {
        return Err(format!("No sessions defined in {}", file_path.display()));
    }

    Ok(PtyFile {
        dir: resolved_dir,
        prefix,
        sessions,
    })
}

/// Resolve `p` against `base`: absolute stays, relative joins + normalizes.
fn resolve_against(base: &Path, p: &str) -> PathBuf {
    let pp = Path::new(p);
    let joined = if pp.is_absolute() {
        pp.to_path_buf()
    } else {
        base.join(pp)
    };
    normalize(&joined)
}

/// Lexically normalize a path (resolve `.` and `..` without touching the fs),
/// matching Node's `path.resolve` semantics used by the TS.
fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = Vec::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                if matches!(out.last(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push(comp);
                }
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out.iter().collect()
}

fn value_to_string(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        other => other.to_string(),
    }
}

/// Build the `/bin/sh -c` payload for a session: an optional
/// `export K='V'; ...` prefix from `env`, followed by the command.
pub fn command_with_env_exports(sess: &PtySessionDef) -> String {
    let env = match &sess.env {
        Some(e) if !e.is_empty() => e,
        _ => return sess.command.clone(),
    };
    let prefix = env
        .iter()
        .map(|(k, v)| format!("export {k}='{}'", v.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join("; ");
    format!("{prefix}; {}", sess.command)
}
