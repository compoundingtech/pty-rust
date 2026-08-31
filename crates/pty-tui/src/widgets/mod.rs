//! The 28 Node widgets (`src/tui/widgets/`), state-first: you own the state,
//! rendering and key dispatch are pure. Widgets that ratatui already ships
//! (table, tabs, sparkline, bar chart, gauge, paragraph, list, scrollbar)
//! are thin wrappers keeping Node's state and key maps.
//!
//! | Node widget | module |
//! |---|---|
//! | tree | [`tree`] |
//! | date-picker | [`date_picker`] |
//! | form | [`form`] (+ [`crate::line_edit`]) |
//! | markdown | [`markdown`] |
//! | text-area | [`text_area`] |
//! | virtual-list | [`virtual_list`] |
//! | stream-view | [`stream_view`] |
//! | tabs | [`tabs`] |
//! | confirm | [`confirm`] |
//! | toast | [`toast`] |
//! | command-palette | [`command_palette`] |
//! | command-registry | [`command_registry`] |
//! | table | [`table`] |
//! | help-overlay | [`help_overlay`] |
//! | prompt-bar | [`prompt_bar`] |
//! | toolbar | [`toolbar`] |
//! | sparkline | [`sparkline`] |
//! | bar-chart | [`bar_chart`] |
//! | pty-pane | [`crate::pane`] |
//! | badge | [`badge`] |
//! | breadcrumbs | [`breadcrumbs`] |
//! | progress-bars | [`progress_bars`] |
//! | accordion | [`accordion`] |
//! | action-list-item | [`action_list_item`] |
//! | code-block | [`code_block`] |
//! | message | [`message`] |
//! | select | [`select`] |
//! | canvas | [`canvas`] |
//!
//! Plus the [`panel`] chrome and the centred [`overlay`].

pub mod accordion;
pub mod action_list_item;
pub mod badge;
pub mod bar_chart;
pub mod breadcrumbs;
pub mod canvas;
pub mod code_block;
pub mod command_palette;
pub mod command_registry;
pub mod confirm;
pub mod date_picker;
pub mod form;
pub mod help_overlay;
pub mod markdown;
pub mod message;
pub mod overlay;
pub mod panel;
pub mod progress_bars;
pub mod prompt_bar;
pub mod select;
pub mod sparkline;
pub mod stream_view;
pub mod table;
pub mod tabs;
pub mod text_area;
pub mod toast;
pub mod toolbar;
pub mod tree;
pub mod virtual_list;

pub use accordion::{AccordionOptions, accordion, accordion_header};
pub use action_list_item::{ActionListItem, ActionListItemOptions, action_list_item, action_list_item_spans};
pub use badge::{BadgeOptions, BadgeSpec, BadgeVariant, badge, badge_spec};
pub use bar_chart::{BarChartItem, BarChartOptions, bar_chart, bar_chart_draw, bar_chart_height, bar_chart_widget};
pub use breadcrumbs::{BreadCrumbsOptions, bread_crumbs};
pub use canvas::{Canvas, CanvasCell, DrawContext};
pub use code_block::{CodeBlockOptions, Highlighter, code_block, code_block_paragraph};
pub use command_palette::{
    Command, CommandPalette, CommandPaletteState, PaletteAction, PaletteKeyResult, RankedCommand,
    command_palette_lines, filter_commands, handle_command_palette_key,
};
pub use command_registry::{CommandDisposer, CommandRegistry};
pub use confirm::{ConfirmAction, ConfirmChoice, ConfirmPanel, ConfirmState, handle_confirm_key};
pub use date_picker::{
    DATE_PICKER_HINT, DatePickerPanel, DatePickerState, MONTH_NAMES, calendar_context, calendar_draw,
    calendar_lines, day_of_week, days_in_month, handle_date_picker_key,
};
pub use form::{FormAction, FormState, handle_form_key};
pub use help_overlay::{HelpBinding, HelpPanel, HelpRow, HelpSection, help_rows};
pub use markdown::{Block, InlineSegment, MdRow, markdown_lines, parse_inline, parse_markdown, render_markdown, wrap_line};
pub use message::{MessageOptions, message};
pub use overlay::{Overlay, centered};
pub use panel::Panel;
pub use progress_bars::{BarOptions, bar_loader, bar_progress, bar_string, gauge};
pub use prompt_bar::{
    PromptBar, PromptBarOptions, PromptBarStatus, PromptBarTitle, PromptBarValue, PromptRow, TitleAlign,
    prompt_bar_rows, title_rule,
};
pub use select::{SelectOptions, SelectState, handle_select_key, render_select};
pub use sparkline::{SparklineOptions, sparkline, sparkline_levels, sparkline_string, sparkline_widget};
pub use stream_view::{StreamViewState, handle_stream_key, handle_stream_mouse, render_stream_view};
pub use table::{
    SortDirection, SortValue, TableAction, TableAlign, TableColumn, TableState, column_widths,
    handle_table_key, pad_cell, render_table, sort_rows, table_widget,
};
pub use tabs::{TabDef, TabsState, handle_tabs_key, handle_tabs_mouse, render_tabs, tabs_widget};
pub use text_area::{TextAreaState, apply_text_area_key, render_text_area};
pub use toast::{PushToastOptions, Toast, ToastKind, ToastQueue};
pub use toolbar::{ToolbarFormat, ToolbarItem, ToolbarOptions, toolbar, toolbar_item_for};
pub use tree::{
    TreeAction, TreeKeyResult, TreeNode, TreeRow, TreeState, flatten_tree, handle_tree_key,
    move_selection, render_tree, render_tree_row, select_by_id, toggle_expanded, tree_glyph,
};
pub use virtual_list::{
    VirtualAction, VirtualListState, VirtualWindow, handle_virtual_key, handle_virtual_mouse,
    render_virtual_list,
};
