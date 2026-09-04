//! `pty rename`: set, show or clear a session's display name.
//!
//! node: src/cli.ts:1589-1592, 2809-2813 (`renameUsage`), 2926-3034 (`cmdRename`)

use pty_core::registry::{self, set_display_name, validate_display_name};

use super::{CliError, CliResult, help};

/// The rename help, to stderr for the error paths.
fn usage() {
    eprint!("{}", help::command_help("rename").unwrap_or_default());
}

fn lookup(reference: &str) -> Result<registry::SessionInfo, CliError> {
    match registry::get_session(reference).map_err(CliError)? {
        Some(s) => Ok(s),
        None => Err(CliError(format!("Session \"{reference}\" not found."))),
    }
}

fn inside_session() -> Option<String> {
    std::env::var("PTY_SESSION").ok().filter(|s| !s.is_empty())
}

/// `cmdRename`.
pub fn run(raw_args: &[String]) -> CliResult {
    let mut show = false;
    let mut clear = false;
    let mut positional: Vec<&str> = Vec::new();
    for a in raw_args {
        match a.as_str() {
            "--show" => show = true,
            "--clear" => clear = true,
            "-h" | "--help" => {
                usage();
                return Ok(0);
            }
            other => positional.push(other),
        }
    }

    if show {
        if positional.len() != 1 {
            eprintln!("pty rename --show requires exactly one ref.");
            usage();
            return Ok(1);
        }
        let session = lookup(positional[0])?;
        match session.display_name().filter(|d| !d.is_empty()) {
            Some(dn) => println!("{dn}"),
            None => println!(
                "(no displayName; session is referenced by its id: {})",
                session.name
            ),
        }
        return Ok(0);
    }

    if clear {
        let target = match positional.as_slice() {
            [] => match inside_session() {
                Some(id) => id,
                None => {
                    eprintln!(
                        "pty rename --clear with no ref requires being inside a pty session (PTY_SESSION not set)."
                    );
                    usage();
                    return Ok(1);
                }
            },
            [reference] => lookup(reference)?.name,
            _ => {
                eprintln!("pty rename --clear takes at most one ref.");
                usage();
                return Ok(1);
            }
        };
        set_display_name(&target, None).map_err(CliError)?;
        println!("Cleared displayName on \"{target}\".");
        return Ok(0);
    }

    let (target, new_display) = match positional.as_slice() {
        [dn] => match inside_session() {
            Some(id) => (id, dn.to_string()),
            None => {
                eprintln!("pty rename with a single arg is only allowed inside a pty session.");
                eprintln!("Outside, use: pty rename <ref> <new-display-name>");
                usage();
                return Ok(1);
            }
        },
        [reference, dn] => (lookup(reference)?.name, dn.to_string()),
        _ => {
            usage();
            return Ok(1);
        }
    };

    if let Err(e) = validate_display_name(&new_display) {
        return Err(CliError(format!("Invalid displayName: {e}")));
    }
    set_display_name(&target, Some(&new_display)).map_err(CliError)?;
    println!("Set displayName on \"{target}\" \u{2192} \"{new_display}\".");
    Ok(0)
}
