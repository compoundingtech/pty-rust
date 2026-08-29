//! `pty metadata patch --id <stable-id>`: merge one JSON patch object from
//! stdin into a session's presentation metadata (`displayName`, `tags`).
//!
//! node: src/cli.ts:1594-1597, 2815-2873

use std::io::Read;

use pty_core::registry::{MetadataPatch, patch_metadata_by_id};

use super::{CliError, CliResult, help};

/// `cmdMetadata`.
pub fn run(raw_args: &[String]) -> CliResult {
    let first = raw_args.first().map(String::as_str);
    if first == Some("patch")
        && matches!(
            raw_args.get(1).map(String::as_str),
            Some("-h") | Some("--help")
        )
    {
        print!("{}", help::command_help("metadata").unwrap_or_default());
        return Ok(0);
    }
    if first != Some("patch") {
        eprintln!("pty metadata: expected subcommand \"patch\".");
        eprintln!("  Usage: pty metadata patch --id <stable-id>");
        return Ok(1);
    }

    let mut id: Option<String> = None;
    let mut i = 1;
    while i < raw_args.len() {
        let arg = raw_args[i].as_str();
        if arg == "--id" {
            i += 1;
            let Some(value) = raw_args.get(i).filter(|v| !v.is_empty()) else {
                return Err("pty metadata patch: --id requires a stable session id.".into());
            };
            if id.is_some() {
                return Err("pty metadata patch: --id may only be provided once.".into());
            }
            id = Some(value.clone());
        } else {
            eprintln!("pty metadata patch: unexpected argument \"{arg}\".");
            eprintln!("  Usage: pty metadata patch --id <stable-id>");
            return Ok(1);
        }
        i += 1;
    }
    let Some(id) = id else {
        return Err("pty metadata patch: missing required --id <stable-id>.".into());
    };

    let mut input = String::new();
    let _ = std::io::stdin().lock().read_to_string(&mut input);
    let input = input.trim();
    if input.is_empty() {
        eprintln!("pty metadata patch: expected one JSON patch object on stdin.");
        eprintln!(
            "  Example: printf '%s' '{{\"displayName\":\"Worker\"}}' | pty metadata patch --id a1b2c3d4"
        );
        return Ok(1);
    }

    let value: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => {
            return Err(CliError(format!(
                "pty metadata patch: invalid JSON on stdin: {e}"
            )));
        }
    };
    let patch = MetadataPatch::from_json(&value)
        .map_err(|e| CliError(format!("pty metadata patch: {e}")))?;
    let result = patch_metadata_by_id(&id, &patch)
        .map_err(|e| CliError(format!("pty metadata patch: {e}")))?;
    println!("{}", serde_json::to_string(&result).unwrap_or_default());
    Ok(0)
}
