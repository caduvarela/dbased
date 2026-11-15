use std::{
    collections::{HashMap, HashSet, VecDeque},
    ops::Range,
};

use dbased_commands::Value;

use super::format::{DisplayCell, display_value};

/// Rows per window (docs/PROJECT.md 6.3).
pub const WINDOW_ROWS: u64 = 200;
/// Windows kept; 24 x 200 = 4,800 rows is all the UI ever holds of a result.
pub const MAX_WINDOWS: usize = 24;

/// A window the grid wants; the caller turns it into a `FetchRows` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchRequest {
    pub start: u64,
    pub count: u32,
}

/// What `cell` knows about one position.
#[derive(Debug, PartialEq, Eq)]
pub enum CellLookup<'cache> {
    Ready(&'cache DisplayCell),
    /// The window has not arrived: the grid draws a skeleton.
    Loading,
}

/// Bounded cache of display-ready row windows for one result (docs/PROJECT.md 6.2, 6.3).
/// Windows are only ever pulled: the cache answers `visible_changed` with the windows to
/// fetch, and `apply_window` stores what the Core sent back.
#[derive(Debug, Default)]
pub struct WindowCache {
    total_rows: u64,
    windows: HashMap<u64, Vec<Vec<DisplayCell>>>,
    // Least recently used first.
    order: VecDeque<u64>,
    in_flight: HashSet<u64>,
    visible: Range<u64>,
}

impl WindowCache {
    pub fn total_rows(&self) -> u64 {
        self.total_rows
    }

    pub fn cached_windows(&self) -> usize {
        self.windows.len()
    }

    /// Updates the row count. Growth can make a partial window stale, so the windows
    /// around the last visible range are re-evaluated.
    pub fn set_total_rows(&mut self, total_rows: u64) -> Vec<FetchRequest> {
        if total_rows == self.total_rows {
            return Vec::new();
        }
        self.total_rows = total_rows;
        // @NOTE: nothing is fetched before the grid reports what it shows.
        if self.visible.is_empty() {
            return Vec::new();
        }
        let visible = self.visible.clone();
        self.requests_around(visible)
    }

    /// The grid scrolled: returns the windows around `range` that are missing.
    pub fn visible_changed(&mut self, range: Range<u64>) -> Vec<FetchRequest> {
        self.visible = range.clone();
        for window in self.window_indexes_around(&range) {
            self.touch(window);
        }
        self.requests_around(range)
    }

    pub fn apply_window(&mut self, start: u64, rows: Vec<Vec<Value>>) {
        if !start.is_multiple_of(WINDOW_ROWS) {
            return;
        }
        let window = start / WINDOW_ROWS;
        self.in_flight.remove(&window);
        let cells = rows
            .iter()
            .map(|row| row.iter().map(display_value).collect())
            .collect();
        self.windows.insert(window, cells);
        self.touch(window);
        self.evict();
    }

    pub fn cell(&self, row: u64, column: usize) -> CellLookup<'_> {
        let Some(window) = self.windows.get(&(row / WINDOW_ROWS)) else {
            return CellLookup::Loading;
        };
        let cell = usize::try_from(row % WINDOW_ROWS)
            .ok()
            .and_then(|offset| window.get(offset))
            .and_then(|cells| cells.get(column));
        match cell {
            Some(cell) => CellLookup::Ready(cell),
            None => CellLookup::Loading,
        }
    }

    fn window_indexes_around(&self, range: &Range<u64>) -> Range<u64> {
        let first = (range.start / WINDOW_ROWS).saturating_sub(1);
        let last_row = range.end.saturating_sub(1).max(range.start);
        let last = (last_row / WINDOW_ROWS + 1).min(self.last_window_index());
        first..last.saturating_add(1)
    }

    fn last_window_index(&self) -> u64 {
        self.total_rows.saturating_sub(1) / WINDOW_ROWS
    }

    fn requests_around(&mut self, range: Range<u64>) -> Vec<FetchRequest> {
        if self.total_rows == 0 {
            return Vec::new();
        }
        let mut requests = Vec::new();
        for window in self.window_indexes_around(&range) {
            if !self.needs_fetch(window) {
                continue;
            }
            self.in_flight.insert(window);
            let start = window * WINDOW_ROWS;
            let count = (self.total_rows - start).min(WINDOW_ROWS);
            requests.push(FetchRequest {
                start,
                count: count as u32,
            });
        }
        requests
    }

    fn needs_fetch(&self, window: u64) -> bool {
        let start = window * WINDOW_ROWS;
        if start >= self.total_rows || self.in_flight.contains(&window) {
            return false;
        }
        match self.windows.get(&window) {
            None => true,
            Some(rows) => {
                let held = rows.len() as u64;
                // @NOTE: a partial window is refetched only once the result has grown past it.
                held < WINDOW_ROWS && self.total_rows > start + held
            }
        }
    }

    fn touch(&mut self, window: u64) {
        if !self.windows.contains_key(&window) {
            return;
        }
        self.order.retain(|known| *known != window);
        self.order.push_back(window);
    }

    fn evict(&mut self) {
        while self.windows.len() > MAX_WINDOWS {
            let Some(oldest) = self.order.pop_front() else {
                return;
            };
            self.windows.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(from: u64, count: u64) -> Vec<Vec<Value>> {
        (from..from + count)
            .map(|index| vec![Value::Int(index as i64)])
            .collect()
    }

    fn text_at(cache: &WindowCache, row: u64) -> Option<String> {
        match cache.cell(row, 0) {
            CellLookup::Ready(cell) => Some(cell.text.clone()),
            CellLookup::Loading => None,
        }
    }

    #[test]
    fn an_empty_result_requests_nothing() {
        let mut cache = WindowCache::default();
        assert!(cache.visible_changed(0..20).is_empty());
    }

    #[test]
    fn first_view_requests_the_visible_and_the_next_window() {
        let mut cache = WindowCache::default();
        cache.set_total_rows(1_000);
        let requests = cache.visible_changed(0..30);
        let starts: Vec<u64> = requests.iter().map(|request| request.start).collect();
        assert_eq!(starts, vec![0, 200]);
        assert!(requests.iter().all(|request| request.count == 200));
    }

    #[test]
    fn requests_are_not_repeated_while_in_flight() {
        let mut cache = WindowCache::default();
        cache.set_total_rows(1_000);
        assert!(!cache.visible_changed(0..30).is_empty());
        assert!(cache.visible_changed(0..30).is_empty());
        assert!(cache.visible_changed(5..35).is_empty());
    }

    #[test]
    fn the_last_window_asks_only_for_what_exists() {
        let mut cache = WindowCache::default();
        cache.set_total_rows(250);
        let requests = cache.visible_changed(200..250);
        assert!(requests.contains(&FetchRequest {
            start: 200,
            count: 50
        }));
        assert!(requests.iter().all(|request| request.start < 250));
    }

    #[test]
    fn cells_load_after_their_window_arrives() {
        let mut cache = WindowCache::default();
        cache.set_total_rows(1_000);
        cache.visible_changed(0..30);
        assert_eq!(cache.cell(3, 0), CellLookup::Loading);
        cache.apply_window(0, rows(0, 200));
        assert_eq!(text_at(&cache, 3).as_deref(), Some("3"));
        assert_eq!(text_at(&cache, 199).as_deref(), Some("199"));
        assert_eq!(text_at(&cache, 200), None);
    }

    #[test]
    fn a_missing_column_renders_as_loading_not_a_panic() {
        let mut cache = WindowCache::default();
        cache.set_total_rows(10);
        cache.visible_changed(0..10);
        cache.apply_window(0, rows(0, 10));
        assert_eq!(cache.cell(0, 5), CellLookup::Loading);
    }

    #[test]
    fn memory_stays_bounded_while_scrolling_far() {
        let mut cache = WindowCache::default();
        cache.set_total_rows(5_000_000);
        for step in 0..100_u64 {
            let first = step * 3 * WINDOW_ROWS;
            for request in cache.visible_changed(first..first + 40) {
                cache.apply_window(request.start, rows(request.start, 200));
            }
        }
        assert!(cache.cached_windows() <= MAX_WINDOWS);
        assert_eq!(cache.total_rows(), 5_000_000);
    }

    #[test]
    fn the_windows_in_view_survive_eviction() {
        let mut cache = WindowCache::default();
        cache.set_total_rows(5_000_000);
        for step in 0..60_u64 {
            let first = step * 2 * WINDOW_ROWS;
            for request in cache.visible_changed(first..first + 40) {
                cache.apply_window(request.start, rows(request.start, 200));
            }
        }
        let last_first = 59 * 2 * WINDOW_ROWS;
        assert!(text_at(&cache, last_first + 5).is_some());
    }

    #[test]
    fn a_partial_window_is_refetched_when_the_result_grows() {
        let mut cache = WindowCache::default();
        cache.set_total_rows(50);
        cache.visible_changed(0..30);
        cache.apply_window(0, rows(0, 50));
        assert!(cache.visible_changed(0..30).is_empty());
        let requests = cache.set_total_rows(120);
        assert_eq!(
            requests,
            vec![FetchRequest {
                start: 0,
                count: 120
            }]
        );
    }

    #[test]
    fn a_full_window_is_not_refetched_when_the_result_grows() {
        let mut cache = WindowCache::default();
        cache.set_total_rows(400);
        cache.visible_changed(0..30);
        cache.apply_window(0, rows(0, 200));
        cache.apply_window(200, rows(200, 200));
        assert!(cache.set_total_rows(900).iter().all(|request| request.start >= 400));
    }

    #[test]
    fn an_unaligned_window_is_ignored() {
        let mut cache = WindowCache::default();
        cache.set_total_rows(1_000);
        cache.apply_window(7, rows(7, 10));
        assert_eq!(cache.cached_windows(), 0);
    }
}
