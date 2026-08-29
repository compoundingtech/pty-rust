//! The commands this build documents as absent: `recover`, `evidence` and
//! `test` (docs/parity.md §12). Their `--help` still prints the vendored
//! Node help (the central interceptor handles that before dispatch); the
//! commands themselves say so on stderr and exit 1.

use super::CliResult;

/// `pty <cmd>: not available in this build. See docs/parity.md.` on stderr,
/// exit 1.
pub fn run(cmd: &str) -> CliResult {
    eprintln!("pty {cmd}: not available in this build. See docs/parity.md.");
    Ok(1)
}
