//! `pty run` — interim port kept from the v0 binary so the surface builds
//! and `run -d` works for tests. The Node-exact rewrite (cli.ts:767-982,
//! `cmdRun` 1664-1769) replaces this module.

use pty_core::client;
use pty_core::registry::{self, EnvMap, TagMap};

use super::{CliResult, SpawnParams};

/// `pty run [--id X] [--name X] [--cwd D] [--tag k=v] [--env K=V]
/// [--unset-env K] [--isolate-env] [--rows R] [--cols C] -- <cmd...>`
pub fn run(args: &[String]) -> CliResult {
    let mut id: Option<String> = None;
    let mut display_name: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut rows: Option<u16> = None;
    let mut cols: Option<u16> = None;
    let mut background = false;
    let mut force = false;
    let mut ephemeral = false;
    let mut attach_existing = false;
    let mut isolate_env = false;
    let mut tags = TagMap::new();
    let mut extra_env = EnvMap::new();
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
            "--cwd" => {
                cwd = args.get(i + 1).cloned();
                i += 2;
            }
            "--rows" => {
                rows = args.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "--cols" => {
                cols = args.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "-d" | "--detach" => {
                background = true;
                i += 1;
            }
            "--force" => {
                // --force creates even from inside a pty session (bypasses
                // the nesting guard), symmetric with attach's --force.
                force = true;
                i += 1;
            }
            "-e" | "--ephemeral" => {
                ephemeral = true;
                i += 1;
            }
            "--tag" => {
                match args.get(i + 1).and_then(|kv| kv.split_once('=')) {
                    Some((k, v)) => {
                        tags.insert(k.to_string(), v.to_string());
                    }
                    None => {
                        let tok = args.get(i + 1).cloned().unwrap_or_default();
                        eprintln!("Invalid tag format: \"{tok}\". Use --tag key=value");
                        return Ok(1);
                    }
                }
                i += 2;
            }
            // An assignment whose `=` is missing or leading is rejected;
            // `KEY=` with an empty value is accepted. A repeated key keeps
            // its first position and takes the last value.
            //
            // node: src/cli.ts:811-814
            "--env" => {
                let tok = args.get(i + 1).cloned().unwrap_or_default();
                match tok.find('=') {
                    Some(eq) if eq > 0 => {
                        extra_env.insert(tok[..eq].to_string(), tok[eq + 1..].to_string());
                    }
                    _ => {
                        eprintln!("Invalid env format: \"{tok}\". Use --env KEY=VALUE");
                        return Ok(1);
                    }
                }
                i += 2;
            }
            // De-duplicated, order preserved.
            //
            // node: src/cli.ts:820-823
            "--unset-env" => {
                let tok = args.get(i + 1).cloned().unwrap_or_default();
                if tok.is_empty() || tok.contains('=') {
                    eprintln!("Invalid env key: \"{tok}\". Use --unset-env KEY");
                    return Ok(1);
                }
                if !unset_env.contains(&tok) {
                    unset_env.push(tok);
                }
                i += 2;
            }
            "--isolate-env" => {
                isolate_env = true;
                i += 1;
            }
            "-a" | "--attach" => {
                attach_existing = true;
                i += 1;
            }
            "--no-display-name" => {
                // Accepted for CLI compatibility.
                i += 1;
            }
            "--" => {
                command = args[i + 1..].to_vec();
                break;
            }
            _ => {
                // Bare command without `--`.
                command = args[i..].to_vec();
                break;
            }
        }
    }
    if command.is_empty() {
        eprintln!("Usage: pty run [--id <id>] [--name <displayName>] [-d] [-a] -- <command> [args...]");
        return Ok(1);
    }

    // Nesting prevention: running `pty run` from inside a pty session would
    // create a session-inside-a-session. Run the command directly unless
    // `-d` or `--force` explicitly asks for a real nested session.
    if !background
        && !force
        && std::env::var("PTY_SESSION").map(|v| !v.is_empty()).unwrap_or(false)
    {
        // A plain nested `run` executes in place. `-a` is narrower: it asked
        // to attach if the target is already running, and attaching would
        // nest a client, so that case refuses instead.
        if attach_existing
            && let Some(reference) = id.as_ref().or(display_name.as_ref())
        {
            let existing = match &id {
                Some(explicit) => registry::get_session_by_name(explicit),
                None => registry::get_session(reference).ok().flatten(),
            };
            if existing.is_some_and(|s| s.status == registry::SessionStatus::Running)
                && let Some(code) = super::ensure_not_nested(
                    "run -a",
                    false,
                    Some(&format!(
                        "  Target session \"{reference}\" is already running; attaching would nest a client inside the current session.\n  Pass --force to attach anyway, or detach first (Ctrl+\\) and re-run from outside."
                    )),
                )
            {
                return Ok(code);
            }
        }
        let nested = std::env::var("PTY_SESSION").unwrap_or_default();
        eprintln!("Already inside pty session \"{nested}\", running directly.");
        let (program, pargs) = command.split_first().unwrap();
        let mut c = std::process::Command::new(program);
        c.args(pargs);
        // The child inherits this process's environment, minus the
        // `--unset-env` keys, plus the `--env` overlays. An assignment beats
        // a removal of the same key, whatever order the flags came in.
        //
        // node: src/cli.ts:907-917
        for key in &unset_env {
            c.env_remove(key);
        }
        for (key, value) in &extra_env {
            c.env(key, value);
        }
        if let Some(dir) = &cwd {
            c.current_dir(dir);
        }
        return Ok(c.status().ok().and_then(|s| s.code()).unwrap_or(1));
    }

    let name = id.unwrap_or_else(registry::generate_id);
    if let Err(e) = registry::validate_name(&name) {
        eprintln!("{e}");
        return Ok(1);
    }
    if registry::session_exists(&name) && client::is_alive(&name) {
        eprintln!("Session id \"{name}\" is already in use.");
        return Ok(1);
    }

    let cwd = cwd.unwrap_or_else(|| crate::daemon::default_cwd().to_string_lossy().into_owned());

    let (program, pargs) = command.split_first().unwrap();
    let mut params = SpawnParams::new(&name, program, pargs);
    params.cwd = cwd;
    if let Some(r) = rows {
        params.rows = r;
    }
    if let Some(c) = cols {
        params.cols = c;
    }
    params.ephemeral = ephemeral;
    params.tags = tags;
    params.display_name = display_name;
    params.isolate_env = isolate_env;
    params.extra_env = extra_env;
    params.unset_env = unset_env;

    match super::spawn_daemon(&params) {
        Ok(()) => {
            println!("{name}");
            Ok(0)
        }
        Err(msg) => {
            eprintln!("pty run: {msg}");
            Ok(1)
        }
    }
}
