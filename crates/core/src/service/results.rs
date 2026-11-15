use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use dbased_commands::{ResultId, TabId, Value};

use crate::CoreError;

/// Largest window a single `FetchRows` returns (docs/PROJECT.md 11.2).
pub(super) const FETCH_WINDOW_LIMIT: u32 = 200;

/// The rows of one result, appended by the tab actor and read by the dispatcher.
pub(super) type ResultRows = Arc<RwLock<Vec<Vec<Value>>>>;

/// How many releases of not-yet-created results are remembered.
const RELEASED_MEMORY: usize = 256;

struct StoredResult {
    tab: TabId,
    // @NOTE: tells apart two actors that served the same `TabId` one after the other.
    generation: u64,
    rows: ResultRows,
}

#[derive(Default)]
struct StoreState {
    results: HashMap<ResultId, StoredResult>,
    // @NOTE: a `ReleaseResult` can arrive before the buffer is created; remembering it keeps
    // that buffer from being created and then living forever.
    released_early: VecDeque<ResultId>,
}

/// Shared exception 2 of docs/PROJECT.md 11.2. Locks are never held across an `.await`.
#[derive(Clone, Default)]
pub(super) struct ResultStore {
    state: Arc<Mutex<StoreState>>,
}

impl ResultStore {
    /// Registers an empty buffer. Called before `ResultStarted` is emitted, so a `FetchRows`
    /// that follows the event always finds it. `None` when the result was already released.
    pub(super) fn create(
        &self,
        tab: TabId,
        generation: u64,
        result: ResultId,
    ) -> Result<Option<ResultRows>, CoreError> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        if let Some(position) = state.released_early.iter().position(|id| *id == result) {
            state.released_early.remove(position);
            return Ok(None);
        }
        let rows = ResultRows::default();
        let stored = StoredResult {
            tab,
            generation,
            rows: rows.clone(),
        };
        state.results.insert(result, stored);
        Ok(Some(rows))
    }

    /// `None` when the result is unknown (released, or never started).
    pub(super) fn window(
        &self,
        result: ResultId,
        start: u64,
        count: u32,
    ) -> Result<Option<Vec<Vec<Value>>>, CoreError> {
        let rows = {
            let state = self.state.lock().map_err(|_| poisoned())?;
            let Some(stored) = state.results.get(&result) else {
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
        let mut state = self.lock();
        if state.results.remove(&result).is_some() {
            return;
        }
        if state.released_early.len() == RELEASED_MEMORY {
            state.released_early.pop_front();
        }
        state.released_early.push_back(result);
    }

    /// Only the actor generation that created the results may release them, so a closing
    /// actor never frees the results of a tab that reused its id.
    pub(super) fn release_tab(&self, tab: TabId, generation: u64) {
        self.lock()
            .results
            .retain(|_, stored| stored.tab != tab || stored.generation != generation);
    }

    // @NOTE: releasing only removes entries, so a poisoned state is still safe to clean.
    fn lock(&self) -> MutexGuard<'_, StoreState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use dbased_commands::RequestId;

    use super::*;

    const TAB: TabId = TabId(1);

    fn result(statement: u32) -> ResultId {
        ResultId {
            request: RequestId(1),
            statement,
        }
    }

    #[test]
    fn a_release_before_the_create_keeps_the_buffer_from_living() {
        let store = ResultStore::default();
        store.release(result(0));
        assert!(store.create(TAB, 1, result(0)).unwrap().is_none());
        assert!(store.window(result(0), 0, 10).unwrap().is_none());
        // The release is consumed: a later create for the same id works again.
        assert!(store.create(TAB, 1, result(0)).unwrap().is_some());
    }

    #[test]
    fn the_early_release_memory_is_bounded() {
        let store = ResultStore::default();
        for statement in 0..=u32::try_from(RELEASED_MEMORY).unwrap() {
            store.release(result(statement));
        }
        assert!(store.create(TAB, 1, result(0)).unwrap().is_some());
    }

    #[test]
    fn an_old_actor_generation_cannot_release_a_reused_tab() {
        let store = ResultStore::default();
        store.create(TAB, 1, result(0)).unwrap();
        store.create(TAB, 2, result(1)).unwrap();
        store.release_tab(TAB, 1);
        assert!(store.window(result(0), 0, 10).unwrap().is_none());
        assert!(store.window(result(1), 0, 10).unwrap().is_some());
        store.release_tab(TAB, 2);
        assert!(store.window(result(1), 0, 10).unwrap().is_none());
    }
}
