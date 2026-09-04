//! The `[Y/n]` prompt: write the question to stdout, read one line from
//! stdin. No tty check, exactly like Node's `readline` prompt; a closed
//! stdin reads as an empty answer, which proceeds.
//!
//! node: src/cli.ts:4069-4078 (`ask`), 3922-3926 and 1836-1840 (callers).

#![allow(dead_code)] // callers: the restart / dead-session prompts

use std::io::{BufRead, Write};

/// Print `prompt` (no newline) and return the answer line without its
/// line terminator.
pub fn ask(prompt: &str) -> String {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(prompt.as_bytes());
    let _ = out.flush();
    drop(out);
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    line
}

/// Did the answer decline? Node compares `answer.toLowerCase() === "n"`
/// (the restart prompts) — only a lone `n`/`N` says no; anything else,
/// including an empty line, proceeds.
pub fn declined(answer: &str) -> bool {
    answer.to_lowercase() == "n"
}

#[cfg(test)]
mod tests {
    use super::declined;

    #[test]
    fn only_a_lone_n_declines() {
        assert!(declined("n"));
        assert!(declined("N"));
        assert!(!declined(""));
        assert!(!declined("y"));
        assert!(!declined("no"));
    }
}
