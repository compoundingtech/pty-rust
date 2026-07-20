//! Screenshot capture from a libghostty terminal, matching the semantics of
//! the pty project's `src/testing/screenshot.ts`.

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::terminal::Terminal;

/// Captured terminal state at a point in time. Mirrors the TS `Screenshot`.
#[derive(Debug, Clone)]
pub struct Screenshot {
    /// Plain text lines. Trailing whitespace is trimmed per line; trailing
    /// empty lines are removed.
    pub lines: Vec<String>,
    /// All lines joined with `"\n"`. Convenient for `contains()` assertions.
    pub text: String,
    /// Full VT-serialized terminal state, including escape codes. Use to verify
    /// colors, bold, etc.
    pub ansi: String,
}

impl Screenshot {
    /// True if the joined text contains `needle`.
    pub fn contains(&self, needle: &str) -> bool {
        self.text.contains(needle)
    }
}

fn format(term: &Terminal, opts: FormatterOptions) -> String {
    let mut f = Formatter::new(term, opts).expect("formatter new");
    let bytes = f.format_alloc(None).expect("format_alloc");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Capture the current terminal state into a [`Screenshot`].
///
/// Replicates the TS harness exactly: each visible row is right-trimmed, then
/// trailing empty rows are dropped. `ansi` is the VT serialization.
pub fn capture(term: &Terminal) -> Screenshot {
    // Plain grid text, one row per '\n'. We do the trimming ourselves so the
    // result matches xterm's `translateToString(true)` + trailing-empty pop.
    // libghostty's Plain format already keeps written cells (including trailing
    // written spaces, e.g. a bash prompt's "$ ") and drops never-written
    // trailing cells — exactly like xterm's `translateToString(true)` that node
    // uses. So we do NOT right-trim per line (that would strip the written
    // cursor-cell space and diverge from node); we only pop trailing blank rows,
    // matching node's `while (lines[last] === "") lines.pop()`.
    let plain = format(
        term,
        FormatterOptions::new().with_format(Format::Plain),
    );

    let mut lines: Vec<String> = plain.split('\n').map(|l| l.to_string()).collect();

    while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }

    let ansi = format(term, FormatterOptions::new().with_format(Format::Vt));
    let text = lines.join("\n");

    Screenshot { lines, text, ansi }
}
