use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use dbased_commands::{ResultId, TabId, Value};

use crate::CoreError;

/// Largest window a single `FetchRows` returns (docs/PROJECT.md 11.2).
pub(super) const FETCH_WINDOW_LIMIT: u32 = 200;

/// The rows of one result, appended by the tab actor and read by the dispatcher.
pub(super) type ResultRows = Arc<RwLock<Vec<Vec<Value>>>>;

struct StoredResult {
    tab: TabId,
    rows: ResultRows,
}

/// Shared exception 2 of docs/PROJECT.md 11.2. Locks are never held across an `.await`.
#[derive(Clone, Default)]
pub(super) struct ResultStore {
    results: Arc<Mutex<HashMap<ResultId, StoredResult>>>,
}

impl ResultStore {
    /// Registers an empty buffer. Called before `ResultStarted` is emitted, so a `FetchRows`
    /// that follows the event always finds it.
    pub(super) fn create(&self, tab: TabId, result: ResultId) -> Result<ResultRows, CoreError> {
        let rows = ResultRows::default();
        let stored = StoredResult {
            tab,
            rows: rows.clone(),
        };
        self.results
            .lock()
            .map_err(|_| poisoned())?
            .insert(result, stored);
        Ok(rows)
    }

    /// `None` when the result is unknown (released, or never started).
    pub(super) fn window(
        &self,
        result: ResultId,
        start: u64,
        count: u32,
    ) -> Result<Option<Vec<Vec<Value>>>, CoreError> {
        let rows = {
            let results = self.results.lock().map_err(|_| poisoned())?;
            let Some(stored) = results.get(&result) else {
                return Ok(None);
            };
            stored.rows.clone()
        };
        let rows = rows.read().map_err(|_| poisoned())?;
        let first = usize::try_from(start).unwrap_or(usize::MAX).min(rows.len());
        let wanted = usize::try_from(count.min(FETCH_WINDOW_LIMIT)).unwrap_or(0);
        let last = first.saturating_add(wanted).min(rows.len());
        Ok(Some(rows[first..last].to_vec()))
    }

    pub(super) fn release(&self, result: ResultId) {
        self.lock().remove(&result);
    }

    pub(super) fn release_tab(&self, tab: TabId) {
        self.lock().retain(|_, stored| stored.tab != tab);
    }

    // @NOTE: releasing only removes entries, so a poisoned map is still safe to clean.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ResultId, StoredResult>> {
        self.results.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Appends a batch under one write lock and returns the new row count.
pub(super) fn append_rows(
    rows: &ResultRows,
    batch: &mut Vec<Vec<Value>>,
) -> Result<u64, CoreError> {
    let mut rows = rows.write().map_err(|_| poisoned())?;
    rows.append(batch);
    Ok(u64::try_from(rows.len()).unwrap_or(u64::MAX))
}

fn poisoned() -> CoreError {
    CoreError::Internal("a result buffer lock was poisoned".to_owned())
}
