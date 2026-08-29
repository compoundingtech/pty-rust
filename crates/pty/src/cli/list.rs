//! `pty list` / `pty ls`: the registry as text (with Node's SGR codes) or
//! JSON, with tag / status / age filters, `--summary`, and the `--remote`
//! host groups.
//!
//! node: src/cli.ts:1250-1316 (parsing), 2165-2446 (`cmdList`),
//! 4102-4130 (`strategyMarker`, `shortPath`, `timeAgo`)

use std::collections::HashSet;

use pty_core::duration::{format_duration, parse_duration};
use pty_core::registry::{
    self, SessionInfo, SessionStatus, TagMap, extract_filter_tags, is_reserved_tag_key,
    matches_all_tags, now_epoch_ms, parse_iso8601_ms, short_path, time_ago,
};
use serde_json::{Map, Value};

use super::{CliError, CliResult};

/// One remote host group as `--remote` renders it.
#[derive(Debug, Clone, Default)]
pub struct RemoteHost {
    pub label: String,
    pub sessions: Vec<RemoteSession>,
    pub error: Option<String>,
}

/// One session as a remote host reports it.
#[derive(Debug, Clone, Default)]
pub struct RemoteSession {
    pub name: String,
    pub status: String,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub tags: Option<TagMap>,
    pub display_name: Option<String>,
}

/// The remote host groups for `--remote [<peer>]`: a peer is dialed over
/// fabric, bare `--remote` asks `pty-relay ls --json`. The remote lane
/// supplies this; until then there are no host groups.
///
/// node: src/cli.ts:2223-2247
pub fn remote_list_hosts(_peer: Option<&str>) -> Vec<RemoteHost> {
    Vec::new()
}

/// Everything `cmdList` takes.
#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    pub json: bool,
    pub show_tags: bool,
    pub remote: bool,
    pub remote_peer: Option<String>,
    pub filter_tags: TagMap,
    pub status_filter: Option<SessionStatus>,
    pub older_than_ms: Option<i64>,
    pub newer_than_ms: Option<i64>,
    pub summary: bool,
}

/// Parse `args` (the full argv from the command word on, as Node's
/// `listArgs = args.slice()`) and list.
///
/// node: src/cli.ts:1250-1316
pub fn run(args: &[String]) -> CliResult {
    let mut list_args = args.to_vec();
    let filter_tags = extract_filter_tags(&mut list_args).map_err(CliError)?;

    let mut opts = ListOptions {
        filter_tags,
        ..Default::default()
    };
    let mut consumed: HashSet<usize> = HashSet::new();
    let mut i = 1;
    while i < list_args.len() {
        let arg = list_args[i].as_str();
        let val = list_args.get(i + 1).map(String::as_str);
        match arg {
            "--status" => {
                opts.status_filter = Some(match val {
                    Some("running") => SessionStatus::Running,
                    Some("exited") => SessionStatus::Exited,
                    Some("vanished") => SessionStatus::Vanished,
                    _ => {
                        return Err(CliError(
                            "--status expects one of: running, exited, vanished".to_string(),
                        ));
                    }
                });
                consumed.insert(i);
                consumed.insert(i + 1);
                i += 1;
            }
            "--older-than" | "--newer-than" => {
                let Some(parsed) = val.and_then(parse_duration) else {
                    return Err(CliError(format!(
                        "{arg} expects a duration like 30s, 5m, 2h, 1d"
                    )));
                };
                if arg == "--older-than" {
                    opts.older_than_ms = Some(parsed);
                } else {
                    opts.newer_than_ms = Some(parsed);
                }
                consumed.insert(i);
                consumed.insert(i + 1);
                i += 1;
            }
            "--remote" => {
                opts.remote = true;
                match val {
                    Some(peer) if !peer.starts_with('-') => {
                        opts.remote_peer = Some(peer.to_string());
                        consumed.insert(i);
                        consumed.insert(i + 1);
                        i += 1;
                    }
                    _ => {
                        consumed.insert(i);
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    let remaining: Vec<&str> = list_args
        .iter()
        .enumerate()
        .filter(|(idx, _)| !consumed.contains(idx))
        .map(|(_, a)| a.as_str())
        .collect();
    opts.json = remaining.contains(&"--json");
    opts.show_tags = remaining.contains(&"--tags");
    opts.summary = remaining.contains(&"--summary");
    cmd_list(&opts)
}

/// `displayName ?? name`.
fn sort_key(s: &SessionInfo) -> &str {
    s.metadata
        .as_ref()
        .and_then(|m| m.display_name.as_deref())
        .unwrap_or(&s.name)
}

/// The truthy display name.
fn display_name(s: &SessionInfo) -> Option<&str> {
    s.display_name().filter(|d| !d.is_empty())
}

/// Non-empty `createdAt`.
fn created_at(s: &SessionInfo) -> Option<&str> {
    s.metadata
        .as_ref()
        .map(|m| m.created_at.as_str())
        .filter(|c| !c.is_empty())
}

/// The filtered, sorted sessions.
fn select(opts: &ListOptions) -> Vec<SessionInfo> {
    let mut sessions = registry::list_sessions();
    if !opts.filter_tags.is_empty() {
        sessions.retain(|s| {
            matches_all_tags(
                s.metadata.as_ref().and_then(|m| m.tags.as_ref()),
                &opts.filter_tags,
            )
        });
    }
    if let Some(status) = opts.status_filter {
        sessions.retain(|s| s.status == status);
    }
    if opts.older_than_ms.is_some() || opts.newer_than_ms.is_some() {
        let now = now_epoch_ms();
        sessions.retain(|s| {
            // Anchor age on exitedAt when available (true exit), else
            // createdAt. Sessions with no timestamp cannot be aged, so they
            // are excluded when either age flag is set.
            let anchor = s
                .metadata
                .as_ref()
                .and_then(|m| m.exited_at.as_deref())
                .or_else(|| created_at(s));
            let Some(anchor) = anchor else {
                return false;
            };
            // An unparseable timestamp is NaN in Node: every comparison is
            // false, so the session stays.
            let Some(then) = parse_iso8601_ms(anchor) else {
                return true;
            };
            let age = now - then;
            if let Some(older) = opts.older_than_ms
                && age < older
            {
                return false;
            }
            if let Some(newer) = opts.newer_than_ms
                && age > newer
            {
                return false;
            }
            true
        });
    }
    // Stable display order: ASCII sort on the user-visible label.
    sessions.sort_by(|a, b| sort_key(a).cmp(sort_key(b)));
    sessions
}

struct Summary {
    total: usize,
    running: usize,
    exited: usize,
    vanished: usize,
    oldest: Option<Endpoint>,
    newest: Option<Endpoint>,
}

struct Endpoint {
    name: String,
    status: SessionStatus,
    age_seconds: i64,
    display_name: Option<String>,
}

impl Endpoint {
    fn json(&self) -> Value {
        let mut m = Map::new();
        m.insert("name".into(), Value::from(self.name.as_str()));
        m.insert("status".into(), Value::from(self.status.as_str()));
        m.insert("ageSeconds".into(), Value::from(self.age_seconds));
        if let Some(dn) = &self.display_name {
            m.insert("displayName".into(), Value::from(dn.as_str()));
        }
        Value::Object(m)
    }

    fn label(&self) -> String {
        match &self.display_name {
            Some(dn) => format!("{dn} ({})", self.name),
            None => self.name.clone(),
        }
    }
}

/// Oldest/newest anchored on `createdAt` only.
///
/// node: src/cli.ts:2252-2285
fn build_summary(sessions: &[SessionInfo]) -> Summary {
    let mut running = 0;
    let mut exited = 0;
    let mut vanished = 0;
    for s in sessions {
        match s.status {
            SessionStatus::Running => running += 1,
            SessionStatus::Exited => exited += 1,
            SessionStatus::Vanished => vanished += 1,
        }
    }
    let mut oldest: Option<(&SessionInfo, i64)> = None;
    let mut newest: Option<(&SessionInfo, i64)> = None;
    for s in sessions {
        let Some(ts) = created_at(s).and_then(parse_iso8601_ms) else {
            continue;
        };
        if oldest.is_none_or(|(_, t)| ts < t) {
            oldest = Some((s, ts));
        }
        if newest.is_none_or(|(_, t)| ts > t) {
            newest = Some((s, ts));
        }
    }
    let now = now_epoch_ms();
    let endpoint = |pick: Option<(&SessionInfo, i64)>| {
        pick.map(|(s, ts)| Endpoint {
            name: s.name.clone(),
            status: s.status,
            age_seconds: ((now - ts).div_euclid(1000)).max(0),
            display_name: display_name(s).map(str::to_string),
        })
    };
    Summary {
        total: sessions.len(),
        running,
        exited,
        vanished,
        oldest: endpoint(oldest),
        newest: endpoint(newest),
    }
}

/// One `list --json` element, keys in Node's order.
///
/// node: src/cli.ts:2292-2306
fn session_json(s: &SessionInfo) -> Value {
    let meta = s.metadata.as_ref();
    let mut m = Map::new();
    m.insert("name".into(), Value::from(s.name.as_str()));
    m.insert("status".into(), Value::from(s.status.as_str()));
    m.insert("pid".into(), s.pid.map(Value::from).unwrap_or(Value::Null));
    m.insert(
        "command".into(),
        meta.map(|m| Value::from(m.display_command.as_str()))
            .unwrap_or(Value::Null),
    );
    m.insert(
        "cwd".into(),
        meta.map(|m| Value::from(m.cwd.as_str())).unwrap_or(Value::Null),
    );
    m.insert(
        "createdAt".into(),
        meta.map(|m| Value::from(m.created_at.as_str()))
            .unwrap_or(Value::Null),
    );
    m.insert(
        "exitCode".into(),
        meta.and_then(|m| m.exit_code)
            .map(Value::from)
            .unwrap_or(Value::Null),
    );
    m.insert(
        "exitedAt".into(),
        meta.and_then(|m| m.exited_at.as_deref())
            .map(Value::from)
            .unwrap_or(Value::Null),
    );
    if let Some(tags) = meta.and_then(|m| m.tags.as_ref()) {
        m.insert("tags".into(), tag_map_json(tags));
    }
    if let Some(dn) = display_name(s) {
        m.insert("displayName".into(), Value::from(dn));
    }
    Value::Object(m)
}

fn tag_map_json(tags: &TagMap) -> Value {
    Value::Object(
        tags.iter()
            .map(|(k, v)| (k.clone(), Value::from(v.as_str())))
            .collect(),
    )
}

fn remote_host_json(h: &RemoteHost) -> Value {
    let mut m = Map::new();
    m.insert("label".into(), Value::from(h.label.as_str()));
    m.insert(
        "sessions".into(),
        Value::Array(
            h.sessions
                .iter()
                .map(|s| {
                    let mut o = Map::new();
                    o.insert("name".into(), Value::from(s.name.as_str()));
                    o.insert("status".into(), Value::from(s.status.as_str()));
                    if let Some(c) = &s.command {
                        o.insert("command".into(), Value::from(c.as_str()));
                    }
                    if let Some(c) = &s.cwd {
                        o.insert("cwd".into(), Value::from(c.as_str()));
                    }
                    if let Some(t) = &s.tags {
                        o.insert("tags".into(), tag_map_json(t));
                    }
                    if let Some(d) = &s.display_name {
                        o.insert("displayName".into(), Value::from(d.as_str()));
                    }
                    Value::Object(o)
                })
                .collect(),
        ),
    );
    m.insert(
        "error".into(),
        h.error
            .as_deref()
            .map(Value::from)
            .unwrap_or(Value::Null),
    );
    Value::Object(m)
}

/// Tags as hashtags; reserved keys hidden unless `show_all`.
///
/// node: src/cli.ts:2340-2344
fn render_tags(tags: Option<&TagMap>, show_all: bool) -> String {
    let Some(tags) = tags else {
        return String::new();
    };
    let entries: Vec<String> = tags
        .iter()
        .filter(|(k, _)| show_all || !is_reserved_tag_key(k))
        .map(|(k, v)| format!("#{k}={v}"))
        .collect();
    if entries.is_empty() {
        String::new()
    } else {
        format!(" {}", entries.join(" "))
    }
}

/// ` [flapping]` (red) beats ` [permanent]` (yellow).
///
/// node: src/cli.ts:4102-4112
fn strategy_marker(tags: Option<&TagMap>) -> &'static str {
    let Some(tags) = tags else {
        return "";
    };
    if tags.get("strategy.status").map(String::as_str) == Some("flapping") {
        return " \x1b[31m[flapping]\x1b[0m";
    }
    if tags.get("strategy").map(String::as_str) == Some("permanent") {
        return " \x1b[33m[permanent]\x1b[0m";
    }
    ""
}

/// `<bold>dn</bold> <dim>(name)</dim>` or `<bold>name</bold>`.
///
/// node: src/cli.ts:2349-2355
fn render_label(dn: Option<&str>, name: &str, bold: &str) -> String {
    match dn {
        Some(dn) => format!("{bold}{dn}\x1b[0m \x1b[2m({name})\x1b[0m"),
        None => format!("{bold}{name}\x1b[0m"),
    }
}

/// `cmdList`.
///
/// node: src/cli.ts:2165-2446
pub fn cmd_list(opts: &ListOptions) -> CliResult {
    let sessions = select(opts);
    let remote_hosts = if opts.remote_peer.is_some() || opts.remote {
        remote_list_hosts(opts.remote_peer.as_deref())
    } else {
        Vec::new()
    };

    if opts.json {
        if opts.summary {
            let s = build_summary(&sessions);
            let mut by_status = Map::new();
            by_status.insert("running".into(), Value::from(s.running));
            by_status.insert("exited".into(), Value::from(s.exited));
            by_status.insert("vanished".into(), Value::from(s.vanished));
            let mut m = Map::new();
            m.insert("total".into(), Value::from(s.total));
            m.insert("byStatus".into(), Value::Object(by_status));
            m.insert(
                "oldest".into(),
                s.oldest.as_ref().map(Endpoint::json).unwrap_or(Value::Null),
            );
            m.insert(
                "newest".into(),
                s.newest.as_ref().map(Endpoint::json).unwrap_or(Value::Null),
            );
            println!("{}", Value::Object(m));
            return Ok(0);
        }
        let local = Value::Array(sessions.iter().map(session_json).collect());
        if opts.remote && !remote_hosts.is_empty() {
            let mut m = Map::new();
            m.insert("local".into(), local);
            m.insert(
                "remote".into(),
                Value::Array(remote_hosts.iter().map(remote_host_json).collect()),
            );
            println!("{}", Value::Object(m));
        } else {
            println!("{local}");
        }
        return Ok(0);
    }

    if opts.summary {
        let s = build_summary(&sessions);
        if s.total == 0 {
            println!("No matching sessions.");
            return Ok(0);
        }
        let mut parts: Vec<String> = Vec::new();
        if s.running > 0 {
            parts.push(format!("{} running", s.running));
        }
        if s.exited > 0 {
            parts.push(format!("{} exited", s.exited));
        }
        if s.vanished > 0 {
            parts.push(format!("{} vanished", s.vanished));
        }
        println!(
            "{} session{} — {}",
            s.total,
            if s.total == 1 { "" } else { "s" },
            parts.join(", ")
        );
        if let Some(oldest) = &s.oldest {
            println!(
                "oldest: {} ({}, {})",
                oldest.label(),
                oldest.status,
                format_duration(oldest.age_seconds * 1000)
            );
        }
        if let Some(newest) = &s.newest
            && s.oldest.as_ref().is_none_or(|o| o.name != newest.name)
        {
            println!(
                "newest: {} ({}, {})",
                newest.label(),
                newest.status,
                format_duration(newest.age_seconds * 1000)
            );
        }
        return Ok(0);
    }

    if sessions.is_empty() && remote_hosts.is_empty() {
        println!("No active sessions.");
        return Ok(0);
    }

    let running: Vec<&SessionInfo> = sessions.iter().filter(|s| s.is_running()).collect();
    let exited: Vec<&SessionInfo> = sessions
        .iter()
        .filter(|s| s.status == SessionStatus::Exited)
        .collect();
    let vanished: Vec<&SessionInfo> = sessions
        .iter()
        .filter(|s| s.status == SessionStatus::Vanished)
        .collect();

    let cwd_of = |s: &SessionInfo| -> String {
        s.metadata
            .as_ref()
            .map(|m| m.cwd.as_str())
            .filter(|c| !c.is_empty())
            .map(short_path)
            .unwrap_or_default()
    };

    if !running.is_empty() {
        println!("Active sessions:");
        for s in &running {
            let cmd = s
                .metadata
                .as_ref()
                .map(|m| m.display_command.as_str())
                .unwrap_or("unknown");
            let pid = s
                .pid
                .map(|p| p.to_string())
                .unwrap_or_else(|| "null".to_string());
            println!(
                "  {}{}{} (pid: {pid}) — {} — \x1b[2m{cmd}\x1b[0m",
                render_label(display_name(s), &s.name, "\x1b[1;36m"),
                strategy_marker(tags_of(s)),
                render_tags(tags_of(s), opts.show_tags),
                cwd_of(s)
            );
        }
    }

    if !exited.is_empty() {
        if !running.is_empty() {
            println!();
        }
        println!("Exited sessions:");
        for s in &exited {
            let meta = s.metadata.as_ref();
            let code = meta
                .and_then(|m| m.exit_code)
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".to_string());
            let ago = meta
                .and_then(|m| m.exited_at.as_deref())
                .map(time_ago)
                .unwrap_or_else(|| "unknown".to_string());
            let cmd = meta.map(|m| m.display_command.as_str()).unwrap_or("");
            println!(
                "  {}{}{} (exited with code {code}, {ago}) — {} — \x1b[2m{cmd}\x1b[0m",
                render_label(display_name(s), &s.name, "\x1b[1m"),
                strategy_marker(tags_of(s)),
                render_tags(tags_of(s), opts.show_tags),
                cwd_of(s)
            );
        }
    }

    if !vanished.is_empty() {
        if !running.is_empty() || !exited.is_empty() {
            println!();
        }
        println!("\x1b[33mVanished sessions (no exit record — killed or crashed):\x1b[0m");
        for s in &vanished {
            let meta = s.metadata.as_ref();
            let ago = created_at(s)
                .map(time_ago)
                .unwrap_or_else(|| "unknown".to_string());
            let cmd = meta.map(|m| m.display_command.as_str()).unwrap_or("");
            println!(
                "  \u{26a0} {}{}{} (vanished, started {ago}) — {} — \x1b[2m{cmd}\x1b[0m",
                render_label(display_name(s), &s.name, "\x1b[1;33m"),
                strategy_marker(tags_of(s)),
                render_tags(tags_of(s), opts.show_tags),
                cwd_of(s)
            );
        }
    }

    for host in &remote_hosts {
        println!();
        if let Some(err) = &host.error {
            println!("\x1b[1m{}\x1b[0m \x1b[31m(error: {err})\x1b[0m", host.label);
            continue;
        }
        println!("\x1b[1m{}\x1b[0m ({} sessions):", host.label, host.sessions.len());
        let mut sorted: Vec<&RemoteSession> = host.sessions.iter().collect();
        sorted.sort_by(|a, b| {
            let ka = a.display_name.as_deref().unwrap_or(&a.name);
            let kb = b.display_name.as_deref().unwrap_or(&b.name);
            ka.cmp(kb)
        });
        for s in sorted {
            let icon = if s.status == "running" { "\u{25cf}" } else { "\u{25cb}" };
            let cwd = s.cwd.as_deref().map(short_path).unwrap_or_default();
            let cmd = s.command.as_deref().unwrap_or("");
            let dn = s.display_name.as_deref().filter(|d| !d.is_empty());
            println!(
                "  {icon} {}{}{} — {cwd} — \x1b[2m{cmd}\x1b[0m",
                render_label(dn, &s.name, "\x1b[1;36m"),
                strategy_marker(s.tags.as_ref()),
                render_tags(s.tags.as_ref(), opts.show_tags)
            );
        }
    }
    Ok(0)
}

fn tags_of(s: &SessionInfo) -> Option<&TagMap> {
    s.metadata.as_ref().and_then(|m| m.tags.as_ref())
}
