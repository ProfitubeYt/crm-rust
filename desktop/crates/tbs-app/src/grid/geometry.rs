//! Pure layout maths for the grid. Nothing here touches the renderer, so all of
//! the hit testing and range logic is covered by plain unit tests.

use std::ops::Range;

pub const MIN_COLUMN: f32 = 20.0;
pub const MAX_COLUMN: f32 = 6000.0;
/// Half-width of the resize grip around a column's right edge.
pub const GRIP: f32 = 4.0;
pub const BAR: f32 = 10.0;
pub const MIN_THUMB: f32 = 28.0;

/// Column positions: pinned columns stay put, the rest move with the scroll.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Columns {
    pub widths: Vec<f32>,
    pub starts: Vec<f32>,
    pub pinned: usize,
    pub pinned_width: f32,
    pub total: f32,
}

impl Columns {
    pub fn new(widths: Vec<f32>, pinned: usize) -> Self {
        let pinned = pinned.min(widths.len());
        let mut starts = Vec::with_capacity(widths.len());
        let mut x = 0.0;
        let mut pinned_width = 0.0;
        for (i, width) in widths.iter().enumerate() {
            starts.push(x);
            x += width;
            if i + 1 == pinned {
                pinned_width = x;
            }
        }
        Self { widths, starts, pinned, pinned_width, total: x }
    }

    pub fn len(&self) -> usize {
        self.widths.len()
    }

    pub fn is_pinned(&self, col: usize) -> bool {
        col < self.pinned
    }

    /// Where a column's left edge sits inside the viewport.
    pub fn screen_x(&self, col: usize, scroll_x: f32) -> f32 {
        let start = self.starts[col];
        if self.is_pinned(col) { start } else { start - scroll_x }
    }

    /// Where a column's right edge sits inside the viewport: its resize grip,
    /// and the line that parts it from the next column.
    pub fn edge_x(&self, col: usize, scroll_x: f32) -> f32 {
        self.screen_x(col, scroll_x) + self.widths[col]
    }

    /// The scrolling columns that intersect a viewport of `width`.
    pub fn visible(&self, scroll_x: f32, width: f32) -> Range<usize> {
        let left = self.pinned_width + scroll_x;
        let right = scroll_x + width;
        let tail = &self.starts[self.pinned..];
        let first = (tail.partition_point(|s| *s <= left).saturating_sub(1) + self.pinned).min(self.len());
        let mut end = first;
        while end < self.len() && self.starts[end] < right {
            end += 1;
        }
        first..end
    }

    /// The column under `x`, measured from the viewport's left edge.
    pub fn at(&self, x: f32, scroll_x: f32) -> Option<usize> {
        if x < 0.0 {
            return None;
        }
        if x < self.pinned_width {
            return self.starts[..self.pinned].iter().rposition(|s| *s <= x);
        }
        let content = x + scroll_x;
        if content >= self.total {
            return None;
        }
        let found = self.starts.partition_point(|s| *s <= content).checked_sub(1)?;
        (found >= self.pinned).then_some(found)
    }

    /// The column whose right edge is within grip distance of `x`.
    pub fn grip_at(&self, x: f32, scroll_x: f32) -> Option<usize> {
        let near = |col: usize| (self.edge_x(col, scroll_x) - x).abs() <= GRIP;
        if let Some(col) = (0..self.pinned).rev().find(|c| near(*c)) {
            return Some(col);
        }
        if x <= self.pinned_width {
            return None;
        }
        (self.pinned..self.len()).find(|c| near(*c))
    }

    pub fn max_scroll(&self, viewport: f32) -> f32 {
        (self.total - viewport).max(0.0)
    }
}

/// A rectangle of cells, inclusive at both ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRange {
    pub rows: (usize, usize),
    pub cols: (usize, usize),
}

impl CellRange {
    pub fn cell(row: usize, col: usize) -> Self {
        Self { rows: (row, row), cols: (col, col) }
    }

    pub fn between(a: (usize, usize), b: (usize, usize)) -> Self {
        Self { rows: (a.0.min(b.0), a.0.max(b.0)), cols: (a.1.min(b.1), a.1.max(b.1)) }
    }

    pub fn contains(&self, row: usize, col: usize) -> bool {
        (self.rows.0..=self.rows.1).contains(&row) && (self.cols.0..=self.cols.1).contains(&col)
    }
}

/// What the user has selected. `cursor` is the cell keyboard commands act on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub ranges: Vec<CellRange>,
    pub anchor: Option<(usize, usize)>,
    pub cursor: Option<(usize, usize)>,
}

impl Selection {
    pub fn single(row: usize, col: usize) -> Self {
        Self { ranges: vec![CellRange::cell(row, col)], anchor: Some((row, col)), cursor: Some((row, col)) }
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn contains(&self, row: usize, col: usize) -> bool {
        self.ranges.iter().any(|r| r.contains(row, col))
    }

    /// Replaces everything with the block from the anchor to `to`.
    pub fn extend_to(&mut self, to: (usize, usize)) {
        let anchor = self.anchor.unwrap_or(to);
        self.anchor = Some(anchor);
        self.cursor = Some(to);
        self.ranges = vec![CellRange::between(anchor, to)];
    }

    /// Ctrl-click: adds a cell, or takes it out of a one-cell range.
    pub fn toggle(&mut self, row: usize, col: usize) {
        let single = CellRange::cell(row, col);
        if let Some(at) = self.ranges.iter().position(|r| *r == single) {
            self.ranges.remove(at);
        } else {
            self.ranges.push(single);
        }
        self.anchor = Some((row, col));
        self.cursor = Some((row, col));
    }

    /// Every selected row once, ascending.
    pub fn rows(&self) -> Vec<usize> {
        let mut out: Vec<usize> = self.ranges.iter().flat_map(|r| r.rows.0..=r.rows.1).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Every selected cell once, by row and then by column.
    pub fn cells(&self) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = self
            .ranges
            .iter()
            .flat_map(|r| (r.rows.0..=r.rows.1).flat_map(move |row| (r.cols.0..=r.cols.1).map(move |col| (row, col))))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Keeps the selection inside a table that just got smaller.
    pub fn clamp(&mut self, rows: usize, cols: usize) {
        if rows == 0 || cols == 0 {
            *self = Self::default();
            return;
        }
        self.ranges.retain(|r| r.rows.0 < rows && r.cols.0 < cols);
        for range in &mut self.ranges {
            range.rows.1 = range.rows.1.min(rows - 1);
            range.cols.1 = range.cols.1.min(cols - 1);
        }
        let inside = |cell: &(usize, usize)| cell.0 < rows && cell.1 < cols;
        self.anchor = self.anchor.filter(inside);
        self.cursor = self.cursor.filter(inside);
    }
}

/// Rows that intersect a viewport of `height`.
pub fn visible_rows(scroll_y: f32, height: f32, row_height: f32, rows: usize) -> Range<usize> {
    if rows == 0 || row_height <= 0.0 {
        return 0..0;
    }
    let first = (scroll_y / row_height).floor().max(0.0) as usize;
    let last = ((scroll_y + height) / row_height).ceil() as usize;
    first.min(rows)..last.min(rows)
}

pub fn max_scroll_y(rows: usize, row_height: f32, viewport: f32) -> f32 {
    (rows as f32 * row_height - viewport).max(0.0)
}

/// Thumb offset and length on a track, or `None` when everything fits.
pub fn thumb(track: f32, content: f32, viewport: f32, scroll: f32) -> Option<(f32, f32)> {
    if content <= viewport || track <= 0.0 {
        return None;
    }
    let length = (track * viewport / content).clamp(MIN_THUMB.min(track), track);
    let max_scroll = content - viewport;
    let offset = (track - length) * (scroll / max_scroll).clamp(0.0, 1.0);
    Some((offset, length))
}

/// The scroll position a thumb dragged to `offset` stands for.
pub fn scroll_from_thumb(track: f32, content: f32, viewport: f32, offset: f32) -> f32 {
    let Some((_, length)) = thumb(track, content, viewport, 0.0) else { return 0.0 };
    let room = (track - length).max(1.0);
    (offset / room).clamp(0.0, 1.0) * (content - viewport)
}

/// Scroll needed so that `start..start + size` shows inside the viewport.
pub fn reveal(scroll: f32, start: f32, size: f32, viewport: f32) -> f32 {
    if start < scroll {
        start
    } else if start + size > scroll + viewport {
        start + size - viewport
    } else {
        scroll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols() -> Columns {
        Columns::new(vec![50.0, 100.0, 100.0, 100.0, 100.0], 1)
    }

    #[test]
    fn offsets_and_pinned_width() {
        let c = cols();
        assert_eq!(c.starts, vec![0.0, 50.0, 150.0, 250.0, 350.0]);
        assert_eq!(c.pinned_width, 50.0);
        assert_eq!(c.total, 450.0);
    }

    #[test]
    fn pinned_columns_ignore_the_scroll() {
        let c = cols();
        assert_eq!(c.screen_x(0, 80.0), 0.0);
        assert_eq!(c.screen_x(2, 80.0), 70.0);
    }

    #[test]
    fn hit_test_prefers_the_pinned_column_on_top() {
        let c = cols();
        assert_eq!(c.at(10.0, 200.0), Some(0));
        assert_eq!(c.at(60.0, 0.0), Some(1));
        // x=60 with scroll 100 lands on content 160, column 2.
        assert_eq!(c.at(60.0, 100.0), Some(2));
        assert_eq!(c.at(449.0, 0.0), Some(4));
        assert_eq!(c.at(451.0, 0.0), None);
    }

    #[test]
    fn visible_skips_columns_scrolled_under_the_pinned_area() {
        let c = cols();
        assert_eq!(c.visible(0.0, 200.0), 1..3);
        assert_eq!(c.visible(120.0, 200.0), 2..4);
        assert_eq!(c.visible(0.0, 1000.0), 1..5);
    }

    #[test]
    fn visible_handles_no_scrolling_columns() {
        let c = Columns::new(vec![50.0, 50.0], 2);
        assert_eq!(c.visible(0.0, 200.0), 2..2);
    }

    #[test]
    fn grip_finds_the_right_edge() {
        let c = cols();
        assert_eq!(c.grip_at(149.0, 0.0), Some(1));
        assert_eq!(c.grip_at(100.0, 0.0), None);
        assert_eq!(c.grip_at(51.0, 0.0), Some(0));
    }

    #[test]
    fn a_column_ends_where_the_next_one_starts() {
        let c = cols();
        for scroll in [0.0, 37.5, 120.0] {
            for col in 0..c.len() - 1 {
                // Not across the pinned edge: there the scrolling column slides under.
                if c.is_pinned(col) == c.is_pinned(col + 1) {
                    assert_eq!(c.edge_x(col, scroll), c.screen_x(col + 1, scroll), "column {col} at scroll {scroll}");
                }
            }
        }
        assert_eq!(c.edge_x(4, 0.0), c.total);
    }

    #[test]
    fn edges_scroll_with_their_columns_and_pinned_ones_stay() {
        let c = cols();
        assert_eq!(c.edge_x(0, 0.0), 50.0);
        assert_eq!(c.edge_x(0, 80.0), 50.0);
        assert_eq!(c.edge_x(2, 0.0), 250.0);
        assert_eq!(c.edge_x(2, 80.0), 170.0);
    }

    #[test]
    fn an_edge_moves_with_its_column_when_columns_are_resized_or_moved() {
        // The widths as they are while «100» is dragged out to 160, and after two columns swap.
        let resized = Columns::new(vec![50.0, 160.0, 100.0], 1);
        assert_eq!(resized.edge_x(1, 0.0), 210.0);
        assert_eq!(resized.edge_x(2, 0.0), 310.0);
        let moved = Columns::new(vec![50.0, 100.0, 160.0], 1);
        assert_eq!(moved.edge_x(1, 0.0), 150.0);
        assert_eq!(moved.edge_x(2, 0.0), 310.0);
    }

    #[test]
    fn the_grip_is_on_the_line_between_columns() {
        let c = cols();
        for scroll in [0.0, 40.0] {
            for col in c.visible(scroll, 400.0) {
                let line = c.edge_x(col, scroll) - 1.0;
                if line > c.pinned_width {
                    assert_eq!(c.grip_at(line, scroll), Some(col), "column {col} at scroll {scroll}");
                }
            }
        }
        assert_eq!(c.grip_at(c.edge_x(0, 40.0) - 1.0, 40.0), Some(0));
    }

    #[test]
    fn grip_is_not_found_under_the_pinned_area() {
        let c = cols();
        // Column 1's edge sits at x=30 once scrolled by 120, under the pinned column.
        assert_eq!(c.grip_at(30.0, 120.0), None);
    }

    #[test]
    fn rows_in_view() {
        assert_eq!(visible_rows(0.0, 100.0, 28.0, 1000), 0..4);
        assert_eq!(visible_rows(56.0, 56.0, 28.0, 1000), 2..4);
        assert_eq!(visible_rows(27990.0, 100.0, 28.0, 1000), 999..1000);
        assert_eq!(visible_rows(0.0, 100.0, 28.0, 0), 0..0);
    }

    #[test]
    fn scroll_limits() {
        assert_eq!(max_scroll_y(10, 28.0, 1000.0), 0.0);
        assert_eq!(max_scroll_y(100, 10.0, 400.0), 600.0);
    }

    #[test]
    fn thumb_moves_with_scroll_and_inverts() {
        assert_eq!(thumb(100.0, 100.0, 100.0, 0.0), None);
        let (top, length) = thumb(100.0, 1000.0, 100.0, 0.0).unwrap();
        assert_eq!(top, 0.0);
        assert_eq!(length, MIN_THUMB);
        let (bottom, _) = thumb(100.0, 1000.0, 100.0, 900.0).unwrap();
        assert_eq!(bottom, 100.0 - MIN_THUMB);
        assert_eq!(scroll_from_thumb(100.0, 1000.0, 100.0, 100.0 - MIN_THUMB), 900.0);
        assert_eq!(scroll_from_thumb(100.0, 1000.0, 100.0, 0.0), 0.0);
    }

    #[test]
    fn reveal_moves_only_as_far_as_needed() {
        assert_eq!(reveal(100.0, 50.0, 28.0, 200.0), 50.0);
        assert_eq!(reveal(100.0, 150.0, 28.0, 200.0), 100.0);
        assert_eq!(reveal(100.0, 290.0, 28.0, 200.0), 118.0);
    }

    #[test]
    fn selection_block_and_toggle() {
        let mut s = Selection::single(2, 1);
        s.extend_to((4, 3));
        assert!(s.contains(3, 2));
        assert!(!s.contains(5, 2));
        assert_eq!(s.rows(), vec![2, 3, 4]);
        assert_eq!(s.cells().len(), 9);
        s.toggle(10, 0);
        assert!(s.contains(10, 0));
        s.toggle(10, 0);
        assert!(!s.contains(10, 0));
    }

    #[test]
    fn selection_extends_upward() {
        let mut s = Selection::single(5, 5);
        s.extend_to((3, 2));
        assert_eq!(s.ranges, vec![CellRange { rows: (3, 5), cols: (2, 5) }]);
        assert_eq!(s.anchor, Some((5, 5)));
        assert_eq!(s.cursor, Some((3, 2)));
    }

    #[test]
    fn selection_clamps_to_a_smaller_table() {
        let mut s = Selection::single(2, 1);
        s.extend_to((9, 3));
        s.clamp(5, 2);
        assert_eq!(s.ranges, vec![CellRange { rows: (2, 4), cols: (1, 1) }]);
        assert_eq!(s.cursor, None);
        s.clamp(0, 0);
        assert!(s.is_empty());
    }
}
