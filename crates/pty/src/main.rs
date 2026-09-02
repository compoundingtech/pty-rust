//! The `pty` command-line tool — a Rust port of the pty project's CLI, backed
//! by libghostty.
//!
//! Persistent sessions are hosted by a per-session daemon (see [`daemon`])
//! that owns the PTY and a libghostty terminal and serves the wire protocol
//! over a unix socket. Command implementations live in [`cli`], one module per
//! verb; [`cli::dispatch`] is Node's `main()`.

mod cli;
mod daemon;
mod interactive;
mod remote;

fn main() {
    set_process_title("pty");
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(cli::dispatch(args));
}

/// `process.title = "pty"` (bin/pty:7, cli.ts:80), the same way the daemon
/// names itself. See `daemon::launch::set_process_title` for why macOS has
/// its own call rather than nothing.
fn set_process_title(title: &str) {
    daemon::set_process_title(title);
}
