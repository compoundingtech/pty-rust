//! Rendering differences that cannot be hidden, each pinned here and
//! explained in `docs/decisions/`.

use pty_terminal::{Range, TerminalActor, Wide};

/// docs/decisions/0003-emoji-width.md — Node (xterm-headless, Unicode 6
/// widths) puts `X` at column 1 and the cursor at 5 for `😀X中Y`; libghostty
/// gives the emoji two cells.
#[test]
fn emoji_is_two_cells_wide() {
    let mut a = TerminalActor::new(3, 20, 0);
    a.write("😀X中Y".as_bytes());
    let g = a.snapshot(0);
    assert_eq!(g.rows[0][0].text, "😀");
    assert_eq!(g.rows[0][0].wide, Wide::Wide);
    assert_eq!(g.rows[0][1].wide, Wide::Spacer);
    assert_eq!(g.rows[0][2].text, "X");
    assert_eq!(g.rows[0][3].text, "中");
    assert_eq!(g.rows[0][5].text, "Y");
    assert_eq!(a.cursor(), (6, 0, true), "Node reports cursorX 5 here");
    assert_eq!(a.plain(Range::Viewport), "😀X中Y", "plain text is unaffected");
}
