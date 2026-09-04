//! Tag rules shared by the CLI, the daemon and gc: the reserved-key rule,
//! `--filter-tag` matching, the `keep` tag, exit-time reap precedence, and
//! the gc bookkeeping keys a manual restart strips.
//!
//! node: src/tags.ts; src/sessions.ts:1020-1097; src/cli.ts:4081-4100

use super::metadata::TagMap;

/// Keys pty itself treats as bookkeeping and hides from the default
/// listing (`pty list --tags` shows them).
pub const EXACT_RESERVED_TAG_KEYS: &[&str] =
    &["ptyfile", "ptyfile.session", "ptyfile.tags", "strategy"];

/// Reserved: one of [`EXACT_RESERVED_TAG_KEYS`], or any key starting with
/// `:` (tool-owned tags such as pty-layout's `:l<pid>-<rand>`).
///
/// node: src/tags.ts:56-79
pub fn is_reserved_tag_key(key: &str) -> bool {
    EXACT_RESERVED_TAG_KEYS.contains(&key) || key.starts_with(':')
}

/// `true` when `session_tags` contains every `key=value` of `filter` (AND).
/// An empty filter always matches; a session without tags matches only an
/// empty filter.
///
/// node: src/tags.ts:38-46
pub fn matches_all_tags(session_tags: Option<&TagMap>, filter: &TagMap) -> bool {
    filter
        .iter()
        .all(|(k, v)| session_tags.and_then(|t| t.get(k)) == Some(v))
}

/// Pull every `--filter-tag key=value` pair out of `args` (consumed in
/// place, repeatable). Errors with Node's text when the value is missing or
/// has no `=`.
///
/// node: src/tags.ts:17-31
pub fn extract_filter_tags(args: &mut Vec<String>) -> Result<TagMap, String> {
    let mut tags = TagMap::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] != "--filter-tag" {
            i += 1;
            continue;
        }
        let kv = args.get(i + 1).cloned();
        let Some(kv) = kv.filter(|kv| kv.contains('=')) else {
            return Err("--filter-tag expects \"key=value\"".to_string());
        };
        let (k, v) = kv.split_once('=').expect("checked");
        tags.insert(k.to_string(), v.to_string());
        args.drain(i..i + 2);
    }
    Ok(tags)
}

/// Tag key that exempts a session from every form of dead-session reaping.
///
/// node: src/sessions.ts:1020
pub const KEEP_TAG: &str = "keep";

/// Values that read as "no" for `keep` and `PTY_REAP_ON_EXIT`.
pub const KEEP_FALSEY: &[&str] = &["false", "0", "no", "off"];

fn reads_as_no(raw: &str) -> bool {
    let normalized = super::names::js_trim(raw).to_lowercase();
    KEEP_FALSEY.contains(&normalized.as_str())
}

/// `true` when `tags` asks for the session to be retained after death: the
/// `keep` key is present with any value other than `false|0|no|off` (after
/// trim + lowercase).
///
/// node: src/sessions.ts:1040-1044
pub fn is_keep_requested(tags: Option<&TagMap>) -> bool {
    match tags.and_then(|t| t.get(KEEP_TAG)) {
        None => false,
        Some(raw) => !reads_as_no(raw),
    }
}

/// The config default for exit-time reaping: `PTY_REAP_ON_EXIT` unset →
/// reap; `false|0|no|off` → preserve; anything else → reap.
///
/// node: src/sessions.ts:1091-1097
pub fn reap_on_exit_default() -> bool {
    match std::env::var("PTY_REAP_ON_EXIT") {
        Err(_) => true,
        Ok(raw) => !reads_as_no(&raw),
    }
}

/// Should the daemon remove its own registry entry as it shuts down?
/// Precedence, highest first: `keep` → preserve; `ephemeral` → reap;
/// `strategy=permanent` → preserve; else `default_reap`.
///
/// node: src/sessions.ts:1069-1089
pub fn should_reap_at_exit(tags: Option<&TagMap>, ephemeral: bool, default_reap: bool) -> bool {
    if is_keep_requested(tags) {
        return false;
    }
    if ephemeral {
        return true;
    }
    if tags.and_then(|t| t.get("strategy")).map(String::as_str) == Some("permanent") {
        return false;
    }
    default_reap
}

/// `pty gc`'s flapping bookkeeping; a manual `restart` / `up` strips them.
///
/// node: src/cli.ts:3667-3672, 4081-4100
pub const GC_BOOKKEEPING_KEYS: &[&str] = &[
    "strategy.status",
    "strategy.consecutive-fast-fails",
    "strategy.last-respawn-at",
    "strategy.command-hash",
];

/// Remove the [`GC_BOOKKEEPING_KEYS`] from `tags`; `None` when nothing is
/// left (so callers skip the `tags` field entirely, as Node does).
///
/// node: src/cli.ts:4088-4100
pub fn strip_gc_bookkeeping(tags: Option<&TagMap>) -> Option<TagMap> {
    let tags = tags?;
    if tags.is_empty() {
        return Some(tags.clone());
    }
    let out: TagMap = tags
        .iter()
        .filter(|(k, _)| !GC_BOOKKEEPING_KEYS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    (!out.is_empty()).then_some(out)
}
