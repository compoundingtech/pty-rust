//! `pty events [--all] [--recent] [--json] [--wait <type>] [-t <sec>] [<ref>]`:
//! print a session's recent events, follow one log (or every log with
//! `--all`), or block until one event type appears.
//!
//! node: src/cli.ts:1219-1248 (parsing), 3965-4051 (`cmdEvents`)

use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use pty_core::events::follow::{EventFollower, FollowerOptions};
use pty_core::events::{DEFAULT_RECENT_EVENTS, Event, format_event, read_recent_events};
use pty_core::registry;

use super::argv::{Argv, js_number, js_parse_float};
use super::{CliError, CliResult, resolve_ref};

const USAGE: &str =
    "Usage: pty events [--all] [--recent] [--json] [--wait <type>] [-t <seconds>] [<name>]";

fn render(event: &Event, json: bool) -> String {
    if json {
        event.to_json()
    } else {
        format_event(event)
    }
}

extern "C" fn on_sigint(_sig: libc::c_int) {
    // SAFETY: `_exit` is async-signal-safe; Node exits 0 on SIGINT here.
    unsafe { libc::_exit(0) }
}

/// SIGINT ends a follow with exit 0.
///
/// node: src/cli.ts:4028-4031, 4044-4047
fn exit_zero_on_sigint() {
    // SAFETY: installing a handler that only calls `_exit`.
    let handler: extern "C" fn(libc::c_int) = on_sigint;
    unsafe {
        libc::signal(libc::SIGINT, handler as *const () as libc::sighandler_t);
    }
}

/// Parse and run.
pub fn run(args: &[String]) -> CliResult {
    let mut all = false;
    let mut recent = false;
    let mut json = false;
    let mut wait_type: Option<String> = None;
    let mut timeout = 0f64;
    let mut cur = Argv::new(args);
    // Leading dash-tokens are flags; an unknown one ends the loop and is the ref.
    while cur.at_dash() {
        match cur.peek() {
            Some("--all") => {
                all = true;
                cur.next();
            }
            Some("--recent") => {
                recent = true;
                cur.next();
            }
            Some("--json") => {
                json = true;
                cur.next();
            }
            Some("--wait") if cur.has_next() => {
                wait_type = cur.take_value().map(str::to_string);
            }
            Some("-t") | Some("--timeout") if cur.has_next() => {
                timeout = js_parse_float(cur.take_value().unwrap_or_default());
            }
            _ => break,
        }
    }
    let reference = cur.peek();

    if !all && reference.is_none() {
        return Err(USAGE.into());
    }
    let name = match reference {
        Some(r) => Some(resolve_ref(r)?),
        None => None,
    };
    cmd_events(name.as_deref(), all, recent, json, wait_type.as_deref(), timeout)
}

/// `cmdEvents`.
fn cmd_events(
    name: Option<&str>,
    all: bool,
    recent: bool,
    json: bool,
    wait_type: Option<&str>,
    timeout: f64,
) -> CliResult {
    if recent {
        let Some(name) = name else {
            return Err("--recent requires a session name.".into());
        };
        let events = read_recent_events(name, DEFAULT_RECENT_EVENTS);
        if events.is_empty() {
            println!("No recent events for \"{name}\".");
            return Ok(0);
        }
        for e in &events {
            println!("{}", render(e, json));
        }
        return Ok(0);
    }

    // Follow mode: re-check the session exists when a name was given.
    if let Some(name) = name
        && registry::get_session(name).map_err(CliError)?.is_none()
    {
        return Err(CliError(format!("Session \"{name}\" not found.")));
    }

    if let Some(wait_type) = wait_type {
        let Some(name) = name else {
            return Err("--wait requires a session name.".into());
        };
        let deadline = (timeout > 0.0).then(|| {
            Instant::now() + Duration::from_secs_f64(timeout.min(u32::MAX as f64))
        });
        exit_zero_on_sigint();
        let (_follower, rx) = EventFollower::channel(FollowerOptions::names(vec![name.to_string()]));
        loop {
            let recv = match deadline {
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        Err(RecvTimeoutError::Timeout)
                    } else {
                        rx.recv_timeout(d - now)
                    }
                }
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match recv {
                Ok(event) => {
                    if event.r#type == wait_type {
                        println!("{}", render(&event, json));
                        return Ok(0);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    eprintln!(
                        "Timed out after {}s waiting for \"{wait_type}\" event.",
                        js_number(timeout)
                    );
                    return Ok(1);
                }
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
    }

    exit_zero_on_sigint();
    let options = match (all, name) {
        (false, Some(name)) => FollowerOptions::names(vec![name.to_string()]),
        _ => FollowerOptions::all(),
    };
    let (_follower, rx) = EventFollower::channel(options);
    while let Ok(event) = rx.recv() {
        println!("{}", render(&event, json));
    }
    Ok(0)
}
