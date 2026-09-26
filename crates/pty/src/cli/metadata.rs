//! `pty metadata patch|cas --id <stable-id>`: filesystem-backed metadata
//! mutations for one exact session id.

use std::io::Read;

use pty_core::registry::{
    MetadataPatch, TagCompareAndSetRequest, compare_and_set_tag_value, patch_metadata_by_id,
    validate_name,
};

use super::{CliError, CliResult, help};

/// Metadata mutation commands.
pub fn run(raw_args: &[String]) -> CliResult {
    let Some(operation @ ("patch" | "cas")) = raw_args.first().map(String::as_str) else {
        eprintln!("pty metadata: expected subcommand \"patch\" or \"cas\".");
        eprintln!("  Usage: pty metadata patch|cas --id <stable-id>");
        return Ok(1);
    };
    if matches!(
        raw_args.get(1).map(String::as_str),
        Some("-h") | Some("--help")
    ) {
        print!("{}", help::command_help("metadata").unwrap_or_default());
        return Ok(0);
    }

    let mut id: Option<String> = None;
    let mut index = 1;
    while index < raw_args.len() {
        match raw_args[index].as_str() {
            "--id" => {
                let Some(value) = raw_args.get(index + 1).filter(|value| !value.is_empty()) else {
                    return Err(format!(
                        "pty metadata {operation}: --id requires a stable session id."
                    )
                    .into());
                };
                if id.is_some() {
                    return Err(format!(
                        "pty metadata {operation}: --id may only be provided once."
                    )
                    .into());
                }
                id = Some(value.clone());
                index += 2;
            }
            argument => {
                eprintln!("pty metadata {operation}: unexpected argument \"{argument}\".");
                eprintln!("  Usage: pty metadata {operation} --id <stable-id>");
                return Ok(1);
            }
        }
    }
    let Some(id) = id else {
        return Err(format!("pty metadata {operation}: missing required --id <stable-id>.").into());
    };

    let mut input = String::new();
    std::io::stdin()
        .lock()
        .read_to_string(&mut input)
        .map_err(|error| CliError(format!("pty metadata {operation}: {error}")))?;
    let input = input.trim();
    if input.is_empty() {
        if operation == "patch" {
            eprintln!("pty metadata patch: expected one JSON patch object on stdin.");
            eprintln!(
                "  Example: printf '%s' '{{\"displayName\":\"Worker\"}}' | pty metadata patch --id a1b2c3d4"
            );
            return Ok(1);
        }
        return Err(CliError(
            "pty metadata cas: expected one JSON request object on stdin.".to_string(),
        ));
    }

    let value: serde_json::Value = serde_json::from_str(input).map_err(|error| {
        CliError(format!(
            "pty metadata {operation}: invalid JSON on stdin: {error}"
        ))
    })?;
    let output = if operation == "patch" {
        let patch = MetadataPatch::from_json(&value)
            .map_err(|error| CliError(format!("pty metadata patch: {error}")))?;
        let result = patch_metadata_by_id(&id, &patch)
            .map_err(|error| CliError(format!("pty metadata patch: {error}")))?;
        serde_json::to_string(&result)
    } else {
        validate_name(&id).map_err(CliError)?;
        let request: TagCompareAndSetRequest = serde_json::from_value(value)
            .ok()
            .filter(TagCompareAndSetRequest::validate)
            .ok_or_else(|| CliError("pty metadata cas: invalid request object.".to_string()))?;
        let result = compare_and_set_tag_value(
            &id,
            &request.expected_generation,
            &request.tag,
            &request.expected_value,
            &request.value,
        );
        serde_json::to_string(&result)
    }
    .map_err(|error| CliError(error.to_string()))?;
    println!("{output}");
    Ok(0)
}
