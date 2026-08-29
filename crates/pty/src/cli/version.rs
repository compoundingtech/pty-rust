//! `pty version` / `--version` / `-v` / `-V`: `<semver>+<short-sha>`, the
//! same shape as Node's `pty --version` (`0.12.0+500eab2`); the number and
//! the `-rust` tag differ by project. Stamped by `build.rs`.
//!
//! node: src/version.ts:42-49; src/cli.ts:1626-1632

use super::CliResult;

/// The version string this binary was built with.
pub fn version() -> &'static str {
    env!("PTY_VERSION")
}

/// Print the version to stdout, exit 0.
pub fn run() -> CliResult {
    println!("{}", version());
    Ok(0)
}
