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

use pty_core::client;
use pty_core::registry::{self, TagMap};

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

/// The variables a restarted session must not inherit from whoever asked
/// for the restart.
///
/// node: src/cli.ts:3869
pub const RESTART_SCRUBBED_ENV: [&str; 2] = ["ST_AGENT", "ST_ROOT"];

/// Refuse a command that would nest a client inside the current session.
/// Returns `Some(1)` when it refused, having printed the reason and the
/// hint; `None` means carry on.
///
/// node: src/cli.ts:626-637 (`ensureNotNested`)
pub fn ensure_not_nested(cmd: &str, force: bool, hint: Option<&str>) -> Option<i32> {
    if force {
        return None;
    }
    let nested = std::env::var("PTY_SESSION").unwrap_or_default();
    if nested.is_empty() {
        return None;
    }
    eprintln!("pty {cmd}: already inside pty session \"{nested}\".");
    match hint {
        Some(h) => eprintln!("{h}"),
        None => eprintln!("  Pass --force to override."),
    }
    Some(1)
}

/// Carry a session's launch-time settings into its replacement. Older
/// records simply have fewer of them.
///
/// node: src/cli.ts:3874-3884 (`persistedLaunchOptions`)
pub fn apply_persisted_launch_options(params: &mut SpawnParams, meta: &registry::SessionMetadata) {
    if let Some(rows) = meta.rows {
        params.rows = rows;
    }
    if let Some(cols) = meta.cols {
        params.cols = cols;
    }
    if let Some(ephemeral) = meta.ephemeral {
        params.ephemeral = ephemeral;
    }
    if meta.isolate_env == Some(true) {
        params.isolate_env = true;
    }
    if let Some(extra) = &meta.extra_env
        && !extra.is_empty()
    {
        params.extra_env = extra.clone();
    }
    if let Some(unset) = &meta.unset_env
        && !unset.is_empty()
    {
        params.unset_env = unset.clone();
    }
    if let Some(env) = &meta.env {
        params.env = Some(env.clone());
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
        "__daemon" => Ok(crate::daemon::daemon_main()),
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

// ── Daemon launch ───────────────────────────────────────────────────────

pub use crate::daemon::SpawnParams;

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

/// Resolve the command, spawn the detached session daemon and wait for it
/// to publish (`daemon::launch::spawn_daemon`).
///
/// node: src/spawn.ts:372-393, 164-243
pub fn spawn_daemon(p: &SpawnParams) -> Result<(), String> {
    let mut params = p.clone();
    params.command = pty_core::spawn::resolve_command(&p.command)?;
    crate::daemon::spawn_daemon(params)
        .map(|_| ())
        .map_err(|e| e.to_string())
}
