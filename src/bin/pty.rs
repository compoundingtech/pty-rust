//! The `pty` command-line tool — a Rust port of the pty project's CLI, backed
//! by libghostty. v0 surface: run / ls / peek / send / attach / kill / status.
//!
//! Persistent sessions are hosted by a per-session daemon (see
//! `pty_testkit::daemon`) that owns the PTY and a libghostty terminal and serves
//! the wire protocol over a unix socket.

use std::collections::BTreeMap;
use std::process::{exit, Stdio};
use std::time::Duration;

use pty_testkit::daemon::{self, DaemonConfig};
use pty_testkit::keys::parse_seq_value;
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
        "kill" => cmd_kill(rest),
        "rm" | "remove" => cmd_rm(rest),
        "status" | "stats" => cmd_status(rest),
        "version" | "--version" | "-v" | "-V" => {
            println!("pty-rust {}", env!("CARGO_PKG_VERSION"));
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
    let mut no_display_name = false;
    let mut cwd: Option<String> = None;
    let mut rows = 24u16;
    let mut cols = 80u16;
    let mut tags: BTreeMap<String, String> = BTreeMap::new();
    let mut env: Vec<(String, String)> = Vec::new();
    let mut unset_env: Vec<String> = Vec::new();
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
            "--no-display-name" => {
                no_display_name = true;
                i += 1;
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
            // Repeatable `k=v`. A later entry for the same key wins.
            "--tag" => {
                match args.get(i + 1).and_then(|kv| split_key_value(kv)) {
                    Some((k, v)) => {
                        tags.insert(k, v);
                    }
                    None => {
                        eprintln!("pty run: --tag needs KEY=VALUE");
                        return 2;
                    }
                }
                i += 2;
            }
            "--env" => {
                match args.get(i + 1).and_then(|kv| split_key_value(kv)) {
                    Some((k, v)) => env.push((k, v)),
                    None => {
                        eprintln!("pty run: --env needs KEY=VALUE");
                        return 2;
                    }
                }
                i += 2;
            }
            "--unset-env" => {
                match args.get(i + 1) {
                    Some(key) => unset_env.push(key.clone()),
                    None => {
                        eprintln!("pty run: --unset-env needs a variable name");
                        return 2;
                    }
                }
                i += 2;
            }
            // Accepted for CLI compatibility; v0 always backgrounds and always
            // allows nesting, so these carry no behaviour of their own.
            "-d" | "-a" | "-e" | "--force" => {
                i += 1;
            }
            "--" => {
                command = args[i + 1..].to_vec();
                break;
            }
            // An unknown flag is an error, never the start of the command. Swallowing
            // it here would run the flag as a program and report only that the daemon
            // never came up.
            other if other.starts_with('-') && other != "-" => {
                eprintln!("pty run: unknown option '{other}'");
                return 2;
            }
            _ => {
                // Bare command without `--`.
                command = args[i..].to_vec();
                break;
            }
        }
    }
    if command.is_empty() {
        eprintln!("pty run: no command given (use `pty run -- <cmd> [args...]`)");
        return 2;
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

    // Spawn the daemon detached (its own session, stdio to /dev/null).
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("pty run: cannot find own executable: {e}");
            return 1;
        }
    };
    let mut dcmd = std::process::Command::new(exe);
    dcmd.arg("__daemon")
        .arg("--name")
        .arg(&name)
        .arg("--rows")
        .arg(rows.to_string())
        .arg("--cols")
        .arg(cols.to_string())
        .arg("--cwd")
        .arg(&cwd);
    // `--no-display-name` wins: st2 asks for it explicitly when a session must carry none.
    if let Some(dn) = &display_name
        && !no_display_name
    {
        dcmd.arg("--display-name").arg(dn);
    }
    for (k, v) in &tags {
        dcmd.arg("--tag").arg(format!("{k}={v}"));
    }
    for (k, v) in &env {
        dcmd.arg("--env").arg(format!("{k}={v}"));
    }
    for k in &unset_env {
        dcmd.arg("--unset-env").arg(k);
    }
    dcmd.arg("--").args(&command);
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
    if let Err(e) = dcmd.spawn() {
        eprintln!("pty run: failed to start daemon: {e}");
        return 1;
    }

    // Wait for the daemon to come up (socket connectable).
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if client::is_alive(&name) {
            println!("{name}");
            return 0;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    eprintln!("pty run: daemon did not come up in time");
    1
}

/// Internal: `pty __daemon --name N --rows R --cols C --cwd D [--display-name X] -- <cmd...>`
fn cmd_daemon(args: &[String]) -> i32 {
    let mut name = String::new();
    let mut rows = 24u16;
    let mut cols = 80u16;
    let mut cwd = String::from(".");
    let mut display_name: Option<String> = None;
    let mut tags: BTreeMap<String, String> = BTreeMap::new();
    let mut env: Vec<(String, String)> = Vec::new();
    let mut unset_env: Vec<String> = Vec::new();
    let mut command: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                name = args.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "--tag" => {
                if let Some((k, v)) = args.get(i + 1).and_then(|kv| split_key_value(kv)) {
                    tags.insert(k, v);
                }
                i += 2;
            }
            "--env" => {
                if let Some((k, v)) = args.get(i + 1).and_then(|kv| split_key_value(kv)) {
                    env.push((k, v));
                }
                i += 2;
            }
            "--unset-env" => {
                if let Some(key) = args.get(i + 1) {
                    unset_env.push(key.clone());
                }
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
    // Tags and the display name are PERSISTED here rather than applied by a separate
    // `metadata patch` command, and that is deliberate. st2 plans a metadata patch only
    // when the presentation it observes in `list --json` differs from the one it asked
    // for at spawn. Saving both fields at launch and printing them back from `list`
    // means the two always agree, so st2 never plans a patch and no patch command is
    // needed. Do not add one because the CLI looks incomplete; add one only when
    // something must change presentation on a session that is already running.
    let cfg = DaemonConfig {
        name,
        command: program.clone(),
        args: cargs.to_vec(),
        display_command: command.join(" "),
        cwd,
        rows,
        cols,
        env,
        unset_env,
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
        let items: Vec<serde_json::Value> = sessions
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "status": session_status(s),
                    "alive": s.alive,
                    "command": s.meta.display_command,
                    "cwd": s.meta.cwd,
                    "exitCode": s.meta.exit_code,
                    "pid": registry::read_pid(&s.name),
                    "createdAt": s.meta.created_at,
                    "displayName": s.meta.display_name,
                    "tags": s.meta.tags.clone().unwrap_or_default(),
                })
            })
            .collect();
        println!("{}", serde_json::Value::Array(items));
        return 0;
    }
    if sessions.is_empty() {
        println!("No sessions.");
        return 0;
    }
    println!("{:<16} {:<10} {}", "NAME", "STATUS", "COMMAND");
    for s in sessions {
        // Same vocabulary as `list --json`, so a person and a supervisor never read two
        // different words for one state.
        let status = match (s.alive, s.meta.exit_code) {
            (false, Some(code)) => format!("exited:{code}"),
            _ => session_status(&s).to_string(),
        };
        println!("{:<16} {:<10} {}", s.name, status, s.meta.display_command);
    }
    0
}

/// `pty peek [--plain] [--full] [--wait TEXT [-t SECS]] <ref>`
fn cmd_peek(args: &[String]) -> i32 {
    let mut plain = false;
    let mut full = false;
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
            "--full" | "-f" => {
                full = true;
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
    // --seq mode: ordered sequence, each value literal or `key:<name>`.
    if rest.iter().any(|a| a == "--seq") {
        let mut chunks: Vec<Vec<u8>> = Vec::new();
        let mut delay: Option<Duration> = None;
        let mut i = 0;
        while i < rest.len() {
            match rest[i].as_str() {
                "--seq" => {
                    match rest.get(i + 1) {
                        Some(v) => match parse_seq_value(v) {
                            Ok(bytes) => chunks.push(bytes.into_bytes()),
                            Err(e) => {
                                eprintln!("pty send: {e}");
                                return 2;
                            }
                        },
                        None => {
                            eprintln!("pty send: --seq needs a value");
                            return 2;
                        }
                    }
                    i += 2;
                }
                // Seconds to wait BETWEEN sequences, as a decimal (`0.5`). A receiving
                // terminal application needs time to process a paste before the Return
                // that submits it arrives.
                "--with-delay" => {
                    match rest.get(i + 1).and_then(|v| v.parse::<f64>().ok()) {
                        Some(secs) if secs >= 0.0 && secs.is_finite() => {
                            delay = Some(Duration::from_secs_f64(secs));
                        }
                        _ => {
                            eprintln!("pty send: --with-delay needs a number of seconds");
                            return 2;
                        }
                    }
                    i += 2;
                }
                other if other.starts_with('-') && other != "-" => {
                    eprintln!("pty send: unknown option '{other}'");
                    return 2;
                }
                _ => i += 1,
            }
        }
        // With no delay asked for, send one buffer, exactly as before.
        if delay.is_none() {
            let out: Vec<u8> = chunks.concat();
            return match client::send(&name, &out) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("pty send: {e}");
                    1
                }
            };
        }
        let delay = delay.unwrap_or_default();
        for (index, chunk) in chunks.iter().enumerate() {
            if index > 0 {
                std::thread::sleep(delay);
            }
            if let Err(e) = client::send(&name, chunk) {
                eprintln!("pty send: {e}");
                return 1;
            }
        }
        return 0;
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
    eprintln!("[attached to {name} — press Ctrl-] to detach]");
    match client::attach(&name) {
        Ok(Some(code)) => code,
        Ok(None) => 0,
        Err(e) => {
            eprintln!("pty attach: {e}");
            1
        }
    }
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
    if let Some(pid) = registry::read_pid(&name) {
        // SIGTERM the session process; the daemon observes exit and cleans up.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        // Give it a moment; then SIGKILL if still alive.
        std::thread::sleep(Duration::from_millis(300));
        if registry::pid_alive(pid) {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
    // Deliberately do NOT remove the session's files here. The daemon records the exit
    // code, the exit time, and the last lines of the screen on its way out, and a
    // supervisor reads that evidence AFTER the kill to learn how the session ended.
    // Removing it here would answer "how did it die" with "it was never here".
    // `pty rm` is what finally discards a dead session.
    std::thread::sleep(Duration::from_millis(200));
    println!("killed {name}");
    0
}

/// `pty status <ref>`
fn cmd_status(args: &[String]) -> i32 {
    let reference = match args.first() {
        Some(r) => r.clone(),
        None => {
            eprintln!("pty status: usage: pty status <ref>");
            return 2;
        }
    };
    let name = match registry::resolve_ref(&reference) {
        Some(n) => n,
        None => {
            eprintln!("pty status: no such session '{reference}'");
            return 1;
        }
    };
    match client::status(&name) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(e) => {
            eprintln!("pty status: {e}");
            1
        }
    }
}

/// Split a repeatable `KEY=VALUE` argument on its FIRST `=`, so a value may contain one.
/// An empty key is rejected.
fn split_key_value(arg: &str) -> Option<(String, String)> {
    let (key, value) = arg.split_once('=')?;
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), value.to_string()))
}

/// The lifecycle word a supervisor reads: `running` while the process lives,
/// `exited` once an exit code is recorded, `vanished` when the process is gone
/// but left no code behind.
fn session_status(session: &registry::SessionInfo) -> &'static str {
    if session.alive {
        "running"
    } else if session.meta.exit_code.is_some() {
        "exited"
    } else {
        "vanished"
    }
}

/// `pty rm <ref>` — remove an exited session's files so its id can be reused.
///
/// Refuses to remove a session that is still running, because that would strip the
/// socket and pid out from under a live daemon.
fn cmd_rm(args: &[String]) -> i32 {
    let reference = match args.iter().find(|a| !a.starts_with('-')) {
        Some(r) => r.clone(),
        None => {
            eprintln!("pty rm: usage: pty rm <ref>");
            return 2;
        }
    };
    let name = match registry::resolve_ref(&reference) {
        Some(n) => n,
        None => {
            // "not found" is the wording a caller matches on to treat this as success.
            eprintln!("pty rm: session '{reference}' not found");
            return 1;
        }
    };
    if client::is_alive(&name) {
        eprintln!("pty rm: session '{name}' is still running; kill it first");
        return 1;
    }
    registry::cleanup(&name);
    println!("removed {name}");
    0
}

fn print_help() {
    println!(
        "pty — persistent terminal sessions (Rust + libghostty)\n\
         \n\
         USAGE:\n\
         \x20 pty run [--id X] [--name X|--no-display-name] [--cwd D] [--rows R] [--cols C]\n\
         \x20     [--tag K=V ...] [--env K=V ...] [--unset-env K ...] -- <cmd...>\n\
         \x20 pty ls [--json]\n\
         \x20 pty peek [--plain] [--full] [--wait TEXT [-t SECS]] <ref>\n\
         \x20 pty send <ref> <text> | [--with-delay SECS] --seq <value> ...\n\
         \x20 pty attach <ref>            (Ctrl-] to detach)\n\
         \x20 pty kill <ref>\n\
         \x20 pty rm <ref>\n\
         \x20 pty status <ref>\n\
         \x20 pty version"
    );
}
