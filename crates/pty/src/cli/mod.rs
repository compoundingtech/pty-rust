//! The `pty` CLI: argv dispatch and one module per verb.
//!
//! [`dispatch`] is Node's `main()` (cli.ts:670-760): the global `--root`
//! scan, the root-length backstop, subcommand detection, the interactive
//! flags, the per-command `--help` interceptor, the command switch, and
//! git-style forwarding to `pty-<cmd>` executables. Every verb returns a
//! [`CliResult`]; an `Err` is printed to stderr and exits 1, the way Node's
//! `main().catch` prints `err.message` (cli.ts:4132-4135).
//!
//! Verbs that talk to a session socket (`run`, `attach`, `exec`, `peek`,
//! `send`, `stats`, `restart`, `kill`) are interim ports kept from the v0
//! binary so the whole surface builds and `run -d` works for tests; they
//! are being rewritten against Node's texts separately.

pub mod argv;
pub mod ask;
pub mod completions;
pub mod deferred;
pub mod down;
pub mod emit;
pub mod events;
pub mod gc;
pub mod help;
pub mod list;
pub mod metadata;
pub mod rename;
pub mod rm;
pub mod tag;
pub mod tag_multi;
pub mod up;
pub mod version;

// Socket verbs (interim ports; see the module comment above).
pub mod attach;
pub mod exec;
pub mod kill;
pub mod peek;
pub mod restart;
pub mod run;
pub mod send;
pub mod stats;

use std::fmt;
use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use pty_core::client;
use pty_core::registry::{self, EnvMap, TagMap};

/// A failed command: `Display` is the exact text Node prints to stderr
/// before exiting 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(pub String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

impl From<String> for CliError {
    fn from(s: String) -> Self {
        CliError(s)
    }
}

impl From<&str> for CliError {
    fn from(s: &str) -> Self {
        CliError(s.to_string())
    }
}

/// What a verb returns: the exit code, or a message that exits 1.
pub type CliResult = Result<i32, CliError>;

/// Resolve a user-supplied reference (stable id or unique display name) to
/// the stable id. Not found → `Session "<ref>" not found.`; several display
/// name matches → the registry's ambiguity text. Both exit 1.
///
/// node: src/cli.ts:610-617 (`resolveRef`), src/sessions.ts:1351-1363
pub fn resolve_ref(reference: &str) -> Result<String, CliError> {
    match registry::get_session(reference) {
        Ok(Some(s)) => Ok(s.name),
        Ok(None) => Err(CliError(format!("Session \"{reference}\" not found."))),
        Err(ambiguous) => Err(CliError(ambiguous)),
    }
}

/// The first argument as a reference, resolved; `usage` is the error when
/// there is none (`Usage: pty kill <name>` and friends).
pub fn require_ref(args: &[String], usage: &str) -> Result<String, CliError> {
    match args.first() {
        Some(r) => resolve_ref(r),
        None => Err(CliError(usage.to_string())),
    }
}

/// Options the interactive picker is opened with.
#[derive(Debug, Clone, Default)]
pub struct InteractiveOptions {
    pub preselect_new: bool,
    pub filter_tags: TagMap,
    pub force: bool,
}

/// `pty` / `pty i` / `pty interactive`: the nesting guard, then the TUI.
/// The picker itself is supplied by the TUI crate; until it lands this
/// reports the absence and exits 1.
///
/// node: src/cli.ts:84-93 (`runInteractive` wrapper)
pub fn interactive(opts: InteractiveOptions) -> CliResult {
    if !opts.force
        && let Ok(nested) = std::env::var("PTY_SESSION")
        && !nested.is_empty()
    {
        eprintln!("pty interactive: already inside pty session \"{nested}\".");
        eprintln!(
            "  The interactive picker would render inside your current session and detach would route to the outer client."
        );
        eprintln!(
            "  Detach first (Ctrl+\\) and run `pty` from outside, or pass --force to open the picker anyway."
        );
        return Ok(1);
    }
    run_interactive(opts)
}

/// The picker entry point (replaced by the TUI crate's `interactive::run`).
fn run_interactive(_opts: InteractiveOptions) -> CliResult {
    eprintln!("pty interactive: not implemented yet");
    Ok(1)
}

/// Node's `main()`: returns the process exit code.
///
/// node: src/cli.ts:670-760, 1626-1660
pub fn dispatch(mut args: Vec<String>) -> i32 {
    // Global `--root <path>`, scanned across the whole argv, first
    // occurrence only (cli.ts:677-686).
    if let Some(idx) = args.iter().position(|a| a == "--root") {
        match args.get(idx + 1) {
            Some(val) if !val.starts_with('-') => {
                // SAFETY: single-threaded at this point; nothing else reads
                // the environment concurrently.
                unsafe { std::env::set_var("PTY_ROOT", val) };
                args.drain(idx..idx + 2);
            }
            _ => {
                eprintln!("pty: --root requires a path (e.g. pty --root /var/lib/pty-eval list)");
                return 1;
            }
        }
    }

    // Root-length backstop before any subcommand runs (cli.ts:703-717).
    if let Some(msg) = registry::root_length_check() {
        eprintln!("{msg}");
        return 1;
    }

    // Subcommand detection: the first token that is not a flag, skipping the
    // value after `--filter-tag` (cli.ts:726-733).
    let mut subcommand = "";
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--filter-tag" {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        subcommand = a;
        break;
    }
    let subcommand = subcommand.to_string();

    let mut interactive_opts = InteractiveOptions::default();
    if subcommand.is_empty() || subcommand == "i" || subcommand == "interactive" {
        interactive_opts.preselect_new = args.iter().any(|a| a == "--preselect-new");
        interactive_opts.force = args.iter().any(|a| a == "--force");
        match registry::extract_filter_tags(&mut args) {
            Ok(tags) => interactive_opts.filter_tags = tags,
            Err(msg) => {
                eprintln!("{msg}");
                return 1;
            }
        }
    }
    let dispatch_args: Vec<&String> = args
        .iter()
        .filter(|a| *a != "--preselect-new" && *a != "--force")
        .collect();

    if dispatch_args.is_empty() {
        return finish(interactive(interactive_opts));
    }
    let command = dispatch_args[0].clone();

    // A subcommand's own `--help` / `-h` in the first position after the
    // command prints the focused help and exits 0 (cli.ts:756-758).
    if matches!(args.get(1).map(String::as_str), Some("-h") | Some("--help"))
        && let Some(text) = help::command_help(&command)
    {
        print_stdout(text);
        return 0;
    }

    // Subcommands parse `args` (which still holds `--force`), not
    // `dispatch_args`, as Node does.
    let rest = if args.len() > 1 { &args[1..] } else { &[][..] };
    let result = match command.as_str() {
        "interactive" | "i" => interactive(interactive_opts),
        "__daemon" => cmd_daemon(rest),
        "run" => run::run(rest),
        "attach" | "a" => attach::run(rest),
        "exec" => exec::run(rest),
        "peek" => peek::run(rest),
        "send" => send::run(rest),
        "events" => events::run(rest),
        "list" | "ls" => list::run(&args),
        "remote-serve" => {
            eprintln!("pty remote-serve: not implemented yet");
            Ok(1)
        }
        "stats" => stats::run(rest),
        "restart" => restart::run(rest),
        "kill" => kill::run(rest),
        "recover" => deferred::run("recover"),
        "gc" => gc::run(rest),
        "tag" => tag::run(rest),
        "tag-multi" => tag_multi::run(rest),
        "emit" => emit::run(rest),
        "up" => up::run(rest),
        "down" => down::run(rest),
        "rename" => rename::run(rest),
        "metadata" => metadata::run(rest),
        "evidence" => deferred::run("evidence"),
        "rm" | "remove" => rm::run(rest),
        "test" => deferred::run("test"),
        "completions" => Ok(completions::run(rest)),
        "version" | "--version" | "-v" | "-V" => version::run(),
        "help" | "--help" | "-h" => {
            help::print_usage();
            Ok(0)
        }
        other => forward_or_unknown(other, rest),
    };
    finish(result)
}

/// Print an `Err` the way `main().catch` does and turn it into exit 1.
fn finish(result: CliResult) -> i32 {
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn print_stdout(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

/// Git-style forwarding: `which pty-<cmd>`; found → run it with inherited
/// stdio and the unfiltered args, exit with its status (`?? 1`); else
/// `Unknown command: <cmd>` on stderr and the full usage on stdout, exit 1.
///
/// node: src/cli.ts:1641-1660
fn forward_or_unknown(command: &str, rest: &[String]) -> CliResult {
    let ext = std::process::Command::new("which")
        .arg(format!("pty-{command}"))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|p| !p.is_empty());
    if let Some(path) = ext {
        let status = std::process::Command::new(path).args(rest).status();
        return Ok(status.ok().and_then(|s| s.code()).unwrap_or(1));
    }
    eprintln!("Unknown command: {command}");
    help::print_usage();
    Ok(1)
}

// ── Daemon adapter ──────────────────────────────────────────────────────
//
// The daemon lane replaces this with `daemon::launch::spawn_daemon`. Until
// then the CLI launches `<current_exe> __daemon ...` itself; `SpawnParams`
// already has the shape that API takes so callers do not change.

/// Parameters for launching a session daemon.
#[derive(Debug, Clone, Default)]
pub struct SpawnParams {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub display_command: String,
    pub cwd: String,
    pub rows: u16,
    pub cols: u16,
    pub ephemeral: bool,
    pub tags: TagMap,
    pub display_name: Option<String>,
    pub extra_env: EnvMap,
}

impl SpawnParams {
    /// Parameters for `command` with the CLI's terminal size (or 24×80).
    pub fn new(name: &str, command: &str, args: &[String]) -> Self {
        let (rows, cols) = client::tty::size_or_default(libc::STDOUT_FILENO);
        SpawnParams {
            name: name.to_string(),
            command: command.to_string(),
            args: args.to_vec(),
            display_command: std::iter::once(command.to_string())
                .chain(args.iter().cloned())
                .collect::<Vec<_>>()
                .join(" "),
            rows,
            cols,
            ..Default::default()
        }
    }
}

/// Spawn a detached session daemon and wait for it to come up.
pub fn spawn_daemon(p: &SpawnParams) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find own executable: {e}"))?;
    let mut dcmd = std::process::Command::new(exe);
    dcmd.arg("__daemon")
        .arg("--name")
        .arg(&p.name)
        .arg("--rows")
        .arg(p.rows.to_string())
        .arg("--cols")
        .arg(p.cols.to_string())
        .arg("--cwd")
        .arg(&p.cwd)
        .arg("--display-command")
        .arg(&p.display_command);
    if let Some(dn) = &p.display_name {
        dcmd.arg("--display-name").arg(dn);
    }
    if p.ephemeral {
        dcmd.arg("--ephemeral");
    }
    for (k, v) in &p.tags {
        dcmd.arg("--tag").arg(format!("{k}={v}"));
    }
    for (k, v) in &p.extra_env {
        dcmd.arg("--env").arg(format!("{k}={v}"));
    }
    dcmd.arg("--").arg(&p.command).args(&p.args);
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
    let mut child = dcmd
        .spawn()
        .map_err(|e| format!("failed to start daemon: {e}"))?;

    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        // Ready if the socket is connectable...
        if client::is_alive(&p.name) {
            return Ok(());
        }
        // ...or the session already ran and exited in PRESERVE mode (metadata
        // records the exit)...
        if let Some(meta) = registry::read_metadata(&p.name)
            && meta.exit_code.is_some()
        {
            return Ok(());
        }
        // ...or the daemon process itself has already exited — a fast-exiting
        // command may have run and been REAPED (leaving no trace) before we
        // observed the socket. The daemon exiting means the session ran.
        if let Ok(Some(_)) = child.try_wait() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Err("daemon did not come up in time".into())
}

/// Internal: `pty __daemon --name N --rows R --cols C --cwd D
/// [--display-name X] [--display-command X] [--ephemeral] [--tag k=v]...
/// [--env K=V]... -- <cmd> [args...]`
fn cmd_daemon(args: &[String]) -> CliResult {
    let mut name = String::new();
    let mut rows = 24u16;
    let mut cols = 80u16;
    let mut cwd = String::from(".");
    let mut display_name: Option<String> = None;
    let mut display_command: Option<String> = None;
    let mut ephemeral = false;
    let mut tags: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut env: Vec<(String, String)> = Vec::new();
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
            "--env" => {
                if let Some((k, v)) = args.get(i + 1).and_then(|kv| kv.split_once('=')) {
                    env.push((k.to_string(), v.to_string()));
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
            "--display-command" => {
                display_command = args.get(i + 1).cloned();
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
        return Ok(2);
    }
    let (program, cargs) = command.split_first().unwrap();
    let cfg = crate::daemon::DaemonConfig {
        name,
        command: program.clone(),
        args: cargs.to_vec(),
        display_command: display_command.unwrap_or_else(|| command.join(" ")),
        cwd,
        rows,
        cols,
        env,
        ephemeral,
        tags,
        display_name,
    };
    match crate::daemon::run(cfg) {
        Ok(code) => Ok(code),
        Err(e) => {
            eprintln!("pty daemon error: {e}");
            Ok(1)
        }
    }
}
