//! The `pty` command-line tool — a Rust port of the pty project's CLI, backed
//! by libghostty. v0 surface: run / ls / peek / send / attach / kill / status.
//!
//! Persistent sessions are hosted by a per-session daemon (see
//! `pty_testkit::daemon`) that owns the PTY and a libghostty terminal and serves
//! the wire protocol over a unix socket.

use std::process::{exit, Stdio};
use std::time::Duration;

use pty_testkit::daemon::{self, DaemonConfig};
use pty_testkit::keys::parse_seq_value;
use pty_testkit::ptyfile::{self, command_with_env_exports, PtySessionDef};
use pty_testkit::registry;
use pty_testkit::client;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_help();
        exit(0);
    }
    let cmd = args[0].as_str();
    let rest = &args[1..];
    let code = match cmd {
        "__daemon" => cmd_daemon(rest),
        "run" | "spawn" => cmd_run(rest),
        "ls" | "list" => cmd_ls(rest),
        "peek" => cmd_peek(rest),
        "send" => cmd_send(rest),
        "attach" | "a" => cmd_attach(rest),
        "up" => cmd_up(rest),
        "down" => cmd_down(rest),
        "restart" => cmd_restart(rest),
        "rm" | "remove" => cmd_rm(rest),
        "rename" => cmd_rename(rest),
        "kill" => cmd_kill(rest),
        "status" | "stats" => cmd_status(rest),
        "version" | "--version" | "-v" | "-V" => {
            // Bare semver, matching node's `pty --version` format (node also
            // appends +<sha>; the number differs by project).
            println!("{}", env!("CARGO_PKG_VERSION"));
            0
        }
        "help" | "--help" | "-h" => {
            print_help();
            0
        }
        other => {
            eprintln!("pty: unknown command '{other}'. Try `pty help`.");
            1
        }
    };
    exit(code);
}

/// `pty run [--id X] [--name X] [--cwd D] [--tag k=v] [--rows R] [--cols C] -- <cmd...>`
fn cmd_run(args: &[String]) -> i32 {
    let mut id: Option<String> = None;
    let mut display_name: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut rows = 24u16;
    let mut cols = 80u16;
    let mut background = false;
    let mut force = false;
    let mut ephemeral = false;
    let mut tags: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    let mut command: Vec<String> = Vec::new();
    while i < args.len() {
        match args[i].as_str() {
            "--id" => {
                id = args.get(i + 1).cloned();
                i += 2;
            }
            "--name" => {
                display_name = args.get(i + 1).cloned();
                i += 2;
            }
            "--cwd" => {
                cwd = args.get(i + 1).cloned();
                i += 2;
            }
            "--rows" => {
                rows = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(24);
                i += 2;
            }
            "--cols" => {
                cols = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(80);
                i += 2;
            }
            "-d" | "--detach" => {
                background = true;
                i += 1;
            }
            "--force" => {
                // Canonical (CoS/Nathan ruling, node code fixed to match its
                // --help docs): --force creates even from inside a pty session
                // (bypasses the nesting guard), symmetric with attach's --force.
                force = true;
                i += 1;
            }
            "-e" | "--ephemeral" => {
                // Force reap-on-exit (highest per-session override except keep).
                ephemeral = true;
                i += 1;
            }
            "--tag" => {
                if let Some((k, v)) = args.get(i + 1).and_then(|kv| kv.split_once('=')) {
                    tags.push((k.to_string(), v.to_string()));
                }
                i += 2;
            }
            "-a" | "--attach" | "--isolate-env" | "--no-display-name" => {
                // Accepted for CLI compatibility (attach/no-label).
                i += 1;
            }
            "--" => {
                command = args[i + 1..].to_vec();
                break;
            }
            other => {
                // Bare command without `--`.
                command = args[i..].to_vec();
                let _ = other;
                break;
            }
        }
    }
    if command.is_empty() {
        eprintln!("pty run: no command given (use `pty run -- <cmd> [args...]`)");
        return 2;
    }

    // Nesting prevention: running `pty run` from inside a pty session would
    // create a session-inside-a-session. Detect it via PTY_SESSION and run the
    // command directly, unless `-d` (background) or `--force` explicitly asks
    // for a real nested session — the canonical behavior (node code fixed to
    // match its docs; --force creates).
    if !background
        && !force
        && std::env::var("PTY_SESSION").map(|v| !v.is_empty()).unwrap_or(false)
    {
        let (program, pargs) = command.split_first().unwrap();
        let mut c = std::process::Command::new(program);
        c.args(pargs);
        if let Some(dir) = &cwd {
            c.current_dir(dir);
        }
        use std::os::unix::process::CommandExt;
        // exec() only returns on failure.
        let err = c.exec();
        eprintln!("pty run: exec '{program}' failed: {err}");
        return 127;
    }

    let name = id.unwrap_or_else(registry::generate_id);
    if let Err(e) = registry::validate_name(&name) {
        eprintln!("pty run: {e}");
        return 2;
    }
    if registry::session_exists(&name) && client::is_alive(&name) {
        eprintln!("pty run: session '{name}' already exists");
        return 1;
    }

    let cwd = cwd.unwrap_or_else(|| daemon::default_cwd().to_string_lossy().into_owned());

    match spawn_session_daemon(
        &name,
        &command,
        &cwd,
        rows,
        cols,
        display_name.as_deref(),
        ephemeral,
        &tags,
    ) {
        Ok(()) => {
            println!("{name}");
            0
        }
        Err(msg) => {
            eprintln!("pty run: {msg}");
            1
        }
    }
}

/// Spawn a detached session daemon and wait for it to come up.
#[allow(clippy::too_many_arguments)]
fn spawn_session_daemon(
    name: &str,
    command: &[String],
    cwd: &str,
    rows: u16,
    cols: u16,
    display_name: Option<&str>,
    ephemeral: bool,
    tags: &[(String, String)],
) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find own executable: {e}"))?;
    let mut dcmd = std::process::Command::new(exe);
    dcmd.arg("__daemon")
        .arg("--name")
        .arg(name)
        .arg("--rows")
        .arg(rows.to_string())
        .arg("--cols")
        .arg(cols.to_string())
        .arg("--cwd")
        .arg(cwd);
    if let Some(dn) = display_name {
        dcmd.arg("--display-name").arg(dn);
    }
    if ephemeral {
        dcmd.arg("--ephemeral");
    }
    for (k, v) in tags {
        dcmd.arg("--tag").arg(format!("{k}={v}"));
    }
    dcmd.arg("--").args(command);
    dcmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Detach into its own session so it survives the parent shell.
    unsafe {
        use std::os::unix::process::CommandExt;
        dcmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut daemon = dcmd
        .spawn()
        .map_err(|e| format!("failed to start daemon: {e}"))?;

    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        // Ready if the socket is connectable...
        if client::is_alive(name) {
            return Ok(());
        }
        // ...or the session already ran and exited in PRESERVE mode (metadata
        // records the exit)...
        if let Some(meta) = registry::read_metadata(name)
            && meta.exit_code.is_some()
        {
            return Ok(());
        }
        // ...or the daemon process itself has already exited — a fast-exiting
        // command may have run and been REAPED (leaving no trace) before we
        // observed the socket. The daemon exiting means the session ran.
        if let Ok(Some(_)) = daemon.try_wait() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Err("daemon did not come up in time".into())
}

/// Internal: `pty __daemon --name N --rows R --cols C --cwd D [--display-name X] -- <cmd...>`
fn cmd_daemon(args: &[String]) -> i32 {
    let mut name = String::new();
    let mut rows = 24u16;
    let mut cols = 80u16;
    let mut cwd = String::from(".");
    let mut display_name: Option<String> = None;
    let mut ephemeral = false;
    let mut tags: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut command: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ephemeral" => {
                ephemeral = true;
                i += 1;
            }
            "--tag" => {
                if let Some((k, v)) = args.get(i + 1).and_then(|kv| kv.split_once('=')) {
                    tags.insert(k.to_string(), v.to_string());
                }
                i += 2;
            }
            "--name" => {
                name = args.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "--rows" => {
                rows = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(24);
                i += 2;
            }
            "--cols" => {
                cols = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(80);
                i += 2;
            }
            "--cwd" => {
                cwd = args.get(i + 1).cloned().unwrap_or_else(|| ".".into());
                i += 2;
            }
            "--display-name" => {
                display_name = args.get(i + 1).cloned();
                i += 2;
            }
            "--" => {
                command = args[i + 1..].to_vec();
                break;
            }
            _ => i += 1,
        }
    }
    if name.is_empty() || command.is_empty() {
        eprintln!("pty __daemon: missing --name or command");
        return 2;
    }
    let (program, cargs) = command.split_first().unwrap();
    let cfg = DaemonConfig {
        name,
        command: program.clone(),
        args: cargs.to_vec(),
        display_command: command.join(" "),
        cwd,
        rows,
        cols,
        env: Vec::new(),
        ephemeral,
        tags,
        display_name,
    };
    match daemon::run(cfg) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("pty daemon error: {e}");
            1
        }
    }
}

/// `pty ls [--json]`
fn cmd_ls(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    let sessions = registry::list_sessions();
    if json {
        // Match node's ls --json shape exactly (node = reference):
        //   {name, status, pid(daemon), command, cwd, createdAt, exitCode,
        //    exitedAt, displayName?}  (displayName omitted when unset).
        // status enum: "running" | "exited" | "vanished".
        use serde_json::{Map, Value};
        let items: Vec<String> = sessions
            .iter()
            .map(|s| {
                let status = if s.alive {
                    "running"
                } else if s.meta.exit_code.is_some() {
                    "exited"
                } else {
                    "vanished"
                };
                let pid = registry::read_pid(&s.name); // daemon pid
                let mut m = Map::new();
                m.insert("name".into(), Value::from(s.name.clone()));
                m.insert("status".into(), Value::from(status));
                m.insert("pid".into(), pid.map(Value::from).unwrap_or(Value::Null));
                m.insert("command".into(), Value::from(s.meta.display_command.clone()));
                m.insert("cwd".into(), Value::from(s.meta.cwd.clone()));
                m.insert("createdAt".into(), Value::from(s.meta.created_at.clone()));
                m.insert(
                    "exitCode".into(),
                    s.meta.exit_code.map(Value::from).unwrap_or(Value::Null),
                );
                m.insert(
                    "exitedAt".into(),
                    s.meta
                        .exited_at
                        .clone()
                        .map(Value::from)
                        .unwrap_or(Value::Null),
                );
                if let Some(dn) = &s.meta.display_name {
                    m.insert("displayName".into(), Value::from(dn.clone()));
                }
                serde_json::to_string(&Value::Object(m)).unwrap()
            })
            .collect();
        println!("[{}]", items.join(","));
        return 0;
    }
    if sessions.is_empty() {
        println!("No sessions.");
        return 0;
    }
    println!("{:<16} {:<10} COMMAND", "NAME", "STATUS");
    for s in sessions {
        let status = if s.alive {
            "running".to_string()
        } else if let Some(code) = s.meta.exit_code {
            format!("exited:{code}")
        } else {
            "dead".to_string()
        };
        println!("{:<16} {:<10} {}", s.name, status, s.meta.display_command);
    }
    0
}

/// `pty peek [--plain] [--full] [--wait TEXT [-t SECS]] <ref>`
fn cmd_peek(args: &[String]) -> i32 {
    let mut plain = false;
    let mut full = false;
    let mut follow = false;
    let mut wait: Option<String> = None;
    let mut timeout = 5u64;
    let mut reference: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--plain" | "-p" => {
                plain = true;
                i += 1;
            }
            "--full" => {
                full = true;
                i += 1;
            }
            "--follow" | "-f" => {
                follow = true;
                i += 1;
            }
            "--wait" => {
                wait = args.get(i + 1).cloned();
                i += 2;
            }
            "-t" | "--timeout" => {
                timeout = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(5);
                i += 2;
            }
            other => {
                reference = Some(other.to_string());
                i += 1;
            }
        }
    }
    let name = match reference.and_then(|r| registry::resolve_ref(&r)) {
        Some(n) => n,
        None => {
            eprintln!("pty peek: no such session");
            return 1;
        }
    };
    if follow {
        return match client::follow(&name) {
            Ok(Some(code)) => {
                if code < 0 {
                    0
                } else {
                    code
                }
            }
            Ok(None) => 0,
            Err(e) => {
                eprintln!("pty peek: {e}");
                1
            }
        };
    }
    if let Some(needle) = wait {
        match client::peek_wait(&name, &needle, Duration::from_secs(timeout)) {
            Ok(Some(screen)) => {
                print!("{screen}");
                0
            }
            Ok(None) => {
                eprintln!("pty peek: timed out waiting for {needle:?}");
                1
            }
            Err(e) => {
                eprintln!("pty peek: {e}");
                1
            }
        }
    } else {
        match client::peek(&name, plain, full) {
            Ok(screen) => {
                print!("{screen}");
                if plain && !screen.ends_with('\n') {
                    println!();
                }
                0
            }
            Err(e) => {
                eprintln!("pty peek: {e}");
                1
            }
        }
    }
}

/// `pty send <ref> <text> | --seq VALUE [--seq VALUE ...]`
fn cmd_send(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("pty send: usage: pty send <ref> <text> | --seq <value> ...");
        return 2;
    }
    let name = match registry::resolve_ref(&args[0]) {
        Some(n) => n,
        None => {
            eprintln!("pty send: no such session '{}'", args[0]);
            return 1;
        }
    };
    let rest = &args[1..];
    // --paste mode: wrap the payload in bracketed-paste markers so a receiving
    // TUI treats it as one paste event. Position-independent.
    if rest.iter().any(|a| a == "--paste") {
        let mut payload = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            if rest[i] == "--paste" {
                if let Some(v) = rest.get(i + 1) {
                    payload.extend_from_slice(v.as_bytes());
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        let wrapped = pty_testkit::paste::wrap_bracketed_paste(&payload);
        return match client::send(&name, &wrapped) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("pty send: {e}");
                1
            }
        };
    }
    // --seq mode: ordered sequence, each value literal or `key:<name>`. Each
    // item is delivered as a separate write with a paced gap between items
    // (node's default 300ms; override with --with-delay <sec>; 0 = stream).
    if rest.iter().any(|a| a == "--seq") {
        // Pull out --with-delay first so its position doesn't matter.
        let mut delay_secs: Option<f64> = None;
        for w in rest.windows(2) {
            if w[0] == "--with-delay" {
                match w[1].parse::<f64>() {
                    Ok(v) if v >= 0.0 => delay_secs = Some(v),
                    _ => {
                        eprintln!("pty send: --with-delay requires a non-negative number (seconds).");
                        return 2;
                    }
                }
            }
        }
        // Collect the ordered --seq items (skipping the --with-delay pair).
        let mut items: Vec<Vec<u8>> = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            match rest[i].as_str() {
                "--seq" => {
                    if let Some(v) = rest.get(i + 1) {
                        match parse_seq_value(v) {
                            Ok(bytes) => items.push(bytes.into_bytes()),
                            Err(e) => {
                                eprintln!("pty send: {e}");
                                return 2;
                            }
                        }
                    }
                    i += 2;
                }
                "--with-delay" => i += 2,
                _ => i += 1,
            }
        }
        let delay = client::resolve_seq_delay_ms(delay_secs);
        return match client::send_seq(&name, &items, delay) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("pty send: {e}");
                1
            }
        };
    }
    // Literal text (no implicit newline), joining any extra args with spaces.
    let text = rest.join(" ");
    match client::send(&name, text.as_bytes()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("pty send: {e}");
            1
        }
    }
}

/// `pty attach <ref>`
fn cmd_attach(args: &[String]) -> i32 {
    let reference = match args.iter().find(|a| !a.starts_with('-')) {
        Some(r) => r.clone(),
        None => {
            eprintln!("pty attach: usage: pty attach <ref>");
            return 2;
        }
    };
    let name = match registry::resolve_ref(&reference) {
        Some(n) => n,
        None => {
            eprintln!("pty attach: no such session '{reference}'");
            return 1;
        }
    };
    eprintln!("[attached to {name} — press Ctrl+\\ to detach]");
    match client::attach(&name) {
        Ok(Some(code)) => code,
        Ok(None) => 0,
        Err(e) => {
            eprintln!("pty attach: {e}");
            1
        }
    }
}

/// The on-disk session name for a manifest entry: its pinned `id`, else the
/// short name from the toml key.
fn manifest_session_name(sess: &PtySessionDef) -> String {
    sess.id.clone().unwrap_or_else(|| sess.short_name.clone())
}

/// Split `up`/`down` args into an optional manifest dir and session filters.
fn split_dir_and_filters(args: &[String]) -> (Option<String>, Vec<String>) {
    if let Some(first) = args.first()
        && std::path::Path::new(first).is_dir() {
            return (Some(first.clone()), args[1..].to_vec());
        }
    (None, args.to_vec())
}

fn matches_filter(sess: &PtySessionDef, filters: &[String]) -> bool {
    filters.is_empty()
        || filters
            .iter()
            .any(|f| *f == sess.short_name || *f == sess.display_name)
}

/// `pty up [dir] [names...]` — start sessions declared in a `pty.toml`.
fn cmd_up(args: &[String]) -> i32 {
    let (dir, filters) = split_dir_and_filters(args);
    let file = match ptyfile::read_pty_file(dir.as_deref().map(std::path::Path::new)) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("pty up: {e}");
            return 1;
        }
    };
    let mut started = 0;
    let mut failed = 0;
    for sess in &file.sessions {
        if !matches_filter(sess, &filters) {
            continue;
        }
        let name = manifest_session_name(sess);
        if let Err(e) = registry::validate_name(&name) {
            eprintln!("pty up: {name}: {e}");
            failed += 1;
            continue;
        }
        if client::is_alive(&name) {
            println!("{name} already running");
            continue;
        }
        let script = command_with_env_exports(sess);
        let command = vec!["sh".to_string(), "-c".to_string(), script];
        let cwd = sess
            .cwd
            .clone()
            .unwrap_or_else(|| file.dir.to_string_lossy().into_owned());
        let sess_tags: Vec<(String, String)> = sess
            .tags
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect();
        match spawn_session_daemon(
            &name,
            &command,
            &cwd,
            24,
            80,
            Some(&sess.display_name),
            false,
            &sess_tags,
        ) {
            Ok(()) => {
                println!("started {name} ({})", sess.display_name);
                started += 1;
            }
            Err(e) => {
                eprintln!("pty up: {name}: {e}");
                failed += 1;
            }
        }
    }
    if started == 0 && failed == 0 {
        println!("no matching sessions to start");
    }
    if failed > 0 {
        1
    } else {
        0
    }
}

/// `pty down [dir] [names...]` — stop sessions declared in a `pty.toml`.
fn cmd_down(args: &[String]) -> i32 {
    let (dir, filters) = split_dir_and_filters(args);
    let file = match ptyfile::read_pty_file(dir.as_deref().map(std::path::Path::new)) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("pty down: {e}");
            return 1;
        }
    };
    let mut stopped = 0;
    for sess in &file.sessions {
        if !matches_filter(sess, &filters) {
            continue;
        }
        let name = manifest_session_name(sess);
        if !client::is_alive(&name) && !registry::session_exists(&name) {
            continue;
        }
        kill_session(&name);
        println!("stopped {name}");
        stopped += 1;
    }
    if stopped == 0 {
        println!("no matching sessions to stop");
    }
    0
}

/// SIGTERM (then SIGKILL) a session's process and clean up if the daemon didn't.
/// SIGTERM a session's daemon (an EXTERNAL stop). The daemon forwards SIGHUP to
/// the child, escalates to SIGKILL via its watchdog if needed, then PRESERVES
/// the session (status=exited) unless it's ephemeral — matching node #114. We
/// wait for the daemon to finish its clean shutdown (so the exit metadata is
/// written) rather than SIGKILL it early. It does NOT remove the session from
/// the registry; that's `rm`.
fn kill_session(name: &str) {
    if let Some(pid) = registry::read_pid(name) {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        // Wait for the daemon to run its shutdown (child dies → preserve/reap →
        // exit). ~3s budget covers the watchdog's 500ms SIGKILL escalation.
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if !registry::pid_alive(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // Last resort if the daemon itself is wedged.
        if registry::pid_alive(pid) {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

/// `pty restart <ref>` — stop (if running) and respawn with the same command.
fn cmd_restart(args: &[String]) -> i32 {
    let reference = match args.first() {
        Some(r) => r.clone(),
        None => {
            eprintln!("pty restart: usage: pty restart <ref>");
            return 2;
        }
    };
    let name = match registry::resolve_ref(&reference) {
        Some(n) => n,
        None => {
            eprintln!("pty restart: no such session '{reference}'");
            return 1;
        }
    };
    let meta = match registry::read_metadata(&name) {
        Some(m) => m,
        None => {
            eprintln!("pty restart: no metadata for '{name}'");
            return 1;
        }
    };
    // Stop the current instance if it's alive, then clear its socket/pid.
    if client::is_alive(&name) {
        kill_session(&name);
    }
    registry::cleanup(&name);

    let mut command = vec![meta.command.clone()];
    command.extend(meta.args.iter().cloned());
    let meta_tags: Vec<(String, String)> = meta
        .tags
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect();
    match spawn_session_daemon(
        &name,
        &command,
        &meta.cwd,
        24,
        80,
        meta.display_name.as_deref(),
        false,
        &meta_tags,
    ) {
        Ok(()) => {
            println!("restarted {name}");
            0
        }
        Err(e) => {
            eprintln!("pty restart: {e}");
            1
        }
    }
}

/// `pty rm <ref>` — kill if running, then remove from the registry.
fn cmd_rm(args: &[String]) -> i32 {
    let reference = match args.first() {
        Some(r) => r.clone(),
        None => {
            eprintln!("pty rm: usage: pty rm <ref>");
            return 2;
        }
    };
    let name = match registry::resolve_ref(&reference) {
        Some(n) => n,
        None => {
            eprintln!("pty rm: no such session '{reference}'");
            return 1;
        }
    };
    if client::is_alive(&name) {
        kill_session(&name);
    }
    registry::cleanup(&name);
    println!("removed {name}");
    0
}

/// `pty rename <ref> <new-display-name>` — set the session's display label.
fn cmd_rename(args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("pty rename: usage: pty rename <ref> <new-display-name>");
        return 2;
    }
    let name = match registry::resolve_ref(&args[0]) {
        Some(n) => n,
        None => {
            eprintln!("pty rename: no such session '{}'", args[0]);
            return 1;
        }
    };
    let new_name = args[1..].join(" ");
    let mut meta = match registry::read_metadata(&name) {
        Some(m) => m,
        None => {
            eprintln!("pty rename: no metadata for '{name}'");
            return 1;
        }
    };
    meta.display_name = Some(new_name.clone());
    if let Err(e) = registry::write_metadata(&name, &meta) {
        eprintln!("pty rename: {e}");
        return 1;
    }
    println!("renamed {name} -> {new_name}");
    0
}

/// `pty kill <ref>`
fn cmd_kill(args: &[String]) -> i32 {
    let reference = match args.first() {
        Some(r) => r.clone(),
        None => {
            eprintln!("pty kill: usage: pty kill <ref>");
            return 2;
        }
    };
    let name = match registry::resolve_ref(&reference) {
        Some(n) => n,
        None => {
            eprintln!("pty kill: no such session '{reference}'");
            return 1;
        }
    };
    kill_session(&name);
    println!("killed {name}");
    0
}

/// The small gone-session stats shape, from metadata.
fn gone_stats(name: &str, meta: &registry::SessionMetadata) -> pty_testkit::stats::GoneStats {
    let status = if meta.exit_code.is_some() {
        "exited"
    } else {
        "vanished"
    };
    pty_testkit::stats::GoneStats {
        name: name.to_string(),
        status: status.to_string(),
        exit_code: meta.exit_code,
        exited_at: meta.exited_at.clone(),
        tags: meta.tags.clone(),
    }
}

/// `pty stats [--json] [--all] [<ref>]` — a running session emits the full
/// StatsResult (queried from the daemon); a gone session emits the small shape;
/// with no ref, an array of all (gone entries only with --all).
fn cmd_status(args: &[String]) -> i32 {
    let all = args.iter().any(|a| a == "--all");
    let reference = args.iter().find(|a| !a.starts_with('-')).cloned();

    if let Some(reference) = reference {
        let name = match registry::resolve_ref(&reference) {
            Some(n) => n,
            None => {
                eprintln!("pty stats: no such session '{reference}'");
                return 1;
            }
        };
        // Running: query the live StatsResult from the daemon.
        if client::is_alive(&name)
            && let Ok(json) = client::status(&name)
            && !json.is_empty()
        {
            println!("{json}");
            return 0;
        }
        // Gone: emit the small shape from metadata.
        if let Some(meta) = registry::read_metadata(&name) {
            println!("{}", serde_json::to_string(&gone_stats(&name, &meta)).unwrap());
            return 0;
        }
        eprintln!("pty stats: no such session '{reference}'");
        return 1;
    }

    // No ref: an array of all sessions.
    let mut items: Vec<String> = Vec::new();
    for s in registry::list_sessions() {
        if s.alive {
            match client::status(&s.name) {
                Ok(json) if !json.is_empty() => items.push(json),
                _ => items.push(format!(
                    "{{\"name\":{},\"error\":\"query failed\"}}",
                    serde_json::to_string(&s.name).unwrap()
                )),
            }
        } else if all {
            items.push(serde_json::to_string(&gone_stats(&s.name, &s.meta)).unwrap());
        }
    }
    println!("[{}]", items.join(","));
    0
}

fn print_help() {
    println!(
        "pty — persistent terminal sessions (Rust + libghostty)\n\
         \n\
         USAGE:\n\
         \x20 pty run [--id X] [--name X] [--cwd D] [--rows R] [--cols C] -- <cmd...>\n\
         \x20 pty ls [--json]\n\
         \x20 pty peek [--plain] [--full] [-f] [--wait TEXT [-t SECS]] <ref>\n\
         \x20 pty send <ref> <text> | [--with-delay <sec>] --seq <value> ...\n\
         \x20                             (--seq gap defaults to 0.3s; --with-delay 0 = stream)\n\
         \x20 pty attach <ref>            (Ctrl+\\ to detach; twice sends it to the child)\n\
         \x20 pty up [dir] [names...]     (start sessions from pty.toml)\n\
         \x20 pty down [dir] [names...]   (stop them)\n\
         \x20 pty restart <ref>\n\
         \x20 pty rename <ref> <label>\n\
         \x20 pty rm <ref>\n\
         \x20 pty kill <ref>\n\
         \x20 pty stats [--json] [--all] [<ref>]\n\
         \x20 pty version"
    );
}
