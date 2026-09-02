//! `pty exec -- <command> [args...]`: swap the command a session will run
//! on its next start, then run it here.
//!
//! It rewrites the session's stored command under the metadata lock, guarded
//! by the generation token the session handed this process, appends a
//! `session_exec` event, and only then runs the command. A session managed
//! by a `pty.toml` refuses, and so does one that a replacement generation has
//! taken over.
//!
//! node: src/cli.ts:1055-1066 (dispatch), 1865-1939 (`cmdExec`)

use pty_core::events::Event;
use pty_core::registry::{
    self, LockRefusal, MutateOptions, MutateStatus, event_lock_path, lock_or_refusal,
    release_event_lock,
};

use super::{CliError, CliResult};

const USAGE: &str = "Usage: pty exec -- <command> [args...]";

/// `cmdExec`.
pub fn run(args: &[String]) -> CliResult {
    // The dispatcher hands us everything after `exec`, so `--` sits at index
    // 0 here where Node finds it at index 1 of the whole argv.
    let Some(dash) = args.iter().position(|a| a == "--") else {
        eprintln!("{USAGE}");
        return Ok(1);
    };
    if dash + 1 >= args.len() {
        eprintln!("{USAGE}");
        return Ok(1);
    }
    let command = args[dash + 1].clone();
    let cmd_args: Vec<String> = args[dash + 2..].to_vec();

    let session = std::env::var("PTY_SESSION").unwrap_or_default();
    if session.is_empty() {
        eprintln!("pty exec: not inside a pty session (PTY_SESSION not set).");
        return Ok(1);
    }
    let owner_generation = std::env::var("PTY_SESSION_GENERATION").unwrap_or_default();
    if owner_generation.is_empty() {
        return Err(
            "pty exec: current session has no generation owner token; restart it before using pty exec."
                .into(),
        );
    }

    if registry::read_metadata(&session).is_none() {
        eprintln!("pty exec: session \"{session}\" metadata not found.");
        return Ok(1);
    }

    let resolved = match pty_core::spawn::resolve_command(&command) {
        Ok(path) => path,
        Err(msg) => {
            eprintln!("{msg}");
            return Ok(1);
        }
    };

    let display_command = std::iter::once(command.clone())
        .chain(cmd_args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");

    let _event_lock = lock_or_refusal(&event_lock_path(&session)).map_err(|r| match r {
        LockRefusal::Busy => {
            format!("pty exec: session \"{session}\" event log is busy; retry the operation.")
        }
        // The lock file could not be made. Asking for a retry would be a
        // lie: nothing about a read-only root changes on the next attempt.
        LockRefusal::Unavailable(cause) => format!("pty exec: {cause}"),
    })?;

    // Node throws out of the mutate callback for a pty.toml session. The
    // callback here reports it instead and writes nothing.
    // Both closures below need this, and they run one after the other on
    // this thread, so a cell is enough to share it.
    let previous_command = std::cell::RefCell::new(String::new());
    let mut managed_by: Option<String> = None;
    let status = registry::mutate_metadata_under_lock_with(
        &session,
        |current| {
            if let Some(path) = current.tags.as_ref().and_then(|t| t.get("ptyfile")) {
                managed_by = Some(path.clone());
                return false;
            }
            *previous_command.borrow_mut() = if current.display_command.is_empty() {
                std::iter::once(current.command.clone())
                    .chain(current.args.iter().cloned())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                current.display_command.clone()
            };
            current.command = resolved.clone();
            current.args = cmd_args.clone();
            current.display_command = display_command.clone();
            true
        },
        &MutateOptions {
            expected_generation: Some(owner_generation),
            ..Default::default()
        },
        |_| {
            let _ = pty_core::events::append_event_locked(
                &session,
                &Event::session_exec(&session, &previous_command.borrow(), &display_command),
            );
        },
    );
    release_event_lock(&session);

    if let Some(path) = managed_by {
        return Err(format!(
            "pty exec: session \"{session}\" is managed by {path}. \
             Edit the pty.toml to change the command instead."
        )
        .into());
    }
    if !matches!(status, MutateStatus::Changed(_)) {
        let reason = if status == MutateStatus::GenerationMismatch {
            "belongs to a replacement generation".to_string()
        } else {
            format!("could not be updated ({})", status.as_str())
        };
        return Err(CliError(format!(
            "pty exec: session \"{session}\" {reason}; command was not run."
        )));
    }

    // Node spawns and waits rather than replacing itself, so this process
    // stays the parent and the session keeps its process tree.
    let code = std::process::Command::new(&resolved)
        .args(&cmd_args)
        .status()
        .ok()
        .and_then(|s| s.code())
        .unwrap_or(1);
    Ok(code)
}
