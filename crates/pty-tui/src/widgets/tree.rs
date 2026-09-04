//! Tree view (`src/tui/widgets/tree.ts`): expand/collapse with a flat
//! visible-rows shape. Keys: `up`/`down` move (the first press lands on row
//! 0), `right` expands, `left` collapses, `return` toggles a folder or
//! activates a leaf. Glyphs `▸ ` collapsed, `▾ ` expanded, two spaces for a
//! leaf.

use std::collections::BTreeSet;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::input::KeyEvent;
use crate::theme::{Color, Theme};

/// `TreeNode<T>` (`tree.ts:13-18`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode<T> {
    pub id: String,
    pub label: String,
    pub data: T,
    pub children: Vec<TreeNode<T>>,
}

impl<T> TreeNode<T> {
    pub fn leaf(id: impl Into<String>, label: impl Into<String>, data: T) -> Self {
        TreeNode {
            id: id.into(),
            label: label.into(),
            data,
            children: Vec::new(),
        }
    }

    pub fn branch(
        id: impl Into<String>,
        label: impl Into<String>,
        data: T,
        children: Vec<TreeNode<T>>,
    ) -> Self {
        TreeNode {
            id: id.into(),
            label: label.into(),
            data,
            children,
        }
    }
}

/// One visible row (`TreeRow<T>`, `tree.ts:20-25`).
#[derive(Debug)]
pub struct TreeRow<'a, T> {
    pub node: &'a TreeNode<T>,
    pub depth: usize,
    pub has_children: bool,
    pub expanded: bool,
}

impl<T> Clone for TreeRow<'_, T> {
    fn clone(&self) -> Self {
        TreeRow {
            node: self.node,
            depth: self.depth,
            has_children: self.has_children,
            expanded: self.expanded,
        }
    }
}

impl<T> PartialEq for TreeRow<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.node, other.node)
            && self.depth == other.depth
            && self.has_children == other.has_children
            && self.expanded == other.expanded
    }
}

/// `TreeState` (`tree.ts:27-32`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TreeState {
    pub expanded: BTreeSet<String>,
    pub selected_id: Option<String>,
}

impl TreeState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Depth-first walk skipping collapsed children (`flattenTree`,
/// `tree.ts:41-53`).
pub fn flatten_tree<'a, T>(
    roots: &'a [TreeNode<T>],
    expanded: &BTreeSet<String>,
) -> Vec<TreeRow<'a, T>> {
    fn walk<'a, T>(
        nodes: &'a [TreeNode<T>],
        depth: usize,
        expanded: &BTreeSet<String>,
        out: &mut Vec<TreeRow<'a, T>>,
    ) {
        for node in nodes {
            let has_children = !node.children.is_empty();
            let is_expanded = has_children && expanded.contains(&node.id);
            out.push(TreeRow {
                node,
                depth,
                has_children,
                expanded: is_expanded,
            });
            if is_expanded {
                walk(&node.children, depth + 1, expanded, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(roots, 0, expanded, &mut out);
    out
}

/// `toggleExpanded`.
pub fn toggle_expanded(state: &TreeState, id: &str) -> TreeState {
    let mut next = state.clone();
    if !next.expanded.remove(id) {
        next.expanded.insert(id.to_string());
    }
    next
}

/// `selectById`.
pub fn select_by_id(state: &TreeState, id: Option<&str>) -> TreeState {
    TreeState {
        selected_id: id.map(str::to_string),
        ..state.clone()
    }
}

/// Move the selection `delta` rows, clamped; with nothing selected any move
/// lands on row 0 (`moveSelection`, `tree.ts:68-83`).
pub fn move_selection<T>(state: &TreeState, rows: &[TreeRow<'_, T>], delta: i64) -> TreeState {
    if rows.is_empty() {
        return state.clone();
    }
    let idx = state
        .selected_id
        .as_ref()
        .and_then(|id| rows.iter().position(|r| &r.node.id == id));
    let Some(idx) = idx else {
        return select_by_id(state, Some(&rows[0].node.id));
    };
    let next = (idx as i64 + delta).clamp(0, rows.len() as i64 - 1) as usize;
    if next == idx {
        return state.clone();
    }
    select_by_id(state, Some(&rows[next].node.id))
}

/// What a key did (`HandleKeyResult.action`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeAction {
    Moved,
    Expanded,
    Collapsed,
    Activated,
    None,
}

/// `handleTreeKey` result.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeKeyResult<'a, T> {
    pub state: TreeState,
    pub action: TreeAction,
    pub row: Option<TreeRow<'a, T>>,
}

/// The default key map (`handleTreeKey`, `tree.ts:98-139`).
pub fn handle_tree_key<'a, T>(
    state: &TreeState,
    rows: &[TreeRow<'a, T>],
    key: &KeyEvent,
) -> TreeKeyResult<'a, T> {
    let selected = state
        .selected_id
        .as_ref()
        .and_then(|id| rows.iter().find(|r| &r.node.id == id))
        .cloned();
    let none = |state: TreeState, row: Option<TreeRow<'a, T>>| TreeKeyResult {
        state,
        action: TreeAction::None,
        row,
    };
    match key.name.as_str() {
        "up" => {
            return TreeKeyResult {
                state: move_selection(state, rows, -1),
                action: TreeAction::Moved,
                row: None,
            };
        }
        "down" => {
            return TreeKeyResult {
                state: move_selection(state, rows, 1),
                action: TreeAction::Moved,
                row: None,
            };
        }
        _ => {}
    }
    let Some(sel) = selected else {
        return none(state.clone(), None);
    };
    match key.name.as_str() {
        "right" => {
            if sel.has_children && !sel.expanded {
                return TreeKeyResult {
                    state: toggle_expanded(state, &sel.node.id),
                    action: TreeAction::Expanded,
                    row: Some(sel),
                };
            }
            none(state.clone(), Some(sel))
        }
        "left" => {
            if sel.has_children && sel.expanded {
                return TreeKeyResult {
                    state: toggle_expanded(state, &sel.node.id),
                    action: TreeAction::Collapsed,
                    row: Some(sel),
                };
            }
            none(state.clone(), Some(sel))
        }
        "return" => {
            if sel.has_children {
                let action = if sel.expanded {
                    TreeAction::Collapsed
                } else {
                    TreeAction::Expanded
                };
                return TreeKeyResult {
                    state: toggle_expanded(state, &sel.node.id),
                    action,
                    row: Some(sel),
                };
            }
            TreeKeyResult {
                state: state.clone(),
                action: TreeAction::Activated,
                row: Some(sel),
            }
        }
        _ => none(state.clone(), None),
    }
}

/// `▸ ` / `▾ ` / two spaces (`treeGlyph`, `tree.ts:143-145`).
pub fn tree_glyph<T>(row: &TreeRow<'_, T>) -> &'static str {
    if !row.has_children {
        "  "
    } else if row.expanded {
        "\u{25be} "
    } else {
        "\u{25b8} "
    }
}

/// A row as a line: indent, glyph, label; the selected row in bold accent.
pub fn render_tree_row<T>(theme: &Theme, row: &TreeRow<'_, T>, selected: bool) -> Line<'static> {
    let indent = "  ".repeat(row.depth);
    let style = if selected {
        Style::default()
            .fg(theme.color(Color::Accent))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.color(Color::Primary))
    };
    Line::from(vec![
        Span::raw(indent),
        Span::styled(tree_glyph(row), style),
        Span::styled(row.node.label.clone(), style),
    ])
}

/// Every visible row as lines.
pub fn render_tree<T>(theme: &Theme, state: &TreeState, roots: &[TreeNode<T>]) -> Vec<Line<'static>> {
    flatten_tree(roots, &state.expanded)
        .iter()
        .map(|r| render_tree_row(theme, r, state.selected_id.as_deref() == Some(&r.node.id)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<TreeNode<()>> {
        vec![
            TreeNode::branch(
                "root/a",
                "a",
                (),
                vec![
                    TreeNode::leaf("root/a/1", "a1", ()),
                    TreeNode::branch("root/a/2", "a2", (), vec![TreeNode::leaf("root/a/2/x", "x", ())]),
                ],
            ),
            TreeNode::leaf("root/b", "b", ()),
        ]
    }

    fn ids<'a, T>(rows: &[TreeRow<'a, T>]) -> Vec<&'a str> {
        rows.iter().map(|r| r.node.id.as_str()).collect()
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    /// node: tests/widgets-tree.test.ts:23-49
    #[test]
    fn flatten() {
        let s = sample();
        let rows = flatten_tree(&s, &set(&[]));
        assert_eq!(ids(&rows), vec!["root/a", "root/b"]);
        assert_eq!(rows[0].depth, 0);
        assert!(rows[0].has_children && !rows[0].expanded);
        assert!(!rows[1].has_children);
        let rows = flatten_tree(&s, &set(&["root/a"]));
        assert_eq!(ids(&rows), vec!["root/a", "root/a/1", "root/a/2", "root/b"]);
        assert_eq!(rows[1].depth, 1);
        assert!(!rows[2].expanded);
        let rows = flatten_tree(&s, &set(&["root/a", "root/a/2"]));
        assert_eq!(ids(&rows), vec!["root/a", "root/a/1", "root/a/2", "root/a/2/x", "root/b"]);
        assert_eq!(rows[3].depth, 2);
    }

    /// node: tests/widgets-tree.test.ts:51-59
    #[test]
    fn toggle() {
        let s1 = toggle_expanded(&TreeState::new(), "foo");
        assert!(s1.expanded.contains("foo"));
        let s2 = toggle_expanded(&s1, "foo");
        assert!(!s2.expanded.contains("foo"));
    }

    /// node: tests/widgets-tree.test.ts:61-74
    #[test]
    fn move_selection_clamps() {
        let s = sample();
        let rows = flatten_tree(&s, &set(&[]));
        let s0 = TreeState {
            expanded: set(&[]),
            selected_id: Some("root/a".into()),
        };
        assert_eq!(move_selection(&s0, &rows, -5).selected_id.as_deref(), Some("root/a"));
        assert_eq!(move_selection(&s0, &rows, 5).selected_id.as_deref(), Some("root/b"));
        assert_eq!(
            move_selection(&TreeState::new(), &rows, 1).selected_id.as_deref(),
            Some("root/a")
        );
    }

    /// node: tests/widgets-tree.test.ts:76-105
    #[test]
    fn key_handling() {
        let s = sample();
        let s0 = TreeState {
            expanded: set(&[]),
            selected_id: Some("root/a".into()),
        };
        let rows = flatten_tree(&s, &s0.expanded);
        let r = handle_tree_key(&s0, &rows, &KeyEvent::named("down"));
        assert_eq!(r.state.selected_id.as_deref(), Some("root/b"));
        assert_eq!(r.action, TreeAction::Moved);
        let r = handle_tree_key(&s0, &rows, &KeyEvent::named("right"));
        assert!(r.state.expanded.contains("root/a"));
        assert_eq!(r.action, TreeAction::Expanded);
        let s1 = TreeState {
            expanded: set(&["root/a"]),
            selected_id: Some("root/a".into()),
        };
        let rows = flatten_tree(&s, &s1.expanded);
        let r = handle_tree_key(&s1, &rows, &KeyEvent::named("left"));
        assert!(!r.state.expanded.contains("root/a"));
        assert_eq!(r.action, TreeAction::Collapsed);
        let s2 = TreeState {
            expanded: set(&["root/a"]),
            selected_id: Some("root/a/1".into()),
        };
        let r = handle_tree_key(&s2, &rows, &KeyEvent::named("return"));
        assert_eq!(r.action, TreeAction::Activated);
        assert_eq!(r.row.unwrap().node.id, "root/a/1");
        assert_eq!(tree_glyph(&rows[0]), "▾ ");
        assert_eq!(tree_glyph(&rows[1]), "  ");
        assert_eq!(tree_glyph(&rows[2]), "▸ ");
        let lines = render_tree(&crate::theme::COOL_BLUE, &s2, &s);
        assert_eq!(lines[1].to_string(), "    a1");
    }
}
