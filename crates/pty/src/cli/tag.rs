//! `pty tag <ref>` shows a session's tags; `pty tag <ref> k=v... [--rm k]...`
//! writes them (updates first, then removals, one atomic write and one
//! `tags_change` event).
//!
//! node: src/cli.ts:1455-1539

use pty_core::registry::{self, TagMap};

use super::{CliError, CliResult, resolve_ref};

/// The parsed write operations.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TagOps {
    pub updates: TagMap,
    pub removals: Vec<String>,
}

impl TagOps {
    fn is_empty(&self) -> bool {
        self.updates.is_empty() && self.removals.is_empty()
    }
}

/// Parse the tokens after the ref: `k=v` (split on the first `=`, last
/// wins) and `--rm <key>` in any order. Errors carry Node's texts and abort
/// before any write.
///
/// node: src/cli.ts:1466-1490
pub fn parse_ops(tokens: &[String]) -> Result<TagOps, CliError> {
    let mut ops = TagOps::default();
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i].as_str();
        if tok == "--rm" {
            let Some(key) = tokens.get(i + 1) else {
                return Err("pty tag: --rm requires a key (e.g. --rm role)".into());
            };
            if key.is_empty() {
                return Err("pty tag: --rm requires a non-empty key".into());
            }
            ops.removals.push(key.clone());
            i += 2;
            continue;
        }
        let Some((key, value)) = tok.split_once('=') else {
            return Err(CliError(format!(
                "pty tag: invalid argument \"{tok}\". Use key=value or --rm key."
            )));
        };
        if key.is_empty() {
            return Err(CliError(format!(
                "pty tag: empty key in \"{tok}\". Tag keys must be non-empty."
            )));
        }
        ops.updates.insert(key.to_string(), value.to_string());
        i += 1;
    }
    Ok(ops)
}

/// `pty tag <ref> [key=value...] [--rm key...]`
pub fn run(args: &[String]) -> CliResult {
    let Some(reference) = args.first() else {
        return Err("Usage: pty tag <name> [key=value...] [--rm key...]".into());
    };
    let name = resolve_ref(reference)?;
    let ops = parse_ops(&args[1..])?;

    if ops.is_empty() {
        let Some(meta) = registry::read_metadata(&name) else {
            return Err(CliError(format!("Session \"{reference}\" not found.")));
        };
        match meta.tags.as_ref().filter(|t| !t.is_empty()) {
            None => println!("No tags on \"{name}\"."),
            Some(tags) => {
                for (k, v) in tags {
                    println!("  {k}={v}");
                }
            }
        }
        return Ok(0);
    }

    // Is the session managed by a pty.toml? Checked before the write so the
    // warning reflects what the operator is overriding.
    let ptyfile_path = registry::read_metadata(&name)
        .and_then(|m| m.tags)
        .and_then(|t| t.get("ptyfile").cloned());

    registry::update_tags(&name, &ops.updates, &ops.removals).map_err(CliError)?;
    let meta = registry::read_metadata(&name);
    match meta.and_then(|m| m.tags).filter(|t| !t.is_empty()) {
        None => println!("Tags cleared on \"{name}\"."),
        Some(tags) => {
            println!("Tags on \"{name}\":");
            for (k, v) in &tags {
                println!("  {k}={v}");
            }
        }
    }

    if let Some(path) = ptyfile_path {
        eprintln!("\nWarning: this session is managed by {path}");
        eprintln!("Running 'pty up' will sync tags from the toml and may overwrite this change.");
        eprintln!("To make it permanent, edit the pty.toml file directly.");
    }
    Ok(0)
}
