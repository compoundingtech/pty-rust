//! `pty stats` — interim port kept from the v0 binary. The Node-exact
//! rewrite (cli.ts:1345-1356, `cmdStats` 2448-2564, `printStats`
//! 2566-2595) replaces this module.

use pty_core::client;
use pty_core::registry;

use super::{CliResult, resolve_ref};

/// The small gone-session stats shape, from metadata.
fn gone_stats(name: &str, meta: &registry::SessionMetadata) -> pty_core::stats::GoneStats {
    let status = if meta.exit_code.is_some() { "exited" } else { "vanished" };
    pty_core::stats::GoneStats {
        name: name.to_string(),
        status: status.to_string(),
        exit_code: meta.exit_code,
        exited_at: meta.exited_at.clone(),
        tags: meta
            .tags
            .as_ref()
            .map(|t| t.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
    }
}

/// `pty stats [--json] [--all] [<ref>]` — a running session emits the full
/// StatsResult (queried from the daemon); a gone session emits the small
/// shape; with no ref, an array of all (gone entries only with --all).
pub fn run(args: &[String]) -> CliResult {
    let all = args.iter().any(|a| a == "--all");
    let reference = args.iter().find(|a| !a.starts_with('-')).cloned();

    if let Some(reference) = reference {
        let name = resolve_ref(&reference)?;
        if client::is_alive(&name)
            && let Ok(json) = client::query_status_json(&name, client::STATS_TIMEOUT)
            && !json.is_empty()
        {
            println!("{json}");
            return Ok(0);
        }
        if let Some(meta) = registry::read_metadata(&name) {
            println!("{}", serde_json::to_string(&gone_stats(&name, &meta)).unwrap());
            return Ok(0);
        }
        eprintln!("Session \"{reference}\" not found.");
        return Ok(1);
    }

    let mut items: Vec<String> = Vec::new();
    for s in registry::list_sessions() {
        if s.is_running() {
            match client::query_status_json(&s.name, client::STATS_TIMEOUT) {
                Ok(json) if !json.is_empty() => items.push(json),
                _ => items.push(format!(
                    "{{\"name\":{},\"error\":\"query failed\"}}",
                    serde_json::to_string(&s.name).unwrap()
                )),
            }
        } else if all {
            let meta = s.metadata.clone().unwrap_or_default();
            items.push(serde_json::to_string(&gone_stats(&s.name, &meta)).unwrap());
        }
    }
    println!("[{}]", items.join(","));
    Ok(0)
}
