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

use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pty_core::duration::{format_duration, parse_duration};
use pty_core::events::{AbandonReason, Event, append_event_locked};
use pty_core::ptyfile;
use pty_core::registry::{
    self, DEFAULT_KEEP_MAX_AGE_MS, DEFAULT_SOCKET_PROBE_BUDGET, SessionInfo, SessionMetadata,
    TagMap, apply_metadata_diff, cleanup_all_while_locked, cleanup_socket, default_session_dir,
    event_lock_path, events_path, has_process_exited_for_reap, is_keep_expired, is_keep_requested,
    lock_or_refusal, metadata_matches_observation, metadata_path, now_epoch_ms, parse_iso8601_ms,
    pid_alive, probe_sockets_within_budget, read_metadata, read_metadata_map, read_pid,
    read_pid_with, recovery_revision_path, session_dir, socket_path, socket_reachable, update_tags,
    with_both_locks, write_metadata, write_metadata_map,
};
use sha2::{Digest, Sha256};

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

/// What one pass did (or would do).
#[derive(Debug, Default)]
struct GcResult {
    removed: Vec<String>,
    kept: Vec<String>,
    /// Dead sessions swept despite a `keep` tag because they outlived the
    /// retention window. Disjoint from `removed`, which holds the untagged
    /// sweep, so the two reasons are reported apart.
    keep_expired: Vec<String>,
    killed_orphan_children: Vec<OrphanKill>,
    abandoned: Vec<Abandoned>,
    respawned: Vec<Respawned>,
    respawn_failed: Vec<RespawnFailed>,
    flapped: Vec<Flapped>,
    flapping_skipped: Vec<String>,
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

#[derive(Debug)]
struct Abandoned {
    name: String,
    reason: AbandonReason,
    idle_days: Option<i64>,
}

#[derive(Debug)]
struct Respawned {
    name: String,
    ptyfile_reread: bool,
}

#[derive(Debug)]
struct RespawnFailed {
    name: String,
    error: String,
}

#[derive(Debug)]
struct Flapped {
    name: String,
    counter: i64,
    limit: i64,
    window: i64,
}

/// One session whose tags were (or would be) pruned.
#[derive(Debug)]
struct PrunedTags {
    name: String,
    removed_keys: Vec<String>,
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
    let result = gc(
        dry_run,
        keep_max_age_ms,
        idle_days,
        fast_fail_window,
        fast_fail_limit,
    );
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

/// One stateless reconciliation pass.
///
/// node: src/sessions.ts:487-726
fn gc(
    dry_run: bool,
    keep_max_age_ms: i64,
    global_idle_days: Option<i64>,
    global_fast_fail_window: Option<i64>,
    global_fast_fail_limit: Option<i64>,
) -> GcResult {
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
    let mut with_parent: Vec<&SessionInfo> =
        initial.iter().filter(|s| parent_of(s).is_some()).collect();
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
        let reason = if parent_meta.is_some() {
            "dead"
        } else {
            "missing"
        };
        if dry_run {
            result.killed_orphan_children.push(OrphanKill {
                name: s.name.clone(),
                parent,
                reason,
            });
            continue;
        }
        match reap_observed_session(s, ReapMode::Orphan) {
            Reap::Reaped(_) => result.killed_orphan_children.push(OrphanKill {
                name: s.name.clone(),
                parent,
                reason,
            }),
            Reap::NotEligible => {}
            Reap::Skipped { reason, signalled } => result.reap_skipped.push(ReapSkip {
                name: s.name.clone(),
                operation: "orphan",
                reason,
                signalled,
            }),
        }
    }

    // STEP 1.5: abandoned permanent sessions. This precedes respawn so a
    // cwd-gone or idle session cannot be recreated during the same pass.
    let after_step1 = if dry_run {
        initial.clone()
    } else {
        registry::list_sessions()
    };
    let now_ms = now_epoch_ms();
    for s in &after_step1 {
        let Some(meta) = s.metadata.as_ref() else {
            continue;
        };
        if !is_permanent(meta) {
            continue;
        }
        let Some(decision) = classify_abandoned(meta, global_idle_days, now_ms) else {
            continue;
        };
        if dry_run {
            result.abandoned.push(Abandoned {
                name: s.name.clone(),
                reason: decision.reason,
                idle_days: decision.idle_days,
            });
            continue;
        }
        match reap_observed_session(s, ReapMode::Abandoned { global_idle_days }) {
            Reap::Reaped(Some(decision)) => result.abandoned.push(Abandoned {
                name: s.name.clone(),
                reason: decision.reason,
                idle_days: decision.idle_days,
            }),
            Reap::Reaped(None) | Reap::NotEligible => {}
            Reap::Skipped { reason, signalled } => result.reap_skipped.push(ReapSkip {
                name: s.name.clone(),
                operation: "abandoned",
                reason,
                signalled,
            }),
        }
    }

    // STEP 2: permanent respawn. A dry run classifies the initial snapshots
    // without mutation; a real pass revalidates and serializes each decision
    // under the existing event → creation lock order.
    let excluded: BTreeSet<String> = result
        .abandoned
        .iter()
        .map(|a| a.name.clone())
        .chain(result.killed_orphan_children.iter().map(|k| k.name.clone()))
        .collect();
    let after_step15 = if dry_run {
        initial.clone()
    } else {
        registry::list_sessions()
    };
    for s in &after_step15 {
        let Some(meta) = s.metadata.as_ref() else {
            continue;
        };
        if excluded.contains(s.name.as_str()) || !s.is_gone() || !is_permanent(meta) {
            continue;
        }
        if dry_run {
            let ptyfile_reread = meta.tags.as_ref().and_then(|t| t.get("ptyfile")).is_some();
            let decision = classify_flapping(
                meta,
                now_epoch_ms(),
                global_fast_fail_window,
                global_fast_fail_limit,
            );
            match decision.action {
                FlappingAction::Skip => result.flapping_skipped.push(s.name.clone()),
                FlappingAction::Flap => {
                    let d = decision;
                    result.flapped.push(Flapped {
                        name: s.name.clone(),
                        counter: d.counter,
                        limit: d.effective_limit,
                        window: d.effective_window,
                    });
                }
                FlappingAction::Respawn => result.respawned.push(Respawned {
                    name: s.name.clone(),
                    ptyfile_reread,
                }),
            }
            continue;
        }
        match reconcile_permanent(s, global_fast_fail_window, global_fast_fail_limit) {
            PermanentOutcome::Respawned { ptyfile_reread } => {
                result.respawned.push(Respawned {
                    name: s.name.clone(),
                    ptyfile_reread,
                });
            }
            PermanentOutcome::Failed(error) => result.respawn_failed.push(RespawnFailed {
                name: s.name.clone(),
                error,
            }),
            PermanentOutcome::Flapped {
                counter,
                limit,
                window,
            } => result.flapped.push(Flapped {
                name: s.name.clone(),
                counter,
                limit,
                window,
            }),
            PermanentOutcome::SkippedFlapping => {
                result.flapping_skipped.push(s.name.clone());
            }
            PermanentOutcome::Stale => {}
        }
    }

    // STEP 3: exited/vanished non-permanent sessions lose their metadata;
    // `keep` exempts them only until the retention window expires.
    let final_list = if dry_run {
        initial
    } else {
        registry::list_sessions()
    };
    let now_ms = now_epoch_ms();
    for s in &final_list {
        if !s.is_gone() {
            continue;
        }
        let tags = s.metadata.as_ref().and_then(|m| m.tags.as_ref());
        if tags.and_then(|t| t.get("strategy")).map(String::as_str) == Some("permanent") {
            continue;
        }
        let keep_requested = is_keep_requested(tags);
        if keep_requested && !is_keep_expired(s.metadata.as_ref(), now_ms, keep_max_age_ms) {
            result.kept.push(s.name.clone());
            continue;
        }
        if dry_run || cleanup_observed_session(s) {
            let bucket = if keep_requested {
                &mut result.keep_expired
            } else {
                &mut result.removed
            };
            bucket.push(s.name.clone());
        }
    }
    result
}

#[derive(Debug, Clone, Copy)]
struct AbandonDecision {
    reason: AbandonReason,
    idle_days: Option<i64>,
}

fn is_permanent(meta: &SessionMetadata) -> bool {
    meta.tags
        .as_ref()
        .and_then(|t| t.get("strategy"))
        .map(String::as_str)
        == Some("permanent")
}

/// cwd-gone wins over idle; idle needs a valid positive threshold and a
/// parseable `lastAttachAt`.
///
/// node: src/sessions.ts:728-771
fn classify_abandoned(
    meta: &SessionMetadata,
    global_idle_days: Option<i64>,
    now_ms: i64,
) -> Option<AbandonDecision> {
    let tags = meta.tags.as_ref();
    let cwd_opted_out = tags
        .and_then(|t| t.get("strategy.abandon-if-cwd-gone"))
        .map(String::as_str)
        == Some("false");
    if !meta.cwd.is_empty()
        && !cwd_opted_out
        && std::fs::metadata(&meta.cwd).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        return Some(AbandonDecision {
            reason: AbandonReason::CwdGone,
            idle_days: None,
        });
    }

    let tagged = tags
        .and_then(|t| t.get("strategy.idle-days"))
        .and_then(|v| js_parse_int(v))
        .filter(|v| *v > 0);
    let effective = tagged.or(global_idle_days.filter(|v| *v > 0))?;
    let last_attach = parse_iso8601_ms(meta.last_attach_at.as_deref()?)?;
    let age_days = (now_ms - last_attach).div_euclid(86_400_000);
    (age_days >= effective).then_some(AbandonDecision {
        reason: AbandonReason::Idle,
        idle_days: Some(age_days),
    })
}

/// Revalidate the stale list observation against current lock-held metadata.
/// Mutable policy fields are deliberately read from `current`, not from the
/// generation-only observation check.
fn revalidate_abandonment(
    observed: &SessionMetadata,
    current: &SessionMetadata,
    global_idle_days: Option<i64>,
    now_ms: i64,
) -> Option<AbandonDecision> {
    if !metadata_matches_observation(observed, current) || !is_permanent(current) {
        return None;
    }
    classify_abandoned(current, global_idle_days, now_ms)
}

const DEFAULT_FAST_FAIL_WINDOW: i64 = 60;
const DEFAULT_FAST_FAIL_LIMIT: i64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlappingAction {
    Respawn,
    Flap,
    Skip,
}

#[derive(Debug)]
struct FlappingDecision {
    action: FlappingAction,
    effective_window: i64,
    effective_limit: i64,
    counter: i64,
    bookkeeping: TagMap,
}

fn command_fingerprint(command: &str, args: &[String]) -> String {
    let mut hash = Sha256::new();
    hash.update(command.as_bytes());
    hash.update([0_u8]);
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            hash.update([0_u8]);
        }
        hash.update(arg.as_bytes());
    }
    format!("{:x}", hash.finalize())[..16].to_string()
}

/// Classify one stopped permanent session from persisted bookkeeping.
///
/// node: src/sessions.ts:773-897
fn classify_flapping(
    meta: &SessionMetadata,
    now_ms: i64,
    global_window: Option<i64>,
    global_limit: Option<i64>,
) -> FlappingDecision {
    let tags = meta.tags.as_ref();
    let positive_tag = |key: &str| {
        tags.and_then(|t| t.get(key))
            .and_then(|v| js_parse_int(v))
            .filter(|v| *v > 0)
    };
    let effective_window = positive_tag("strategy.fast-fail-window")
        .or(global_window.filter(|v| *v > 0))
        .unwrap_or(DEFAULT_FAST_FAIL_WINDOW);
    let effective_limit = positive_tag("strategy.fast-fail-limit")
        .or(global_limit.filter(|v| *v > 0))
        .unwrap_or(DEFAULT_FAST_FAIL_LIMIT);
    let current_hash = command_fingerprint(&meta.command, &meta.args);
    let stored_hash = tags.and_then(|t| t.get("strategy.command-hash"));
    let command_changed = stored_hash.is_some_and(|h| h != &current_hash);
    let status = tags
        .and_then(|t| t.get("strategy.status"))
        .map(String::as_str);
    let previous_counter = tags
        .and_then(|t| t.get("strategy.consecutive-fast-fails"))
        .and_then(|v| js_parse_int(v))
        .unwrap_or(0);
    if status == Some("flapping") && !command_changed {
        return FlappingDecision {
            action: FlappingAction::Skip,
            effective_window,
            effective_limit,
            counter: previous_counter,
            bookkeeping: TagMap::new(),
        };
    }

    let last_respawn = tags
        .and_then(|t| t.get("strategy.last-respawn-at"))
        .and_then(|v| parse_iso8601_ms(v));
    let exited = meta.exited_at.as_deref().and_then(parse_iso8601_ms);
    let was_fast = last_respawn.zip(exited).is_some_and(|(start, end)| {
        let live_ms = end - start;
        live_ms >= 0 && live_ms < effective_window.saturating_mul(1000)
    });
    let counter = if command_changed {
        0
    } else if was_fast {
        previous_counter.saturating_add(1)
    } else {
        0
    };
    let mut bookkeeping = TagMap::new();
    if counter >= effective_limit {
        bookkeeping.insert("strategy.status".into(), "flapping".into());
        bookkeeping.insert(
            "strategy.consecutive-fast-fails".into(),
            counter.to_string(),
        );
        bookkeeping.insert("strategy.command-hash".into(), current_hash);
        if let Some(last) = tags.and_then(|t| t.get("strategy.last-respawn-at")) {
            bookkeeping.insert("strategy.last-respawn-at".into(), last.clone());
        }
        return FlappingDecision {
            action: FlappingAction::Flap,
            effective_window,
            effective_limit,
            counter,
            bookkeeping,
        };
    }

    bookkeeping.insert(
        "strategy.last-respawn-at".into(),
        pty_core::registry::iso8601_from_epoch_ms(now_ms),
    );
    bookkeeping.insert(
        "strategy.consecutive-fast-fails".into(),
        counter.to_string(),
    );
    bookkeeping.insert("strategy.command-hash".into(), current_hash);
    FlappingDecision {
        action: FlappingAction::Respawn,
        effective_window,
        effective_limit,
        counter,
        bookkeeping,
    }
}

enum PermanentOutcome {
    Respawned {
        ptyfile_reread: bool,
    },
    Failed(String),
    Flapped {
        counter: i64,
        limit: i64,
        window: i64,
    },
    SkippedFlapping,
    Stale,
}

/// Revalidate and serialize one permanent-session decision. The creation
/// lock remains held across cleanup and daemon publication so concurrent gc
/// invocations cannot both replace the same stable id. The event lock is
/// released before spawn because the new daemon publishes `session_start`.
fn reconcile_permanent(
    observed: &SessionInfo,
    global_window: Option<i64>,
    global_limit: Option<i64>,
) -> PermanentOutcome {
    let name = &observed.name;
    let event_guard = match lock_or_refusal(&event_lock_path(name)) {
        Ok(guard) => guard,
        Err(_) => return PermanentOutcome::Stale,
    };
    let creation_guard = match lock_or_refusal(&registry::lock_path(name)) {
        Ok(guard) => guard,
        Err(_) => {
            event_guard.release();
            return PermanentOutcome::Stale;
        }
    };
    let Some(current) = read_metadata(name) else {
        return PermanentOutcome::Stale;
    };
    if observed
        .metadata
        .as_ref()
        .is_none_or(|old| !metadata_matches_observation(old, &current))
        || !is_permanent(&current)
        || read_pid_with(name, Some(&current)).is_some_and(pid_alive)
    {
        return PermanentOutcome::Stale;
    }

    let now_ms = now_epoch_ms();
    let decision = classify_flapping(&current, now_ms, global_window, global_limit);
    match decision.action {
        FlappingAction::Skip => PermanentOutcome::SkippedFlapping,
        FlappingAction::Flap => {
            let Some(mut raw) = read_metadata_map(name) else {
                return PermanentOutcome::Stale;
            };
            let Some(before) = SessionMetadata::from_map(raw.clone()) else {
                return PermanentOutcome::Stale;
            };
            if !metadata_matches_observation(&current, &before) {
                return PermanentOutcome::Stale;
            }
            let mut after = before.clone();
            let tags = after.tags.get_or_insert_with(TagMap::new);
            for (key, value) in &decision.bookkeeping {
                tags.insert(key.clone(), value.clone());
            }
            apply_metadata_diff(&before, &after, &mut raw);
            if write_metadata_map(name, &raw).is_err() {
                return PermanentOutcome::Stale;
            }
            let event = Event::session_flapping(
                name,
                decision.counter as u64,
                decision.effective_limit as u64,
                decision.effective_window as u64,
            );
            let _ = append_event_locked(name, &event);
            PermanentOutcome::Flapped {
                counter: decision.counter,
                limit: decision.effective_limit,
                window: decision.effective_window,
            }
        }
        FlappingAction::Respawn => {
            let (params, ptyfile_reread) = respawn_params(name, &current, &decision.bookkeeping);
            if let Err(error) = pty_core::spawn::resolve_command(&params.command) {
                return PermanentOutcome::Failed(error);
            }
            let mut retry_meta = current.clone();
            retry_meta.tags = (!params.tags.is_empty()).then(|| params.tags.clone());
            cleanup_all_while_locked(name);
            event_guard.release();
            let spawn = crate::daemon::spawn_daemon(params).map_err(|e| e.to_string());
            if spawn.is_err() && read_metadata(name).is_none() {
                // Preserve the desired state when launch failed before a new
                // daemon could publish. The next gc tick must be able to retry.
                let _ = write_metadata(name, &retry_meta);
            }
            creation_guard.release();
            match spawn {
                Ok(_) => PermanentOutcome::Respawned { ptyfile_reread },
                Err(error) => PermanentOutcome::Failed(error),
            }
        }
    }
}

/// Rebuild launch parameters from last-known-good metadata, overlaying a
/// fresh pty.toml definition when the recorded binding still resolves.
///
/// node: src/sessions.ts:899-982
fn respawn_params(
    name: &str,
    meta: &SessionMetadata,
    bookkeeping: &TagMap,
) -> (super::SpawnParams, bool) {
    let mut command = meta.command.clone();
    let mut args = meta.args.clone();
    let mut display_command = meta.display_command.clone();
    let mut cwd = meta.cwd.clone();
    let mut tags = meta.tags.clone().unwrap_or_default();
    let ptyfile_path = tags.get("ptyfile").cloned();
    let ptyfile_session = tags.get("ptyfile.session").cloned();
    let ptyfile_reread = ptyfile_path.is_some();
    if let (Some(path), Some(short_name)) = (&ptyfile_path, &ptyfile_session)
        && let Some(dir) = Path::new(path).parent()
        && let Ok(file) = ptyfile::read_pty_file(Some(dir))
        && let Some(session) = file.sessions.iter().find(|s| &s.short_name == short_name)
    {
        command = "/bin/sh".into();
        args = vec!["-c".into(), ptyfile::command_with_env_exports(session)];
        display_command = session.command.clone();
        cwd = session
            .cwd
            .clone()
            .unwrap_or_else(|| file.dir.to_string_lossy().into_owned());
        tags = session.tags.clone().unwrap_or_default();
        tags.insert("ptyfile".into(), path.clone());
        tags.insert("ptyfile.session".into(), short_name.clone());
    }
    for (key, value) in bookkeeping {
        tags.insert(key.clone(), value.clone());
    }
    if !bookkeeping.contains_key("strategy.status") {
        tags.shift_remove("strategy.status");
    }

    let mut params = super::SpawnParams::new(name, &command, &args);
    params.display_command = display_command;
    params.cwd = cwd;
    params.tags = tags;
    params.display_name = meta.display_name.clone();
    params.respawn = true;
    (params, ptyfile_reread)
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
    with_both_locks(&session.name, || match read_metadata(&session.name) {
        Some(current) if metadata_matches_observation(observed, &current) => {
            cleanup_all_while_locked(&session.name);
            true
        }
        _ => false,
    })
    .unwrap_or(false)
}

#[derive(Clone, Copy)]
enum ReapMode {
    Orphan,
    Abandoned { global_idle_days: Option<i64> },
}

enum Reap {
    Reaped(Option<AbandonDecision>),
    /// The lock-held metadata no longer satisfies the abandonment policy.
    NotEligible,
    Skipped {
        reason: &'static str,
        signalled: bool,
    },
}

/// SIGTERM a session (when running), wait for its daemon, optionally append
/// an abandonment event, then remove its files. Abandonment is recomputed
/// from lock-held metadata before any signal. A running/socket-reachable
/// observation without a confirmed live pid is never unlinked.
///
/// node: src/sessions.ts:821-880
fn reap_observed_session(session: &SessionInfo, mode: ReapMode) -> Reap {
    let Some(observed) = &session.metadata else {
        return Reap::Skipped {
            reason: "stale",
            signalled: false,
        };
    };
    let name = &session.name;
    let mut abandonment = None;
    let mut signalled_pid = None;
    let first = with_both_locks(name, || -> Result<(), &'static str> {
        let current = read_metadata(name).ok_or("stale")?;
        if let ReapMode::Abandoned { global_idle_days } = mode {
            abandonment =
                revalidate_abandonment(observed, &current, global_idle_days, now_epoch_ms());
            if abandonment.is_none() {
                return Err("not-eligible");
            }
        } else if !metadata_matches_observation(observed, &current) {
            return Err("stale");
        }
        let current_pid =
            read_pid_with(name, Some(&current)).filter(|pid| !has_process_exited_for_reap(*pid));
        let running_now =
            session.is_running() || current_pid.is_some() || socket_reachable(&socket_path(name));
        if running_now {
            let Some(pid) = current_pid else {
                return Err("pid-unavailable");
            };
            // SAFETY: signalling a live pid validated against lock-held
            // metadata and its process-start token.
            let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
            if rc == 0 {
                signalled_pid = Some(pid);
            } else if pid_alive(pid) {
                return Err("signal-failed");
            }
        }
        Ok(())
    });
    match first {
        Err(_) => {
            return Reap::Skipped {
                reason: "busy",
                signalled: false,
            };
        }
        Ok(Err("not-eligible")) => return Reap::NotEligible,
        Ok(Err(reason)) => {
            return Reap::Skipped {
                reason,
                signalled: false,
            };
        }
        Ok(Ok(())) => {}
    }

    if let Some(pid) = signalled_pid {
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
        let current = match read_metadata(name) {
            Some(current) if metadata_matches_observation(observed, &current) => current,
            _ => return false,
        };
        if read_pid_with(name, Some(&current)).is_some_and(|pid| !has_process_exited_for_reap(pid))
            || socket_reachable(&socket_path(name))
        {
            return false;
        }
        if let Some(decision) = abandonment {
            let event = Event::session_abandoned(
                name,
                decision.reason,
                decision.idle_days.map(|days| days as u64),
            );
            let _ = append_event_locked(name, &event);
        }
        cleanup_all_while_locked(name);
        true
    });
    match second {
        Err(_) => Reap::Skipped {
            reason: "busy",
            signalled: signalled_pid.is_some(),
        },
        Ok(false) => Reap::Skipped {
            reason: "stale",
            signalled: signalled_pid.is_some(),
        },
        Ok(true) => Reap::Reaped(abandonment),
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
    if tail.is_empty()
        || !tail
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
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
    fn layout_tag_shape() {
        assert_eq!(orphan_layout_tag_pid(":l1234-abc"), Some(Some(1234)));
        assert_eq!(orphan_layout_tag_pid(":layout"), None);
        assert_eq!(orphan_layout_tag_pid(":l12-"), None);
        assert_eq!(orphan_layout_tag_pid(":l-abc"), None);
        assert_eq!(orphan_layout_tag_pid(":l12-ABC"), None);
    }

    #[test]
    fn abandonment_revalidation_uses_current_policy_fields() {
        let mut tags = TagMap::new();
        tags.insert("strategy".into(), "permanent".into());
        tags.insert("strategy.idle-days".into(), "1".into());
        let observed = SessionMetadata {
            generation: Some("same-generation".into()),
            cwd: "/".into(),
            last_attach_at: Some("1970-01-01T00:00:00.000Z".into()),
            tags: Some(tags),
            ..Default::default()
        };
        let mut current = observed.clone();
        current.last_attach_at = Some("2030-01-01T00:00:00.000Z".into());
        assert!(metadata_matches_observation(&observed, &current));
        assert!(classify_abandoned(&observed, None, 172_800_000).is_some());
        assert!(revalidate_abandonment(&observed, &current, None, 172_800_000).is_none());
        let mut cwd_tags = TagMap::new();
        cwd_tags.insert("strategy".into(), "permanent".into());
        let missing = SessionMetadata {
            generation: Some("cwd-generation".into()),
            cwd: "/definitely/missing/pty-gc-revalidation".into(),
            tags: Some(cwd_tags),
            ..Default::default()
        };
        let mut opted_out = missing.clone();
        opted_out
            .tags
            .as_mut()
            .unwrap()
            .insert("strategy.abandon-if-cwd-gone".into(), "false".into());
        assert!(classify_abandoned(&missing, None, 0).is_some());
        assert!(revalidate_abandonment(&missing, &opted_out, None, 0).is_none());
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
