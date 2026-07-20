//! Terminal-query stripping, ported from the pty project's
//! `stripTerminalQueries` in `src/server.ts`.
//!
//! Removes terminal *query* escape sequences (DA1/DA2, DSR, XTVERSION, and the
//! OSC 10/11/4 `?` color queries) from a byte/text stream, while leaving normal
//! text and non-query escape sequences (including OSC *set* commands) intact.

use regex::Regex;
use std::sync::OnceLock;

fn patterns() -> &'static [Regex] {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    P.get_or_init(|| {
        [
            r"\x1b\]1[01];\?\x07",   // OSC 10/11 with BEL
            r"\x1b\]1[01];\?\x1b\\", // OSC 10/11 with ST
            r"\x1b\]4;\d+;\?\x07",   // OSC 4 with BEL
            r"\x1b\]4;\d+;\?\x1b\\", // OSC 4 with ST
            r"\x1b\[c",              // DA1
            r"\x1b\[>c",             // DA2
            r"\x1b\[6n",             // DSR cursor position
            r"\x1b\[>0q",            // XTVERSION
        ]
        .iter()
        .map(|p| Regex::new(p).expect("valid query regex"))
        .collect()
    })
}

/// Strip all recognized terminal query sequences from `data`.
pub fn strip_terminal_queries(data: &str) -> String {
    let mut out = data.to_string();
    for re in patterns() {
        out = re.replace_all(&out, "").into_owned();
    }
    out
}
