//! `pty gc [-n|--dry-run] [--idle-days N] [--keep-max-age <dur>]
//! [--fast-fail-window N] [--fast-fail-limit N]`: reclaim registry debris,
//! kill orphaned `parent=<id>` children, reap abandoned permanent sessions,
//! respawn stopped permanent sessions with crash-loop protection, sweep
//! exited/vanished sessions (honouring `keep` until it expires), and prune
//! dead `:l<pid>-<rand>` layout tags.
//! `pty gc --print-launchd-plist [--interval N]` prints a launchd job.
//!
//! node: src/cli.ts:1234-1275 (parsing), 2544-2641 (`cmdGc`), 2643-2695
//! (`printLaunchdPlist`); src/sessions.ts:487-897 (`gc` and classifiers),
//! 899-982 (`respawnPermanent`), 1596-1806 (raw debris and observed cleanup),
//! 2140-2189 (`pruneOrphanLayoutTags`)

use std::io::Write;

use pty_core::duration::{format_duration, parse_duration};
use pty_core::events::AbandonReason;
use pty_core::registry::{DEFAULT_KEEP_MAX_AGE_MS, default_session_dir, session_dir};
use pty_lifecycle::{GcOptions, gc, prune_orphan_layout_tags};

use super::argv::js_parse_int;
use super::{CliError, CliResult};

/// Parse and run.
///
/// node: src/cli.ts:1234-1275
pub fn run(gc_args: &[String]) -> CliResult {
    let dry_run = gc_args.iter().any(|a| a == "--dry-run" || a == "-n");
    let print_plist = gc_args.iter().any(|a| a == "--print-launchd-plist");
    let mut interval: i64 = 30;
    let mut keep_max_age_ms = DEFAULT_KEEP_MAX_AGE_MS;
    let mut idle_days = None;
    let mut fast_fail_window = None;
    let mut fast_fail_limit = None;
    let parse_positive = |flag: &str, raw: &str| -> Result<i64, CliError> {
        match js_parse_int(raw) {
            Some(v) if v > 0 => Ok(v),
            _ => Err(CliError(format!(
                "pty gc: {flag} expects a positive integer (got \"{raw}\")"
            ))),
        }
    };
    // Durations, unlike the integer flags, have a meaningful zero: `0` means
    // "the keep exemption is over, sweep the backlog now". The unit-less
    // spelling is accepted only for zero, since `--keep-max-age 7` would
    // otherwise be ambiguous between seconds and days.
    let parse_age = |flag: &str, raw: &str| -> Result<i64, CliError> {
        if raw.trim() == "0" {
            return Ok(0);
        }
        parse_duration(raw).ok_or_else(|| {
            CliError(format!(
                "pty gc: {flag} expects a duration like 12h, 7d, or 0 (got \"{raw}\")"
            ))
        })
    };
    let mut i = 0;
    while i < gc_args.len() {
        let a = gc_args[i].as_str();
        if a == "--interval" && i + 1 < gc_args.len() {
            i += 1;
            interval = parse_positive("--interval", &gc_args[i])?;
        } else if let Some(raw) = a.strip_prefix("--interval=") {
            interval = parse_positive("--interval", raw)?;
        } else if a == "--keep-max-age" && i + 1 < gc_args.len() {
            i += 1;
            keep_max_age_ms = parse_age("--keep-max-age", &gc_args[i])?;
        } else if let Some(raw) = a.strip_prefix("--keep-max-age=") {
            keep_max_age_ms = parse_age("--keep-max-age", raw)?;
        } else if a == "--idle-days" && i + 1 < gc_args.len() {
            i += 1;
            idle_days = Some(parse_positive("--idle-days", &gc_args[i])?);
        } else if let Some(raw) = a.strip_prefix("--idle-days=") {
            idle_days = Some(parse_positive("--idle-days", raw)?);
        } else if a == "--fast-fail-window" && i + 1 < gc_args.len() {
            i += 1;
            fast_fail_window = Some(parse_positive("--fast-fail-window", &gc_args[i])?);
        } else if let Some(raw) = a.strip_prefix("--fast-fail-window=") {
            fast_fail_window = Some(parse_positive("--fast-fail-window", raw)?);
        } else if a == "--fast-fail-limit" && i + 1 < gc_args.len() {
            i += 1;
            fast_fail_limit = Some(parse_positive("--fast-fail-limit", &gc_args[i])?);
        } else if let Some(raw) = a.strip_prefix("--fast-fail-limit=") {
            fast_fail_limit = Some(parse_positive("--fast-fail-limit", raw)?);
        }
        i += 1;
    }
    if print_plist {
        print_launchd_plist(interval);
        return Ok(0);
    }
    cmd_gc(
        dry_run,
        keep_max_age_ms,
        idle_days,
        fast_fail_window,
        fast_fail_limit,
    )
}

/// `cmdGc`.
///
/// node: src/cli.ts:2544-2641
fn cmd_gc(
    dry_run: bool,
    keep_max_age_ms: i64,
    idle_days: Option<i64>,
    fast_fail_window: Option<i64>,
    fast_fail_limit: Option<i64>,
) -> CliResult {
    let result = gc(GcOptions {
        dry_run,
        keep_max_age_ms,
        idle_days,
        fast_fail_window,
        fast_fail_limit,
        daemon_executable: std::env::current_exe().ok(),
    });
    let pruned = prune_orphan_layout_tags(dry_run);

    let killed_verb = if dry_run {
        "Would kill orphan child"
    } else {
        "Killed orphan child"
    };
    let abandon_verb = if dry_run {
        "Would abandon"
    } else {
        "Abandoned"
    };
    let respawn_verb = if dry_run {
        "Would respawn"
    } else {
        "Respawned"
    };
    let flap_verb = if dry_run { "Would flap" } else { "Flapping" };
    let remove_verb = if dry_run { "Would remove" } else { "Removed" };
    let pruned_verb = if dry_run { "Would prune" } else { "Pruned" };

    for k in &result.killed_orphan_children {
        println!(
            "{killed_verb}: {} (parent {} {})",
            k.name, k.parent, k.reason
        );
    }
    for a in &result.abandoned {
        if a.reason == AbandonReason::Idle {
            println!(
                "{abandon_verb}: {} (idle {}d)",
                a.name,
                a.idle_days.unwrap_or(0)
            );
        } else {
            println!("{abandon_verb}: {} ({})", a.name, a.reason.as_str());
        }
    }
    for r in &result.respawned {
        println!(
            "{respawn_verb}: {}{}",
            r.name,
            if r.ptyfile_reread {
                " (pty.toml re-read)"
            } else {
                ""
            }
        );
    }
    for f in &result.respawn_failed {
        println!("Respawn failed: {} — {}", f.name, f.error);
    }
    for f in &result.flapped {
        println!(
            "{flap_verb}: {} ({} fast-fails in {}s, limit {})",
            f.name, f.counter, f.window, f.limit
        );
    }
    for name in &result.flapping_skipped {
        println!("Skipped (flapping): {name} — remove strategy.status tag to retry");
    }
    for s in &result.reap_skipped {
        let phase = if s.signalled {
            "after signalling"
        } else {
            "before signalling"
        };
        println!(
            "Skipped {} reap: {} ({}, {phase})",
            s.operation, s.name, s.reason
        );
    }
    for name in &result.removed {
        println!("{remove_verb}: {name}");
    }
    // Reported apart from the plain sweep above: an operator who tagged
    // these sessions asked for them to survive, so the reason they went away
    // anyway has to be visible rather than looking like the keep tag was
    // ignored.
    let keep_window = format_duration(keep_max_age_ms);
    for name in &result.keep_expired {
        println!("{remove_verb} (keep expired after {keep_window}): {name}");
    }
    // A kept session is not an action; it is printed so "why is this dead
    // session still listed?" has a visible answer, naming the window it is
    // counting down.
    for name in &result.kept {
        println!(
            "Kept (keep tag): {name} — swept once dead for {keep_window}, or remove the keep tag to reap it now"
        );
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
        + result.abandoned.len()
        + result.respawned.len()
        + result.respawn_failed.len()
        + result.flapped.len()
        + result.flapping_skipped.len()
        + result.reap_skipped.len()
        + result.removed.len()
        + result.keep_expired.len()
        + total_tags;
    if total_actions == 0 {
        println!(
            "{}",
            if dry_run {
                "Nothing would be cleaned up."
            } else {
                "Nothing to clean up."
            }
        );
        return Ok(0);
    }

    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            one.to_string()
        } else {
            many.to_string()
        }
    };
    let mut parts: Vec<String> = Vec::new();
    let n = result.killed_orphan_children.len();
    if n > 0 {
        parts.push(format!("{n} orphan {}", plural(n, "child", "children")));
    }
    let n = result.abandoned.len();
    if n > 0 {
        parts.push(format!("{n} abandoned"));
    }
    let n = result.respawned.len();
    if n > 0 {
        parts.push(format!("{n} {}", plural(n, "respawn", "respawns")));
    }
    let n = result.respawn_failed.len();
    if n > 0 {
        parts.push(format!("{n} respawn {}", plural(n, "failure", "failures")));
    }
    let n = result.flapped.len();
    if n > 0 {
        parts.push(format!("{n} flapping"));
    }
    let n = result.flapping_skipped.len();
    if n > 0 {
        parts.push(format!("{n} skipped-flapping"));
    }
    let n = result.reap_skipped.len();
    if n > 0 {
        parts.push(format!("{n} reap {}", plural(n, "skip", "skips")));
    }
    let n = result.removed.len();
    if n > 0 {
        parts.push(format!("{n} stale {}", plural(n, "session", "sessions")));
    }
    let n = result.keep_expired.len();
    if n > 0 {
        parts.push(format!(
            "{n} keep-expired {}",
            plural(n, "session", "sessions")
        ));
    }
    if total_tags > 0 {
        parts.push(format!(
            "{total_tags} orphan {}",
            plural(total_tags, "tag", "tags")
        ));
    }
    if dry_run {
        println!(
            "Would clean up {}. (Dry run — no changes made.)",
            parts.join(", ")
        );
    } else {
        println!("Cleaned up {}.", parts.join(", "));
    }
    Ok(0)
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
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
    let env_path =
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".to_string());
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
    fn label_basename() {
        assert_eq!(label_basename_from_root("/tmp/x/my-network"), "my-network");

        assert_eq!(
            label_basename_from_root("/tmp/weird name with spaces"),
            "weird-name-with-spaces"
        );
        assert_eq!(label_basename_from_root("/tmp/--a b--"), "a-b");
    }
}
