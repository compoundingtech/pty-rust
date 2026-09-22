//! `pty evidence snapshot|remove`: generation-scoped terminal evidence.

use pty_core::registry::{get_session_exit_evidence, remove_session_generation, validate_name};

use super::{CliError, CliResult};

pub fn run(args: &[String]) -> CliResult {
    let Some(operation @ ("snapshot" | "remove")) = args.first().map(String::as_str) else {
        return Err(CliError(
            "pty evidence: expected subcommand \"snapshot\" or \"remove\".\n  Usage: pty evidence snapshot --id <stable-id>\n         pty evidence remove --id <stable-id> --expected-generation <opaque>"
                .to_string(),
        ));
    };
    if args.len() == 2 && matches!(args[1].as_str(), "-h" | "--help") {
        print!(
            "{}",
            super::help::evidence_leaf_help(operation).unwrap_or("")
        );
        return Ok(0);
    }
    let mut id = None;
    let mut expected_generation = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--id" => {
                let Some(value) = args.get(index + 1).filter(|value| !value.is_empty()) else {
                    return Err(CliError(
                        "pty evidence: --id requires a stable session id.".to_string(),
                    ));
                };
                if id.is_some() {
                    return Err(CliError(
                        "pty evidence: --id may only be provided once.".to_string(),
                    ));
                }
                id = Some(value.clone());
                index += 2;
            }
            "--expected-generation" => {
                let Some(value) = args.get(index + 1).filter(|value| !value.is_empty()) else {
                    return Err(CliError(
                        "pty evidence: --expected-generation requires an opaque generation."
                            .to_string(),
                    ));
                };
                if expected_generation.is_some() {
                    return Err(CliError(
                        "pty evidence: --expected-generation may only be provided once."
                            .to_string(),
                    ));
                }
                expected_generation = Some(value.clone());
                index += 2;
            }
            argument => {
                return Err(CliError(format!(
                    "pty evidence: unexpected argument \"{argument}\"."
                )));
            }
        }
    }
    let Some(id) = id else {
        return Err(CliError(
            "pty evidence: missing required --id <stable-id>.".to_string(),
        ));
    };
    validate_name(&id).map_err(CliError)?;
    let result = if operation == "snapshot" {
        if expected_generation.is_some() {
            return Err(CliError(
                "pty evidence snapshot: --expected-generation is only valid for remove."
                    .to_string(),
            ));
        }
        serde_json::to_string(&get_session_exit_evidence(&id))
    } else {
        let Some(expected_generation) = expected_generation else {
            return Err(CliError(
                "pty evidence remove: missing required --expected-generation <opaque>.".to_string(),
            ));
        };
        let result = remove_session_generation(&id, &expected_generation)
            .map_err(|error| CliError(error.to_string()))?;
        serde_json::to_string(&result)
    }
    .map_err(|error| CliError(error.to_string()))?;
    println!("{result}");
    Ok(0)
}
