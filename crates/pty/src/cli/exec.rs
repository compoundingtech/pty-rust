//! `pty exec` — not ported yet (cli.ts:1055-1066, `cmdExec` 1865-1939).

use super::CliResult;

/// Placeholder until the socket-verb lane lands.
pub fn run(_args: &[String]) -> CliResult {
    eprintln!("pty exec: not implemented yet");
    Ok(1)
}
