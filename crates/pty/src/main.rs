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

/// `process.title = "pty"` (bin/pty:7, cli.ts:80). On Linux `prctl(PR_SET_NAME)`
/// renames the thread the way Node's `process.title` does (`/proc/<pid>/comm`);
/// elsewhere it is a no-op.
fn set_process_title(title: &str) {
    #[cfg(target_os = "linux")]
    {
        let mut bytes = title.as_bytes().to_vec();
        bytes.truncate(15);
        bytes.push(0);
        // SAFETY: PR_SET_NAME reads at most 16 bytes from a NUL-terminated buffer.
        unsafe {
            libc::prctl(libc::PR_SET_NAME, bytes.as_ptr() as libc::c_ulong, 0, 0, 0);
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = title;
    }
}
