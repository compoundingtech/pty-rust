//! `pty restart` — interim port kept from the v0 binary. The Node-exact
//! rewrite (cli.ts:1358-1382, `cmdRestart` 3886-3963) replaces this module.

use pty_core::client;
use pty_core::registry;

use super::kill::kill_session;
use super::{CliResult, SpawnParams, resolve_ref};

/// `pty restart <ref>` — stop (if running) and respawn with the same command.
pub fn run(args: &[String]) -> CliResult {
    let reference = match args.iter().find(|a| !a.starts_with('-')) {
        Some(r) => r.clone(),
        None => {
            eprintln!("Usage: pty restart [-y] [--force] <name>");
            return Ok(1);
        }
    };
    let name = resolve_ref(&reference)?;
    let Some(meta) = registry::read_metadata(&name) else {
        eprintln!("Session \"{name}\" has no metadata — cannot restart.");
        return Ok(1);
    };
    if client::is_alive(&name) {
        kill_session(&name);
    }
    registry::cleanup(&name);

    let mut params = SpawnParams::new(&name, &meta.command, &meta.args);
    params.display_command = meta.display_command.clone();
    params.cwd = meta.cwd.clone();
    if let Some(r) = meta.rows {
        params.rows = r;
    }
    if let Some(c) = meta.cols {
        params.cols = c;
    }
    params.ephemeral = meta.ephemeral == Some(true);
    params.tags = registry::strip_gc_bookkeeping(meta.tags.as_ref()).unwrap_or_default();
    params.display_name = meta.display_name.clone();
    params.extra_env = meta.extra_env.clone().unwrap_or_default();
    match super::spawn_daemon(&params) {
        Ok(()) => {
            println!("Session \"{name}\" restarted.");
            Ok(0)
        }
        Err(e) => {
            eprintln!("pty restart: {e}");
            Ok(1)
        }
    }
}
