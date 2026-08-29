//! The `pty` command-line tool — a Rust port of the pty project's CLI, backed
//! by libghostty. v0 surface: run / ls / peek / send / attach / kill / status.
//!
//! Persistent sessions are hosted by a per-session daemon (see [`daemon`])
//! that owns the PTY and a libghostty terminal and serves the wire protocol
//! over a unix socket. Command implementations live in [`cli`]; this file only
//! dispatches on argv.

mod cli;
mod daemon;

use std::process::exit;

use cli::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        cli::help::print_usage();
        exit(0);
    }
    let cmd = args[0].as_str();
    let rest = &args[1..];
    let code = match cmd {
        "__daemon" => cmd_daemon(rest),
        "run" | "spawn" => cmd_run(rest),
        "ls" | "list" => cmd_ls(rest),
        "peek" => cmd_peek(rest),
        "send" => cmd_send(rest),
        "attach" | "a" => cmd_attach(rest),
        "up" => cmd_up(rest),
        "down" => cmd_down(rest),
        "restart" => cmd_restart(rest),
        "rm" | "remove" => cmd_rm(rest),
        "rename" => cmd_rename(rest),
        "kill" => cmd_kill(rest),
        "status" | "stats" => cmd_status(rest),
        "version" | "--version" | "-v" | "-V" => {
            // `<semver>+<short-sha>`, the same shape as node's `pty --version`
            // (`0.12.0+500eab2`); the number and the `-rust` tag differ by
            // project. Stamped by build.rs.
            println!("{}", env!("PTY_VERSION"));
            0
        }
        "completions" => cli::completions::run(rest),
        "help" | "--help" | "-h" => {
            cli::help::print_usage();
            0
        }
        other => {
            eprintln!("pty: unknown command '{other}'. Try `pty help`.");
            1
        }
    };
    exit(code);
}
