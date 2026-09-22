//! Control-plane launch helpers.

use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::Command;

use super::{CliError, CliResult};

/// `pty ctl exec -- <wrapper>` creates the private capability channel inherited
/// by the wrapper and the daemon it launches.
pub fn run(args: &[String]) -> CliResult {
    if args.first().map(String::as_str) != Some("exec") {
        return Err(CliError("Usage: pty ctl exec -- <command> [args...]".to_string()));
    }
    if args.get(1).map(String::as_str) != Some("--") {
        return Err(CliError("Usage: pty ctl exec -- <command> [args...]".to_string()));
    }
    let Some(command) = args.get(2) else {
        return Err(CliError("Usage: pty ctl exec -- <command> [args...]".to_string()));
    };
    let (hold, daemon) = pty_core::capability::socketpair()
        .map_err(|error| CliError(format!("pty ctl exec: {error}")))?;
    let hold_fd = hold.as_raw_fd();
    let daemon_fd = daemon.as_raw_fd();
    let mut child = Command::new(command);
    child.args(&args[3..]);
    child.env("PTY_CAPABILITY_HOLD_FD", hold_fd.to_string());
    child.env("PTY_CAPABILITY_FD", daemon_fd.to_string());
    // socketpair() descriptors are not CLOEXEC today, but make inheritance an
    // explicit contract so this remains true if its implementation changes.
    unsafe {
        child.pre_exec(move || {
            for fd in [hold_fd, daemon_fd] {
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let status = child
        .status()
        .map_err(|error| CliError(format!("pty ctl exec: {error}")))?;
    Ok(status.code().unwrap_or(1))
}
