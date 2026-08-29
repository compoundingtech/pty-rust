//! The 28 Node widgets (`src/tui/widgets/`), state-first: you own the state,
//! rendering and key dispatch are pure. Widgets that ratatui already ships
//! (table, tabs, sparkline, bar chart, gauge, paragraph, list, scrollbar)
//! are thin wrappers keeping Node's state and key maps.

pub mod overlay;
pub mod panel;

pub use overlay::{Overlay, centered};
pub use panel::Panel;
