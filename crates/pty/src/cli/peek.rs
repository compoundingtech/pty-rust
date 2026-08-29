//! `pty peek` — interim port kept from the v0 binary. The Node-exact
//! rewrite (cli.ts:1068-1107, `cmdPeek` 1992-2014, `cmdPeekWait`
//! 1941-1990) replaces this module.

use pty_core::client;
use pty_core::registry;

use super::{CliResult, resolve_ref};

/// `pty peek [--plain] [--full] [-f] [--wait TEXT [-t SECS]] <ref>`
pub fn run(args: &[String]) -> CliResult {
    let mut plain = false;
    let mut full = false;
    let mut follow = false;
    let mut wait: Vec<String> = Vec::new();
    let mut timeout = 5f64;
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
                if let Some(v) = args.get(i + 1) {
                    wait.push(v.clone());
                }
                i += 2;
            }
            "-t" | "--timeout" => {
                timeout = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(5.0);
                i += 2;
            }
            other => {
                reference = Some(other.to_string());
                i += 1;
            }
        }
    }
    let Some(reference) = reference else {
        eprintln!("Usage: pty peek [--plain] [--full] [-f] [--wait <text>] [-t <seconds>] <name>");
        return Ok(1);
    };
    let name = resolve_ref(&reference)?;
    if follow {
        let params = client::PeekParams { name: &name, plain, full, socket: None };
        return match client::follow(params, &client::ClientIo::default()) {
            Ok(client::PeekOutcome::Exited(code)) => Ok(code.max(0)),
            Ok(_) => Ok(0),
            Err(e) => {
                eprintln!("{e}");
                Ok(1)
            }
        };
    }
    if !wait.is_empty() {
        return match client::peek_wait(&name, &wait, timeout, plain) {
            Ok(screen) => {
                println!("{screen}");
                Ok(0)
            }
            Err(e) => {
                eprintln!("{e}");
                Ok(1)
            }
        };
    }
    let params = client::PeekParams { name: &name, plain, full, socket: None };
    match client::peek(params, &client::ClientIo::default()) {
        Ok(client::PeekOutcome::Exited(code)) => Ok(code.max(0)),
        Ok(_) => Ok(0),
        // Socket gone: node prints the `lastLines` an exited session saved,
        // or says there is no saved output. Both exit 0.
        Err(client::ClientError::NotReachable { .. })
            if registry::read_metadata(&name).is_some() =>
        {
            let meta = registry::read_metadata(&name).unwrap();
            match meta.last_lines.as_deref() {
                Some(lines) if !lines.is_empty() => {
                    println!("{}", lines.join("\n"));
                }
                _ => {
                    let status = if meta.exited_at.is_some() { "exited" } else { "vanished" };
                    eprintln!("Session \"{name}\" has {status} with no saved output.");
                }
            }
            Ok(0)
        }
        Err(e) => {
            eprintln!("{e}");
            Ok(1)
        }
    }
}
