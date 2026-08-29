//! `pty send` — interim port kept from the v0 binary. The Node-exact
//! rewrite (cli.ts:1109-1217, client.ts:221-288) replaces this module.

use pty_core::client;
use pty_core::keys::parse_seq_value;

use super::{CliResult, resolve_ref};

/// `pty send <ref> <text> | --seq VALUE [--seq VALUE ...]`
pub fn run(args: &[String]) -> CliResult {
    if args.is_empty() {
        eprintln!("Usage: pty send <name> <text>");
        return Ok(1);
    }
    let name = resolve_ref(&args[0])?;
    let rest = &args[1..];
    // --paste mode: wrap the payload in bracketed-paste markers so a receiving
    // TUI treats it as one paste event. Position-independent.
    if rest.iter().any(|a| a == "--paste") {
        let mut payload = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            if rest[i] == "--paste" {
                if let Some(v) = rest.get(i + 1) {
                    payload.extend_from_slice(v.as_bytes());
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        let wrapped = pty_core::paste::wrap_bracketed_paste(&payload);
        return Ok(deliver(&name, &[wrapped], client::SendOptions::default()));
    }
    // --seq mode: ordered sequence, each value literal or `key:<name>`, paced
    // by --with-delay (node's default 300 ms; 0 = stream).
    if rest.iter().any(|a| a == "--seq") {
        let mut delay_secs: Option<f64> = None;
        for w in rest.windows(2) {
            if w[0] == "--with-delay" {
                match w[1].parse::<f64>() {
                    Ok(v) if v >= 0.0 => delay_secs = Some(v),
                    _ => {
                        eprintln!("--with-delay requires a non-negative number (seconds).");
                        return Ok(1);
                    }
                }
            }
        }
        let mut items: Vec<Vec<u8>> = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            match rest[i].as_str() {
                "--seq" => {
                    if let Some(v) = rest.get(i + 1) {
                        match parse_seq_value(v) {
                            Ok(bytes) => items.push(bytes.into_bytes()),
                            Err(e) => {
                                eprintln!("{e}");
                                return Ok(1);
                            }
                        }
                    }
                    i += 2;
                }
                "--with-delay" => i += 2,
                _ => i += 1,
            }
        }
        let delay = client::resolve_seq_delay_ms(delay_secs);
        let opts = client::SendOptions { delay_ms: delay, paste: false };
        return Ok(deliver(&name, &items, opts));
    }
    let text = rest.join(" ");
    Ok(deliver(&name, &[text.into_bytes()], client::SendOptions::default()))
}

fn deliver(name: &str, items: &[Vec<u8>], opts: client::SendOptions) -> i32 {
    match client::send(name, items, opts) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
