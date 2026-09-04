//! `pty emit [<ref>] <type> [--json <payload>] [--text <string>]`: append a
//! `user.*` event to a session's log. Inside a session `$PTY_SESSION` is
//! the default ref.
//!
//! node: src/cli.ts:1546-1549, 3515-3579

use pty_core::events::emit_user_event;

use super::{CliError, CliResult, help, resolve_ref};

fn print_help() {
    print!("{}", help::command_help("emit").unwrap_or_default());
}

/// `cmdEmit`.
pub fn run(argv: &[String]) -> CliResult {
    let mut json_str: Option<&str> = None;
    let mut text_str: Option<&str> = None;
    let mut positional: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        if a == "--json" && i + 1 < argv.len() {
            i += 1;
            json_str = Some(&argv[i]);
        } else if a == "--text" && i + 1 < argv.len() {
            i += 1;
            text_str = Some(&argv[i]);
        } else if a == "-h" || a == "--help" {
            print_help();
            return Ok(0);
        } else {
            positional.push(a);
        }
        i += 1;
    }

    let (reference, event_type) = match positional.as_slice() {
        [r, t] => (Some(r.to_string()), *t),
        [t] => (None, *t),
        _ => {
            print_help();
            return Ok(1);
        }
    };

    // Default to $PTY_SESSION when no explicit ref is given.
    let reference = reference.or_else(|| std::env::var("PTY_SESSION").ok());
    let Some(reference) = reference.filter(|r| !r.is_empty()) else {
        eprintln!("pty emit: no session ref given and not running inside a pty session");
        eprintln!("  tip: run inside a pty session, or: pty emit <session-ref> <type>");
        return Ok(1);
    };
    let name = resolve_ref(&reference)?;

    let data = match json_str {
        Some(raw) => match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(v) => Some(v),
            Err(e) => {
                return Err(CliError(format!(
                    "pty emit: --json payload is not valid JSON: {e}"
                )));
            }
        },
        None => None,
    };

    emit_user_event(&name, event_type, data, text_str).map_err(CliError)?;
    Ok(0)
}
