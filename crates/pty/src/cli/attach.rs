//! `pty attach` — interim port kept from the v0 binary. The Node-exact
//! rewrite (cli.ts:984-1053, `cmdAttach` 1773-1806) replaces this module.

use pty_core::client;

use super::{CliResult, resolve_ref};

/// `pty attach <ref>`
pub fn run(args: &[String]) -> CliResult {
    let reference = match args.iter().find(|a| !a.starts_with('-')) {
        Some(r) => r.clone(),
        None => {
            eprintln!("Usage: pty attach <name>");
            return Ok(1);
        }
    };
    let name = resolve_ref(&reference)?;
    eprintln!("[attached to {name} — press Ctrl+\\ to detach]");
    let socket = match client::connect_session(&name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return Ok(1);
        }
    };
    Ok(client::attach(client::AttachParams::new(&name, socket), &client::ClientIo::default())
        .exit_code())
}
