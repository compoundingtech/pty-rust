//! A tiny runnable demo of the libghostty-backed `Session` harness.
//!
//! Spawns a program in a real PTY, feeds its output through libghostty, and
//! renders a live "screenshot" of the emulated screen on a short loop — so you
//! can watch libghostty do the terminal emulation in real time.
//!
//! Run it:
//!
//! ```sh
//! cargo run --example demo                 # default: a scrolling counter
//! cargo run --example demo -- bash         # or drive any program
//! cargo run --example demo -- top -b        # (Ctrl-C to stop)
//! ```

use std::io::Write;
use std::time::Duration;

use pty_testkit::{Session, SpawnOptions};

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "sh".to_string());
    let rest: Vec<String> = args.collect();

    // Default program: a scrolling, timestamped counter so there's motion.
    let (cmd, argv): (String, Vec<String>) = if command == "sh" && rest.is_empty() {
        (
            "sh".to_string(),
            vec![
                "-c".to_string(),
                "i=0; while :; do i=$((i+1)); printf 'tick %d  \\033[32m%s\\033[0m\\n' \
                 \"$i\" \"$(date +%H:%M:%S)\"; sleep 0.4; done"
                    .to_string(),
            ],
        )
    } else {
        (command, rest)
    };

    let argv_ref: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    let rows = 20u16;
    let cols = 72u16;

    let mut session = Session::spawn(
        &cmd,
        &argv_ref,
        SpawnOptions {
            rows: Some(rows),
            cols: Some(cols),
            ..Default::default()
        },
    )
    .expect("spawn");

    let frames = 25;
    let mut out = std::io::stdout();
    for frame in 0..frames {
        std::thread::sleep(Duration::from_millis(400));
        let ss = session.screenshot();

        // Clear our own terminal and draw a framed live view.
        print!("\x1b[2J\x1b[H");
        let title = format!(
            " pty-testkit demo · libghostty · {} {}  (frame {}/{}) ",
            cmd,
            argv_ref.join(" "),
            frame + 1,
            frames
        );
        println!("\x1b[1m┌{:─<width$}┐\x1b[0m", title, width = cols as usize);
        for i in 0..rows as usize {
            let line = ss.lines.get(i).map(String::as_str).unwrap_or("");
            let truncated: String = line.chars().take(cols as usize).collect();
            println!("│{:<width$}│", truncated, width = cols as usize);
        }
        println!("\x1b[1m└{:─<width$}┘\x1b[0m", "", width = cols as usize);
        println!(
            "  cursor/title tracked by libghostty · window title: {:?}",
            session.title()
        );
        let _ = out.flush();
    }

    session.close();
    println!("\n(demo finished — {frames} frames rendered from libghostty)");
}
