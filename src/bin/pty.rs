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
    let mut cwd: Option<String> = None;
    let mut rows = 24u16;
    let mut cols = 80u16;
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
            "-d" | "-a" | "-e" | "--no-display-name" => {
                // Accepted for CLI compatibility; v0 always backgrounds.
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
    if let Some(dn) = &display_name {
        dcmd.arg("--display-name").arg(dn);
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
    let mut command: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
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
    let _ = &display_name; // display-name is applied by `rename` in a later phase
    let cfg = DaemonConfig {
        name,
        command: program.clone(),
        args: cargs.to_vec(),
        display_command: command.join(" "),
        cwd,
        rows,
        cols,
        env: Vec::new(),
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
        let items: Vec<String> = sessions
            .iter()
            .map(|s| {
                format!(
                    "{{\"name\":{:?},\"alive\":{},\"command\":{:?},\"cwd\":{:?},\"exitCode\":{}}}",
                    s.name,
                    s.alive,
                    s.meta.display_command,
                    s.meta.cwd,
                    s.meta
                        .exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "null".into())
                )
            })
            .collect();
        println!("[{}]", items.join(","));
        return 0;
    }
    if sessions.is_empty() {
        println!("No sessions.");
        return 0;
    }
    println!("{:<16} {:<10} {}", "NAME", "STATUS", "COMMAND");
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
        let mut out = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            if rest[i] == "--seq" {
                if let Some(v) = rest.get(i + 1) {
                    match parse_seq_value(v) {
                        Ok(bytes) => out.extend_from_slice(bytes.as_bytes()),
                        Err(e) => {
                            eprintln!("pty send: {e}");
                            return 2;
                        }
                    }
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        return match client::send(&name, &out) {
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
    // Best-effort registry cleanup if the daemon didn't get to it.
    std::thread::sleep(Duration::from_millis(200));
    if !client::is_alive(&name) {
        registry::cleanup(&name);
    }
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

fn print_help() {
    println!(
        "pty — persistent terminal sessions (Rust + libghostty)\n\
         \n\
         USAGE:\n\
         \x20 pty run [--id X] [--name X] [--cwd D] [--rows R] [--cols C] -- <cmd...>\n\
         \x20 pty ls [--json]\n\
         \x20 pty peek [--plain] [--full] [--wait TEXT [-t SECS]] <ref>\n\
         \x20 pty send <ref> <text> | --seq <value> ...\n\
         \x20 pty attach <ref>            (Ctrl-] to detach)\n\
         \x20 pty kill <ref>\n\
         \x20 pty status <ref>\n\
         \x20 pty version"
    );
}
