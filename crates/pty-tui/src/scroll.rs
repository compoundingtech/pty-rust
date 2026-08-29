//! The scroll-region model behind selectable lists, ported from
//! `src/tui/scrollable.ts`: an offset, a selection, a total, and a viewport
//! height, with pure operations that keep the selection visible. Also the
//! grouped list layout (`groupedSelectable`, `builders.ts:207-245`): section
//! headers and spacer rows are visual rows, the selection index counts items
//! only.

/// `ScrollRegion` (`scrollable.ts:3-8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScrollRegion {
    pub offset: usize,
    pub selected: usize,
    pub total: usize,
    pub viewport: usize,
}

impl ScrollRegion {
    /// `createScrollRegion`.
    pub fn new(total: usize, viewport: usize) -> Self {
        ScrollRegion {
            offset: 0,
            selected: 0,
            total,
            viewport,
        }
    }

    /// `updateScrollRegion`: new total (and viewport), selection clamped,
    /// offset adjusted so the selection stays visible.
    pub fn update(self, total: usize, viewport: Option<usize>) -> Self {
        let vh = viewport.unwrap_or(self.viewport);
        let sel = self.selected.min(total.saturating_sub(1));
        ScrollRegion {
            total,
            viewport: vh,
            selected: sel,
            ..self
        }
        .ensure_visible()
    }

    /// `scrollUp`: selection one up, clamped.
    pub fn scroll_up(self) -> Self {
        if self.selected == 0 {
            return self;
        }
        ScrollRegion {
            selected: self.selected - 1,
            ..self
        }
        .ensure_visible()
    }

    /// `scrollDown`: selection one down, clamped.
    pub fn scroll_down(self) -> Self {
        if self.selected + 1 >= self.total {
            return self;
        }
        ScrollRegion {
            selected: self.selected + 1,
            ..self
        }
        .ensure_visible()
    }

    /// `pageUp`: selection up by one viewport.
    pub fn page_up(self) -> Self {
        ScrollRegion {
            selected: self.selected.saturating_sub(self.viewport),
            ..self
        }
        .ensure_visible()
    }

    /// `pageDown`: selection down by one viewport, clamped.
    pub fn page_down(self) -> Self {
        let max = self.total.saturating_sub(1);
        ScrollRegion {
            selected: (self.selected + self.viewport).min(max),
            ..self
        }
        .ensure_visible()
    }

    /// `scrollToTop`.
    pub fn to_top(self) -> Self {
        ScrollRegion {
            selected: 0,
            ..self
        }
        .ensure_visible()
    }

    /// `scrollToBottom`.
    pub fn to_bottom(self) -> Self {
        ScrollRegion {
            selected: self.total.saturating_sub(1),
            ..self
        }
        .ensure_visible()
    }

    /// Move the offset so the selection is inside the viewport, then clamp
    /// the offset to the list (`ensureVisible`, `scrollable.ts:48-58`).
    pub fn ensure_visible(self) -> Self {
        let mut offset = self.offset;
        if self.selected < offset {
            offset = self.selected;
        } else if self.selected >= offset + self.viewport {
            offset = self.selected + 1 - self.viewport;
        }
        offset = offset.min(self.total.saturating_sub(self.viewport));
        ScrollRegion { offset, ..self }
    }

    /// `visibleSlice`: the items in the viewport.
    pub fn visible_slice<'a, T>(&self, items: &'a [T]) -> &'a [T] {
        let start = self.offset.min(items.len());
        let end = (self.offset + self.viewport).min(items.len());
        &items[start..end]
    }
}

/// One group of a grouped list (`SelectableGroup`, `builders.ts:196-199`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group<T> {
    pub title: String,
    pub items: Vec<T>,
}

impl<T> Group<T> {
    pub fn new(title: impl Into<String>, items: Vec<T>) -> Self {
        Group {
            title: title.into(),
            items,
        }
    }
}

/// One visual row of a grouped list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupedRow {
    /// A blank spacer between groups.
    Spacer,
    /// The header of group `group`.
    Header { group: usize },
    /// Item `index` (counting items across groups) of group `group`.
    Item {
        group: usize,
        item: usize,
        index: usize,
        selected: bool,
    },
}

/// The visual rows of a grouped list plus the visual offset that keeps the
/// selected item in view (`groupedSelectable`, `builders.ts:207-245`).
/// `show_headers = false` flattens the list (no headers, no spacers) the way
/// the session manager does without relay hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedLayout {
    pub rows: Vec<GroupedRow>,
    /// First visual row to draw.
    pub offset: usize,
    /// Number of selectable items.
    pub total: usize,
}

impl GroupedLayout {
    pub fn new<T>(region: &ScrollRegion, groups: &[Group<T>], show_headers: bool) -> Self {
        let mut rows = Vec::new();
        let mut index = 0;
        let mut selected_visual_row = 0;
        for (g, group) in groups.iter().enumerate() {
            if show_headers {
                if !rows.is_empty() {
                    rows.push(GroupedRow::Spacer);
                }
                rows.push(GroupedRow::Header { group: g });
            }
            for item in 0..group.items.len() {
                if index == region.selected {
                    selected_visual_row = rows.len();
                }
                rows.push(GroupedRow::Item {
                    group: g,
                    item,
                    index,
                    selected: index == region.selected,
                });
                index += 1;
            }
        }
        let mut offset = 0;
        if selected_visual_row >= region.viewport {
            offset = selected_visual_row + 2 - region.viewport;
        }
        GroupedLayout {
            rows,
            offset,
            total: index,
        }
    }

    /// The rows that fit in `viewport` rows from `offset`.
    pub fn visible(&self, viewport: usize) -> &[GroupedRow] {
        let start = self.offset.min(self.rows.len());
        let end = (self.offset + viewport).min(self.rows.len());
        &self.rows[start..end]
    }

    /// The global item index of the first `Item` row (the `--preselect-new`
    /// walk uses the same counting).
    pub fn find_item(&self, mut pred: impl FnMut(usize, usize) -> bool) -> Option<usize> {
        self.rows.iter().find_map(|r| match r {
            GroupedRow::Item {
                group, item, index, ..
            } if pred(*group, *item) => Some(*index),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: src/tui/scrollable.ts (behaviour pinned by tests/tui-framework.test.ts scrollable cases)
    #[test]
    fn selection_stays_visible() {
        let mut r = ScrollRegion::new(10, 3);
        for _ in 0..5 {
            r = r.scroll_down();
        }
        assert_eq!(r.selected, 5);
        assert_eq!(r.offset, 3);
        r = r.page_up();
        assert_eq!((r.selected, r.offset), (2, 2));
        r = r.to_bottom();
        assert_eq!((r.selected, r.offset), (9, 7));
        r = r.to_top();
        assert_eq!((r.selected, r.offset), (0, 0));
        r = r.page_down();
        assert_eq!((r.selected, r.offset), (3, 1));
        assert_eq!(r.scroll_up().scroll_up().scroll_up().scroll_up().selected, 0);
    }

    #[test]
    fn update_clamps_and_slices() {
        let r = ScrollRegion::new(10, 4).to_bottom();
        let r = r.update(3, None);
        assert_eq!((r.selected, r.offset, r.total), (2, 0, 3));
        let items = ["a", "b", "c"];
        assert_eq!(r.visible_slice(&items), &["a", "b", "c"]);
        let r = ScrollRegion::new(0, 4);
        assert_eq!(r.scroll_down().selected, 0);
        assert_eq!(r.to_bottom().selected, 0);
    }

    /// node: src/tui/builders.ts:207-245
    #[test]
    fn grouped_layout_counts_items_only() {
        let groups = vec![
            Group::new("Local", vec!["a", "b"]),
            Group::new("host", vec!["c"]),
        ];
        let region = ScrollRegion {
            offset: 0,
            selected: 2,
            total: 3,
            viewport: 10,
        };
        let l = GroupedLayout::new(&region, &groups, true);
        assert_eq!(l.total, 3);
        assert_eq!(l.rows.len(), 6);
        assert_eq!(l.rows[0], GroupedRow::Header { group: 0 });
        assert_eq!(l.rows[3], GroupedRow::Spacer);
        assert_eq!(
            l.rows[5],
            GroupedRow::Item {
                group: 1,
                item: 0,
                index: 2,
                selected: true
            }
        );
        assert_eq!(l.offset, 0);
        let flat = GroupedLayout::new(&region, &groups, false);
        assert_eq!(flat.rows.len(), 3);
        // Selected visual row 5 with a viewport of 4 scrolls to 5 - 4 + 2.
        let small = ScrollRegion { viewport: 4, ..region };
        assert_eq!(GroupedLayout::new(&small, &groups, true).offset, 3);
    }
}
