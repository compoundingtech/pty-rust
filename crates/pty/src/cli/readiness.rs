//! `pty readiness ownership|cas`: one JSON request on stdin, one tagged JSON
//! result on stdout.

use std::io::{Read, Write};

use pty_core::client::{
    compare_and_set_lifecycle, compare_and_set_lifecycle_with_capability_fd,
    query_accepted_socket_ownership, query_accepted_socket_ownership_with_capability_fd,
};
use pty_core::protocol::{AcceptedSocketOwnershipRequest, LifecycleCompareAndSetRequest};
use super::{CliError, CliResult};

pub fn run(args: &[String]) -> CliResult {
    let Some(operation @ ("ownership" | "cas")) = args.first().map(String::as_str) else {
        return Err(CliError(
            "pty readiness: expected subcommand \"ownership\" or \"cas\".".to_string(),
        ));
    };
    if args.len() == 2 && matches!(args[1].as_str(), "-h" | "--help") {
        print!(
            "{}",
            super::help::readiness_leaf_help(operation).unwrap_or("")
        );
        return Ok(0);
    }
    let mut id = None;
    let mut capability_fd = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--id" => {
                let Some(value) = args.get(index + 1).filter(|value| !value.is_empty()) else {
                    return Err(CliError(format!(
                        "pty readiness {operation}: --id requires a stable session id."
                    )));
                };
                if id.is_some() {
                    return Err(CliError(format!(
                        "pty readiness {operation}: --id may only be provided once."
                    )));
                }
                id = Some(value.clone());
                index += 2;
            }
            "--capability-fd" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError(format!(
                        "pty readiness {operation}: --capability-fd requires a file descriptor."
                    )));
                };
                if capability_fd.is_some() {
                    return Err(CliError(format!(
                        "pty readiness {operation}: --capability-fd may only be provided once."
                    )));
                }
                let fd = value.parse::<i32>().ok().filter(|fd| *fd >= 0).ok_or_else(|| {
                    CliError(format!(
                        "pty readiness {operation}: --capability-fd requires a non-negative integer."
                    ))
                })?;
                capability_fd = Some(fd);
                index += 2;
            }
            _ => {
                return Err(CliError(format!(
                    "pty readiness {operation}: unexpected argument \"{}\".",
                    args[index]
                )));
            }
        }
    }
    let Some(id) = id else {
        return Err(CliError(format!(
            "pty readiness {operation}: missing required --id <stable-id>."
        )));
    };

    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| CliError(format!("pty readiness {operation}: {error}")))?;
    let input = input.trim();
    if input.is_empty() {
        return Err(CliError(format!(
            "pty readiness {operation}: expected one JSON object on stdin."
        )));
    }
    let value: serde_json::Value = serde_json::from_str(input).map_err(|error| {
        CliError(format!(
            "pty readiness {operation}: invalid JSON on stdin: {error}"
        ))
    })?;
    let output = if operation == "ownership" {
        let request: AcceptedSocketOwnershipRequest = serde_json::from_value(value)
            .ok()
            .filter(AcceptedSocketOwnershipRequest::validate)
            .ok_or_else(|| {
                CliError("pty readiness ownership: invalid request object.".to_string())
            })?;
        let result = match capability_fd {
            Some(fd) => query_accepted_socket_ownership_with_capability_fd(&id, &request, fd),
            None => query_accepted_socket_ownership(&id, &request),
        }
        .map_err(|error| CliError(error.to_string()))?;
        serde_json::to_string(&result)
    } else {
        let request: LifecycleCompareAndSetRequest = serde_json::from_value(value)
            .ok()
            .filter(LifecycleCompareAndSetRequest::validate)
            .ok_or_else(|| CliError("pty readiness cas: invalid request object.".to_string()))?;
        let result = match capability_fd {
            Some(fd) => compare_and_set_lifecycle_with_capability_fd(&id, &request, fd),
            None => compare_and_set_lifecycle(&id, &request),
        }
        .map_err(|error| CliError(error.to_string()))?;
        serde_json::to_string(&result)
    }
    .map_err(|error| CliError(error.to_string()))?;
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(format!("{output}\n").as_bytes())
        .map_err(|error| CliError(error.to_string()))?;
    Ok(0)
}
