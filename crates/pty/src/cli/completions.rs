//! `pty completions <shell>`: print a shell completion script to stdout.
//!
//! The three scripts are vendored byte for byte from the Node repo
//! (`completions/pty.{fish,bash,zsh}` at the repo root; Node generates them
//! from its command spec and checks them in, so the files are the contract).
//! The usage text is the `completions.txt` help fixture, captured from the
//! Node binary like the other help texts.
//!
//! Exit codes: `--help`/`-h` → usage on stdout, 0; a known shell → its script,
//! 0; no shell → usage on stderr, 2; an unknown shell → an `unknown shell`
//! line, a blank line, then usage on stderr, 2. This is the only command whose
//! usage errors exit 2 (the rest of the CLI exits 1).
//!
//! node: src/completions.ts:710-745 (`usageText`, `cmdCompletions`).

use std::io::Write;

/// The shells with a vendored script, in Node's order.
pub const SHELLS: [&str; 3] = ["fish", "bash", "zsh"];

/// `usage: pty completions <shell>` ... — printed for `--help` and on errors.
pub fn usage() -> &'static str {
    include_str!("../../tests/fixtures/help/completions.txt")
}

/// The vendored completion script for `shell`, if there is one.
pub fn script(shell: &str) -> Option<&'static str> {
    match shell {
        "fish" => Some(include_str!("../../../../completions/pty.fish")),
        "bash" => Some(include_str!("../../../../completions/pty.bash")),
        "zsh" => Some(include_str!("../../../../completions/pty.zsh")),
        _ => None,
    }
}

/// Run `pty completions` with the arguments after the verb; returns the exit
/// code.
pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("--help") | Some("-h") => {
            write_stdout(usage());
            0
        }
        None => {
            write_stderr(usage());
            2
        }
        Some(shell) => match script(shell) {
            Some(text) => {
                write_stdout(text);
                0
            }
            None => {
                // Node: `console.error(`pty completions: unknown shell: ${shell}\n`)`
                // (console.error adds the second newline) then the usage.
                write_stderr(&format!("pty completions: unknown shell: {shell}\n\n"));
                write_stderr(usage());
                2
            }
        },
    }
}

fn write_stdout(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

fn write_stderr(text: &str) {
    let mut err = std::io::stderr().lock();
    let _ = err.write_all(text.as_bytes());
    let _ = err.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shell_has_a_script() {
        for shell in SHELLS {
            let text = script(shell).unwrap_or_else(|| panic!("no script for {shell}"));
            assert!(!text.is_empty(), "{shell} script is empty");
        }
        assert!(script("tcsh").is_none());
        assert!(script("").is_none());
    }

    /// node: tests/completions.test.ts:91-96.
    #[test]
    fn every_script_offers_run_env() {
        assert!(script("fish").unwrap().contains("-l env"));
        assert!(script("bash").unwrap().contains("--env"));
        assert!(script("zsh").unwrap().contains("--env"));
    }

    #[test]
    fn usage_shape() {
        let usage = usage();
        assert!(usage.starts_with("usage: pty completions <shell>\n"));
        for shell in SHELLS {
            assert!(
                usage.contains(&format!("\n  {shell}\n")),
                "usage lacks {shell}"
            );
        }
        assert!(
            usage.ends_with("\n\n"),
            "console.log adds the second newline"
        );
    }
}
