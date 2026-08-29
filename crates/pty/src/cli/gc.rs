//! `pty gc [-n|--dry-run]`: reclaim registry debris, kill orphaned
//! `parent=<id>` children, sweep exited/vanished sessions (honouring the
//! `keep` tag), and prune dead `:l<pid>-<rand>` layout tags.
//! `pty gc --print-launchd-plist [--interval N]` prints a launchd job.
//!
//! The permanent-respawn, flapping and abandoned-reap steps of Node's gc
//! are not ported (docs/parity.md §12); `strategy=permanent` still keeps a
//! dead session out of the sweep. `--idle-days` and `--fast-fail-*` belonged
//! to those steps: they are accepted and ignored.
//!
//! node: src/cli.ts:1411-1453 (parsing), 3089-3202 (`cmdGc`), 3224-3276
//! (`printLaunchdPlist`); src/sessions.ts:620-880 (raw debris, observed
//! cleanup, `reapObservedSession`), 1521-1724 (`gc`), 2026-2075
//! (`pruneOrphanLayoutTags`)

use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pty_core::registry::{
    self, DEFAULT_SOCKET_PROBE_BUDGET, SessionInfo, TagMap, cleanup_all_while_locked,
    cleanup_socket, default_session_dir, events_path, has_process_exited_for_reap,
    is_keep_requested, metadata_matches_observation, metadata_path, pid_alive,
    probe_sockets_within_budget, read_metadata, read_pid, read_pid_with, recovery_revision_path,
    session_dir, socket_path, update_tags, with_both_locks,
};

use super::argv::js_parse_int;
use super::{CliError, CliResult};

/// Parse and run.
///
/// node: src/cli.ts:1411-1453
pub fn run(gc_args: &[String]) -> CliResult {
    let dry_run = gc_args.iter().any(|a| a == "--dry-run" || a == "-n");
    let print_plist = gc_args.iter().any(|a| a == "--print-launchd-plist");
    let mut interval: i64 = 30;
    let parse_positive = |flag: &str, raw: &str| -> Result<i64, CliError> {
        match js_parse_int(raw) {
            Some(v) if v > 0 => Ok(v),
            _ => Err(CliError(format!(
                "pty gc: {flag} expects a positive integer (got \"{raw}\")"
            ))),
        }
    };
    // The dropped tuning flags: consumed with their value, never validated.
    const IGNORED: [&str; 3] = ["--idle-days", "--fast-fail-window", "--fast-fail-limit"];
    let mut i = 0;
    while i < gc_args.len() {
        let a = gc_args[i].as_str();
        if a == "--interval" && i + 1 < gc_args.len() {
            i += 1;
            interval = parse_positive("--interval", &gc_args[i])?;
        } else if let Some(raw) = a.strip_prefix("--interval=") {
            interval = parse_positive("--interval", raw)?;
        } else if IGNORED.contains(&a) && i + 1 < gc_args.len() {
            i += 1;
        }
        i += 1;
    }
    if print_plist {
        print_launchd_plist(interval);
        return Ok(0);
    }
    cmd_gc(dry_run)
}

/// What one pass did (or would do).
#[derive(Debug, Default)]
struct GcResult {
    removed: Vec<String>,
    kept: Vec<String>,
    killed_orphan_children: Vec<OrphanKill>,
    reap_skipped: Vec<ReapSkip>,
}

#[derive(Debug)]
struct OrphanKill {
    name: String,
    parent: String,
    reason: &'static str,
}

#[derive(Debug)]
struct ReapSkip {
    name: String,
    operation: &'static str,
    reason: &'static str,
    signalled: bool,
}

/// One session whose tags were (or would be) pruned.
#[derive(Debug)]
struct PrunedTags {
    name: String,
    removed_keys: Vec<String>,
}

/// `cmdGc`.
///
/// node: src/cli.ts:3089-3202
fn cmd_gc(dry_run: bool) -> CliResult {
    let result = gc(dry_run);
    let pruned = prune_orphan_layout_tags(dry_run);

    let killed_verb = if dry_run { "Would kill orphan child" } else { "Killed orphan child" };
    let remove_verb = if dry_run { "Would remove" } else { "Removed" };
    let pruned_verb = if dry_run { "Would prune" } else { "Pruned" };

    for k in &result.killed_orphan_children {
        println!("{killed_verb}: {} (parent {} {})", k.name, k.parent, k.reason);
    }
    for s in &result.reap_skipped {
        let phase = if s.signalled { "after signalling" } else { "before signalling" };
        println!(
            "Skipped {} reap: {} ({}, {phase})",
            s.operation, s.name, s.reason
        );
    }
    for name in &result.removed {
        println!("{remove_verb}: {name}");
    }
    // A kept session is not an action; it is printed so "why is this dead
    // session still listed?" has a visible answer.
    for name in &result.kept {
        println!("Kept (keep tag): {name} — remove the keep tag to reap it");
    }
    for p in &pruned {
        println!(
            "{pruned_verb} orphan tags on {}: {}",
            p.name,
            p.removed_keys
                .iter()
                .map(|k| format!("#{k}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    let total_tags: usize = pruned.iter().map(|p| p.removed_keys.len()).sum();
    let total_actions = result.killed_orphan_children.len()
        + result.reap_skipped.len()
        + result.removed.len()
        + total_tags;
    if total_actions == 0 {
        println!(
            "{}",
            if dry_run { "Nothing would be cleaned up." } else { "Nothing to clean up." }
        );
        return Ok(0);
    }

    let plural = |n: usize, one: &str, many: &str| if n == 1 { one.to_string() } else { many.to_string() };
    let mut parts: Vec<String> = Vec::new();
    let n = result.killed_orphan_children.len();
    if n > 0 {
        parts.push(format!("{n} orphan {}", plural(n, "child", "children")));
    }
    let n = result.reap_skipped.len();
    if n > 0 {
        parts.push(format!("{n} reap {}", plural(n, "skip", "skips")));
    }
    let n = result.removed.len();
    if n > 0 {
        parts.push(format!("{n} stale {}", plural(n, "session", "sessions")));
    }
    if total_tags > 0 {
        parts.push(format!(
            "{total_tags} orphan {}",
            plural(total_tags, "tag", "tags")
        ));
    }
    if dry_run {
        println!("Would clean up {}. (Dry run — no changes made.)", parts.join(", "));
    } else {
        println!("Cleaned up {}.", parts.join(", "));
    }
    Ok(0)
}

/// The pass.
///
/// node: src/sessions.ts:1521-1724
fn gc(dry_run: bool) -> GcResult {
    let mut result = GcResult::default();

    // Raw debris: runtime files whose metadata is missing or malformed.
    let raw_candidates = inventory_raw_cleanup_candidates(None);
    if dry_run {
        result.removed.extend(raw_candidates.iter().cloned());
    } else {
        for name in &raw_candidates {
            if cleanup_raw_candidate_guarded(name) {
                result.removed.push(name.clone());
            }
        }
    }
    let initial = registry::list_sessions();

    // STEP 1: orphan children, in name order so cycles resolve
    // deterministically.
    let mut with_parent: Vec<&SessionInfo> = initial
        .iter()
        .filter(|s| parent_of(s).is_some())
        .collect();
    with_parent.sort_by(|a, b| a.name.cmp(&b.name));
    for s in with_parent {
        let parent = parent_of(s).unwrap_or_default();
        let parent_meta = read_metadata(&parent);
        let parent_pid = parent_meta
            .as_ref()
            .and_then(|m| read_pid_with(&parent, Some(m)));
        let parent_alive = parent_meta.is_some() && parent_pid.is_some_and(pid_alive);
        if parent_alive {
            continue;
        }
        let reason = if parent_meta.is_some() { "dead" } else { "missing" };
        if !dry_run
            && let Reap::Skipped { reason, signalled } = reap_observed_session(s)
        {
            result.reap_skipped.push(ReapSkip {
                name: s.name.clone(),
                operation: "orphan",
                reason,
                signalled,
            });
            continue;
        }
        result.killed_orphan_children.push(OrphanKill {
            name: s.name.clone(),
            parent,
            reason,
        });
    }

    // STEP 3: the historic sweep. Exited/vanished non-permanent sessions
    // lose their metadata; `keep` exempts.
    let final_list = if dry_run { initial } else { registry::list_sessions() };
    for s in &final_list {
        if !s.is_gone() {
            continue;
        }
        let tags = s.metadata.as_ref().and_then(|m| m.tags.as_ref());
        if tags.and_then(|t| t.get("strategy")).map(String::as_str) == Some("permanent") {
            continue;
        }
        if is_keep_requested(tags) {
            result.kept.push(s.name.clone());
            continue;
        }
        if dry_run || cleanup_observed_session(s) {
            result.removed.push(s.name.clone());
        }
    }
    result
}

fn parent_of(s: &SessionInfo) -> Option<String> {
    s.metadata
        .as_ref()
        .and_then(|m| m.tags.as_ref())
        .and_then(|t| t.get("parent"))
        .filter(|p| !p.is_empty())
        .cloned()
}

/// Registry debris that `list_sessions` cannot represent: `.sock`/`.pid`
/// with a dead pid and missing/malformed metadata (socket unreachable), or
/// malformed metadata with no runtime files at all.
///
/// node: src/sessions.ts:643-705
fn inventory_raw_cleanup_candidates(only: Option<&str>) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(session_dir()) else {
        return Vec::new();
    };
    let entries: BTreeSet<String> = dir
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    let mut names: BTreeSet<String> = BTreeSet::new();
    for entry in &entries {
        let name = entry
            .strip_suffix(".events.jsonl")
            .or_else(|| entry.strip_suffix(".sock"))
            .or_else(|| entry.strip_suffix(".pid"))
            .or_else(|| entry.strip_suffix(".json"));
        if let Some(name) = name
            && only.is_none_or(|o| o == name)
        {
            names.insert(name.to_string());
        }
    }

    struct Candidate {
        name: String,
        has_socket: bool,
        has_pid: bool,
        has_metadata: bool,
        pid_dead: bool,
    }
    let candidates: Vec<Candidate> = names
        .into_iter()
        .filter_map(|name| {
            let has_socket = entries.contains(&format!("{name}.sock"));
            let has_pid = entries.contains(&format!("{name}.pid"));
            let has_metadata = entries.contains(&format!("{name}.json"));
            if has_metadata && !metadata_is_malformed(&name) {
                return None;
            }
            let pid = if has_pid { read_pid(&name) } else { None };
            let pid_dead = pid.is_some_and(|p| !pid_alive(p));
            Some(Candidate {
                name,
                has_socket,
                has_pid,
                has_metadata,
                pid_dead,
            })
        })
        .collect();

    let to_probe: Vec<PathBuf> = candidates
        .iter()
        .filter(|c| c.has_socket && c.pid_dead)
        .map(|c| socket_path(&c.name))
        .collect();
    let reachability: HashMap<PathBuf, bool> =
        probe_sockets_within_budget(&to_probe, DEFAULT_SOCKET_PROBE_BUDGET);

    candidates
        .into_iter()
        .filter(|c| {
            if c.pid_dead {
                return !c.has_socket || reachability.get(&socket_path(&c.name)) == Some(&false);
            }
            c.has_metadata && !c.has_pid && !c.has_socket
        })
        .map(|c| c.name)
        .collect()
}

/// "malformed": the file exists but is not a JSON object (an unreadable
/// file is retained, as Node does).
///
/// node: src/sessions.ts:620-634
fn metadata_is_malformed(name: &str) -> bool {
    match std::fs::read(metadata_path(name)) {
        Ok(bytes) => !matches!(
            serde_json::from_slice::<serde_json::Value>(&bytes),
            Ok(serde_json::Value::Object(_))
        ),
        Err(_) => false,
    }
}

/// Remove one raw candidate while owning both locks, re-inventorying under
/// the lock so a generation that appeared meanwhile is left alone.
///
/// node: src/sessions.ts:707-753
fn cleanup_raw_candidate_guarded(name: &str) -> bool {
    with_both_locks(name, || {
        if !inventory_raw_cleanup_candidates(Some(name))
            .iter()
            .any(|n| n == name)
        {
            return false;
        }
        cleanup_socket(name);
        let _ = std::fs::remove_file(metadata_path(name));
        let _ = std::fs::remove_file(events_path(name));
        let _ = std::fs::remove_file(recovery_revision_path(name));
        true
    })
    .unwrap_or(false)
}

/// Generation-CAS cleanup of an observed session.
///
/// node: src/sessions.ts:769-790
fn cleanup_observed_session(session: &SessionInfo) -> bool {
    let Some(observed) = &session.metadata else {
        return false;
    };
    with_both_locks(&session.name, || {
        match read_metadata(&session.name) {
            Some(current) if metadata_matches_observation(observed, &current) => {
                cleanup_all_while_locked(&session.name);
                true
            }
            _ => false,
        }
    })
    .unwrap_or(false)
}

enum Reap {
    Reaped,
    Skipped { reason: &'static str, signalled: bool },
}

/// SIGTERM a session (when running), wait for its daemon, then remove its
/// files — every step under both locks with the observation re-checked.
///
/// node: src/sessions.ts:821-880
fn reap_observed_session(session: &SessionInfo) -> Reap {
    let Some(observed) = &session.metadata else {
        return Reap::Skipped {
            reason: "stale",
            signalled: false,
        };
    };
    let name = &session.name;
    let mut signalled = false;
    let mut signal_failed = false;
    let first = with_both_locks(name, || {
        match read_metadata(name) {
            Some(current) if metadata_matches_observation(observed, &current) => {}
            _ => return false,
        }
        if session.is_running()
            && let Some(pid) = session.pid
        {
            // SAFETY: signalling a pid read from the registry.
            let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
            if rc == 0 {
                signalled = true;
            } else {
                signal_failed = pid_alive(pid);
            }
        }
        true
    });
    match first {
        Err(_) => {
            return Reap::Skipped {
                reason: "busy",
                signalled: false,
            };
        }
        Ok(false) => {
            return Reap::Skipped {
                reason: "stale",
                signalled: false,
            };
        }
        Ok(true) => {}
    }
    if signal_failed {
        return Reap::Skipped {
            reason: "signal-failed",
            signalled: false,
        };
    }
    if signalled && let Some(pid) = session.pid {
        let deadline = Instant::now() + Duration::from_secs(7);
        while Instant::now() < deadline && !has_process_exited_for_reap(pid) {
            std::thread::sleep(Duration::from_millis(25));
        }
        if !has_process_exited_for_reap(pid) {
            return Reap::Skipped {
                reason: "shutdown-timeout",
                signalled: true,
            };
        }
    }
    let second = with_both_locks(name, || {
        match read_metadata(name) {
            Some(current) if metadata_matches_observation(observed, &current) => {}
            _ => return false,
        }
        cleanup_all_while_locked(name);
        true
    });
    match second {
        Err(_) => Reap::Skipped {
            reason: "busy",
            signalled,
        },
        Ok(false) => Reap::Skipped {
            reason: "stale",
            signalled,
        },
        Ok(true) => Reap::Reaped,
    }
}

/// `:l<pid>-<rand>` → the pid, when the key has that shape.
///
/// node: src/sessions.ts:2026 (`ORPHAN_LAYOUT_TAG_RE`)
fn orphan_layout_tag_pid(key: &str) -> Option<Option<i64>> {
    let rest = key.strip_prefix(":l")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let tail = rest[digits..].strip_prefix('-')?;
    if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) {
        return None;
    }
    Some(rest[..digits].parse::<i64>().ok())
}

/// Drop layout tags whose owning pid is dead on every running session.
///
/// node: src/sessions.ts:2042-2074
fn prune_orphan_layout_tags(dry_run: bool) -> Vec<PrunedTags> {
    let mut out = Vec::new();
    for s in registry::list_sessions() {
        if !s.is_running() {
            continue;
        }
        let Some(tags) = s.metadata.as_ref().and_then(|m| m.tags.as_ref()) else {
            continue;
        };
        let to_remove: Vec<String> = tags
            .keys()
            .filter(|key| match orphan_layout_tag_pid(key) {
                None => false,
                Some(None) => true,
                Some(Some(pid)) => {
                    pid <= 0 || i32::try_from(pid).map(pid_alive).unwrap_or(false) == false
                }
            })
            .cloned()
            .collect();
        if to_remove.is_empty() {
            continue;
        }
        if !dry_run && update_tags(&s.name, &TagMap::new(), &to_remove).is_err() {
            // The metadata disappeared between listing and update.
            continue;
        }
        out.push(PrunedTags {
            name: s.name,
            removed_keys: to_remove,
        });
    }
    out
}

/// `path.basename(root)` with `[^A-Za-z0-9._-]+` → `-` and edge dashes
/// stripped.
///
/// node: src/cli.ts:3215-3222
fn label_basename_from_root(root: &str) -> String {
    let trimmed = root.trim_end_matches('/');
    let base = trimmed.rsplit('/').next().unwrap_or("");
    let mut out = String::new();
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
        } else if !out.ends_with('-') || out.is_empty() {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A launchd plist that runs `pty gc` every `interval` seconds. Node lists
/// `node` and its launcher script as the program; this binary is its own
/// program, so `ProgramArguments` is `[<pty>, gc]`.
///
/// node: src/cli.ts:3224-3276
fn print_launchd_plist(interval: i64) {
    let root = session_dir();
    let root_str = root.to_string_lossy().into_owned();
    let is_default = root == default_session_dir();
    let suffix = if is_default {
        String::new()
    } else {
        format!(".{}", label_basename_from_root(&root_str))
    };
    let label = format!("com.compoundingtech.pty.gc{suffix}");
    let log_path = root.join("gc.log").to_string_lossy().into_owned();
    let pty_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "pty".to_string());
    let env_path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".to_string());
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{bin}</string>
    <string>gc</string>
  </array>
  <key>StartInterval</key>
  <integer>{interval}</integer>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>{path}</string>
    <key>PTY_ROOT</key>
    <string>{root}</string>
  </dict>
</dict>
</plist>
"#,
        label = xml_escape(&label),
        bin = xml_escape(&pty_bin),
        log = xml_escape(&log_path),
        path = xml_escape(&env_path),
        root = xml_escape(&root_str),
    );
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(plist.as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_tag_shape() {
        assert_eq!(orphan_layout_tag_pid(":l1234-abc"), Some(Some(1234)));
        assert_eq!(orphan_layout_tag_pid(":layout"), None);
        assert_eq!(orphan_layout_tag_pid(":l12-"), None);
        assert_eq!(orphan_layout_tag_pid(":l-abc"), None);
        assert_eq!(orphan_layout_tag_pid(":l12-ABC"), None);
    }

    #[test]
    fn label_basename() {
        assert_eq!(label_basename_from_root("/tmp/x/my-network"), "my-network");
        assert_eq!(
            label_basename_from_root("/tmp/weird name with spaces"),
            "weird-name-with-spaces"
        );
        assert_eq!(label_basename_from_root("/tmp/--a b--"), "a-b");
    }
}
