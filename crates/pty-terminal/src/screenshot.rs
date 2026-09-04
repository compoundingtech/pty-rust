//! Screenshot capture from a libghostty terminal, matching the semantics of
//! the pty project's `src/testing/screenshot.ts`.

use libghostty_vt::terminal::Terminal;

use crate::serialize;

/// Captured terminal state at a point in time. Mirrors the TS `Screenshot`.
#[derive(Debug, Clone)]
pub struct Screenshot {
    /// Plain text lines, every buffer row (history + viewport). Never-written
    /// trailing cells are trimmed per line; trailing whitespace-only lines are
    /// removed.
    pub lines: Vec<String>,
    /// All lines joined with `"\n"`. Convenient for `contains()` assertions.
    pub text: String,
    /// Full VT-serialized terminal state, including escape codes, cursor,
    /// and modes. Use to verify colors, bold, etc.
    pub ansi: String,
}

impl Screenshot {
    /// True if the joined text contains `needle`.
    pub fn contains(&self, needle: &str) -> bool {
        self.text.contains(needle)
    }
}

/// Serialize the terminal for an ATTACH/SCREEN replay: VT sequences that carry
/// not just the visible cells but the terminal *mode* state (mouse tracking,
/// alt-screen, cursor visibility, kitty keyboard) so a reattaching client
/// restores a TUI's full state, not just its glyphs.
///
/// This is the body without Node's mode prefix; the daemon gets the full
/// payload from [`crate::TerminalActor::serialize`].
pub fn serialize_for_replay(term: &Terminal) -> String {
    serialize::vt(term, true, crate::graphics::CellSize::FALLBACK)
}

/// Capture the current terminal state into a [`Screenshot`].
///
/// Replicates the TS testing harness (`src/testing/screenshot.ts:5-24`): every
/// buffer row via `translateToString(true)` (written trailing spaces kept,
/// never-written cells dropped), then trailing rows that are empty or
/// whitespace-only popped.
pub fn capture(term: &Terminal) -> Screenshot {
    let mut lines = serialize::plain_lines_full(term);
    while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        lines.pop();
    }
    let ansi = serialize::vt(term, true, crate::graphics::CellSize::FALLBACK);
    let text = lines.join("\n");
    Screenshot { lines, text, ansi }
}
