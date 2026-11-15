//! One actor per open tab. It owns the tab's session, opened lazily on the first run, and runs
//! one execution at a time (docs/PROJECT.md 9.1, 11.2).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_channel::Sender;
use dbased_commands::{
    ColumnMeta, ConnectionId, DatabaseError, DatabaseKind, Event, ExecuteTarget, ExecutionStatus,
    RequestId, ResultId, ResultOutcome, ResultStatus, SessionResetReason, SessionState,
    StatementSpan, TabId, Value,
};
use dbased_sql::{Dialect, Span, split_statements, statement_at};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::error_info::{database_error, error_info};
use super::results::{ResultRows, ResultStore, append_rows};
use crate::{
    CloseReason, CoreError, Driver, RowSink, RowSinkMessage, Session, SessionHealth,
    StatementOutcome, StatementStatus, UserSql,
};

const SINK_CAPACITY: usize = 1_024;
const BATCH_LIMIT: usize = 1_024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Shared exception 1 of docs/PROJECT.md 11.2: the request that is running (or about to run) on
/// a tab, with its cancel token. `Cancel` is served from it because the actor is busy inside
/// `execute` and a queued message would arrive too late.
#[derive(Clone, Default)]
pub(super) struct RunningSlot {
    running: Arc<Mutex<Option<(RequestId, CancellationToken)>>>,
}

impl RunningSlot {
    // @NOTE: the slot is a plain `Option`, so a poisoned lock holds no broken invariant.
    fn lock(&self) -> MutexGuard<'_, Option<(RequestId, CancellationToken)>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Claimed by the dispatcher when the execute is accepted, so a second one is rejected
    /// even before the actor picks the first up.
    pub(super) fn begin(
        &self,
        request: RequestId,
        token: CancellationToken,
    ) -> Result<(), CoreError> {
        let mut running = self.lock();
        if running.is_some() {
            return Err(CoreError::TabBusy);
        }
        *running = Some((request, token));
        Ok(())
    }

    pub(super) fn finish(&self) {
        *self.lock() = None;
    }

    /// A no-op when `request` is not the running one.
    pub(super) fn cancel(&self, request: RequestId) {
        if let Some((running, token)) = &*self.lock()
            && *running == request
        {
            token.cancel();
        }
    }

    /// Empties the slot and returns the request it held.
    pub(super) fn take(&self) -> Option<RequestId> {
        self.lock().take().map(|(request, _)| request)
    }

    pub(super) fn cancel_current(&self) {
        if let Some((_, token)) = &*self.lock() {
            token.cancel();
        }
    }
}

/// An accepted execute, waiting for its connection or for the tab actor.
pub(super) struct PendingExecute {
    pub(super) request: RequestId,
    pub(super) tab: TabId,
    pub(super) script: String,
    /// Planned by the dispatcher before the slot is claimed (11.2 step 1).
    pub(super) plan: Vec<StatementSpan>,
    pub(super) token: CancellationToken,
}

impl PendingExecute {
    /// A request cancelled before its first statement ran ends like one cancelled at
    /// statement 0: `ExecutionStarted`, then `ExecutionFinished(Cancelled)` with no results.
    // @NOTE: the ordering guarantee of 11.1 stays intact and the UI needs no extra case.
    pub(super) fn cancelled_events(&self) -> [Event; 2] {
        let statement = self.plan.first().map_or(0, |span| span.index);
        [
            Event::ExecutionStarted {
                request: self.request,
                tab: self.tab,
                statements: self.plan.clone(),
            },
            Event::ExecutionFinished {
                request: self.request,
                status: ExecutionStatus::Cancelled { statement },
            },
        ]
    }
}

pub(super) enum TabMessage {
    Execute {
        job: PendingExecute,
        driver: Arc<dyn Driver>,
    },
    /// Closes the session and keeps the tab. The dispatcher cancels a running statement first.
    ReleaseSession { done: oneshot::Sender<()> },
    /// Same, then releases the tab's results, emits `TabClosed` and ends the actor.
    Close { done: oneshot::Sender<()> },
}

pub(super) struct TabHandle {
    pub(super) connection: ConnectionId,
    pub(super) messages: mpsc::UnboundedSender<TabMessage>,
    pub(super) running: RunningSlot,
    /// Tells this actor apart from an earlier one that served the same `TabId`.
    pub(super) generation: u64,
}

/// The handle and the actor's run future; the dispatcher owns the task so it sees a panic.
pub(super) fn new_tab(
    tab: TabId,
    generation: u64,
    connection: ConnectionId,
    events: Sender<Event>,
    results: ResultStore,
) -> (TabHandle, impl Future<Output = ()> + Send + 'static) {
    let (messages, mailbox) = mpsc::unbounded_channel();
    let running = RunningSlot::default();
    let actor = TabActor {
        tab,
        generation,
        events,
        results,
        running: running.clone(),
        session: None,
        state: SessionState::NotOpen,
    };
    let handle = TabHandle {
        connection,
        messages,
        running,
        generation,
    };
    (handle, actor.run(mailbox))
}

pub(super) fn dialect_of(kind: DatabaseKind) -> Dialect {
    match kind {
        DatabaseKind::Postgres => Dialect::Postgres,
        DatabaseKind::MySql => Dialect::MySql,
    }
}

struct TabActor {
    tab: TabId,
    generation: u64,
    events: Sender<Event>,
    results: ResultStore,
    running: RunningSlot,
    session: Option<Box<dyn Session>>,
    state: SessionState,
}

impl TabActor {
    async fn run(mut self, mut mailbox: mpsc::UnboundedReceiver<TabMessage>) {
        while let Some(message) = mailbox.recv().await {
            match message {
                TabMessage::Execute { job, driver } => self.execute(job, driver).await,
                TabMessage::ReleaseSession { done } => {
                    self.release_session().await;
                    done.send(()).ok();
                }
                TabMessage::Close { done } => {
                    self.release_session().await;
                    self.results.release_tab(self.tab, self.generation);
                    emit(&self.events, Event::TabClosed { tab: self.tab }).await;
                    done.send(()).ok();
                    return;
                }
            }
        }
        // @NOTE: the dispatcher is gone without a Close; the session is still closed.
        self.release_session().await;
        self.results.release_tab(self.tab, self.generation);
    }

    fn owner(&self) -> (TabId, u64) {
        (self.tab, self.generation)
    }

    async fn set_state(&mut self, state: SessionState) {
        if self.state == state {
            return;
        }
        self.state = state;
        emit(
            &self.events,
            Event::SessionStateChanged {
                tab: self.tab,
                state,
            },
        )
        .await;
    }

    /// Always calls `close()`: a dropped session would only be cancelled lazily (3.2.4).
    async fn release_session(&mut self) {
        if let Some(session) = self.session.take() {
            session.close().await;
        }
        self.set_state(SessionState::NotOpen).await;
    }

    async fn execute(&mut self, job: PendingExecute, driver: Arc<dyn Driver>) {
        let request = job.request;
        let terminal = match self.prepare(&job, driver.as_ref()).await {
            Ok(()) => {
                let status = self.run_plan(&job).await;
                Event::ExecutionFinished { request, status }
            }
            Err(error) => Event::RequestFailed {
                request,
                error: error_info(&error),
            },
        };
        // @NOTE: cleared before the terminal event so the UI can run again as soon as it sees it.
        self.running.finish();
        emit(&self.events, terminal).await;
    }

    /// Everything that can still end the request with `RequestFailed`.
    async fn prepare(
        &mut self,
        job: &PendingExecute,
        driver: &dyn Driver,
    ) -> Result<(), CoreError> {
        // @NOTE: a cancelled request opens no session (it may sit behind a ReleaseSession);
        // `run_plan` then reports it cancelled before statement 0.
        if !job.token.is_cancelled() {
            self.ensure_session(driver).await?;
        }
        emit(
            &self.events,
            Event::ExecutionStarted {
                request: job.request,
                tab: self.tab,
                statements: job.plan.clone(),
            },
        )
        .await;
        Ok(())
    }

    async fn ensure_session(&mut self, driver: &dyn Driver) -> Result<(), CoreError> {
        if self.session.is_some() {
            return Ok(());
        }
        self.session = Some(driver.open_session().await?);
        self.set_state(SessionState::Open).await;
        Ok(())
    }

    async fn run_plan(&mut self, job: &PendingExecute) -> ExecutionStatus {
        let last = job.plan.len().saturating_sub(1);
        for (position, span) in job.plan.iter().copied().enumerate() {
            if job.token.is_cancelled() {
                return ExecutionStatus::Cancelled {
                    statement: span.index,
                };
            }
            // @NOTE: the session is opened once, in `prepare`. A session closed mid-script ends
            // the script, so the next statement never runs on a fresh one.
            let outcome = self.run_statement(job, span, position < last).await;
            match outcome {
                Ok(None) => {}
                Ok(Some(status)) => return status,
                Err(error) => return aborted(span.index, &error),
            }
        }
        ExecutionStatus::Completed
    }

    /// `Ok(Some(status))` stops the script. `Err` is a session-level failure.
    async fn run_statement(
        &mut self,
        job: &PendingExecute,
        span: StatementSpan,
        more_follow: bool,
    ) -> Result<Option<ExecutionStatus>, CoreError> {
        let result = ResultId {
            request: job.request,
            statement: span.index,
        };
        let text = job.script.get(span.start..span.end).ok_or_else(|| {
            CoreError::Internal("a planned statement is outside the script".to_owned())
        })?;
        let mut session = self
            .session
            .take()
            .ok_or_else(|| CoreError::Internal("the tab has no session".to_owned()))?;

        let (sink, mut receiver) = RowSink::channel(SINK_CAPACITY);
        let mut collector = Collector::new(
            self.events.clone(),
            self.results.clone(),
            self.owner(),
            result,
        );
        // @NOTE: join, never select: the execute future runs to its terminal result (3.2).
        let (executed, ()) = tokio::join!(
            session.execute(UserSql::new(text), sink, job.token.clone()),
            collector.run(&mut receiver),
        );

        match executed {
            Ok(outcome) => {
                collector.ensure_started().await;
                self.finish_statement(session, collector, span, outcome, more_follow)
                    .await
            }
            Err(error) => {
                self.abort_statement(session, collector, &error).await;
                Err(error)
            }
        }
    }

    async fn finish_statement(
        &mut self,
        session: Box<dyn Session>,
        collector: Collector,
        span: StatementSpan,
        outcome: StatementOutcome,
        more_follow: bool,
    ) -> Result<Option<ExecutionStatus>, CoreError> {
        if outcome.extra_statements_discarded {
            // @NOTE: only the splitter builds statements, so this is a splitter bug.
            let error = CoreError::Internal("a statement carried more than one statement".into());
            emit(
                &self.events,
                Event::Error {
                    error: error_info(&error),
                },
            )
            .await;
        }
        collector.report_failure().await;
        emit(
            &self.events,
            Event::ResultFinished {
                result: collector.result,
                outcome: result_outcome(&outcome),
            },
        )
        .await;

        let mut closed = None;
        match outcome.session {
            SessionHealth::Healthy => {
                self.session = Some(session);
                self.set_state(SessionState::Open).await;
            }
            SessionHealth::TransactionAborted => {
                self.session = Some(session);
                self.set_state(SessionState::TransactionAborted).await;
            }
            SessionHealth::Closed(reason) => {
                session.close().await;
                self.set_state(SessionState::Reset {
                    reason: reset_reason(reason),
                })
                .await;
                closed = Some(reason);
            }
        }

        let stop = match outcome.status {
            StatementStatus::Completed | StatementStatus::Truncated => closed
                .filter(|_| more_follow)
                .map(|reason| aborted(span.index, &CoreError::SessionClosed(reason))),
            StatementStatus::Failed => Some(ExecutionStatus::StoppedOnError {
                statement: span.index,
            }),
            StatementStatus::Cancelled => Some(ExecutionStatus::Cancelled {
                statement: span.index,
            }),
        };
        Ok(stop)
    }

    async fn abort_statement(
        &mut self,
        session: Box<dyn Session>,
        mut collector: Collector,
        error: &CoreError,
    ) {
        // @NOTE: a failed statement still gets its ResultStarted and a ResultFinished that
        // carries the error (11.1), exactly like a statement the server rejected.
        collector.ensure_started().await;
        collector.report_failure().await;
        let outcome = ResultOutcome {
            status: ResultStatus::Failed,
            rows_received: collector.buffered,
            rows_affected: None,
            partial: true,
            error: Some(failure_detail(error)),
            cancel_error: None,
        };
        emit(
            &self.events,
            Event::ResultFinished {
                result: collector.result,
                outcome,
            },
        )
        .await;
        // @NOTE: after a session-level error the protocol state is unknown, so it is not reused.
        session.close().await;
        let reason = match error {
            CoreError::SessionClosed(reason) => reset_reason(*reason),
            _ => SessionResetReason::ConnectionError,
        };
        self.set_state(SessionState::Reset { reason }).await;
    }
}

/// The result outcome only has a database-shaped error; a session-level failure has no SQLSTATE.
fn failure_detail(error: &CoreError) -> DatabaseError {
    match error {
        CoreError::Database(info) => database_error(info),
        other => DatabaseError {
            sqlstate: String::new(),
            message: other.to_string(),
            detail: None,
            hint: None,
            position: None,
        },
    }
}

fn aborted(statement: u32, error: &CoreError) -> ExecutionStatus {
    ExecutionStatus::Aborted {
        statement,
        error: error_info(error),
    }
}

fn reset_reason(reason: CloseReason) -> SessionResetReason {
    match reason {
        CloseReason::DrainTimeout => SessionResetReason::DrainTimeout,
        CloseReason::HealthCheckFailed => SessionResetReason::HealthCheckFailed,
        CloseReason::FutureDropped => SessionResetReason::FutureDropped,
        CloseReason::ConnectionError | CloseReason::ClosedByCaller => {
            SessionResetReason::ConnectionError
        }
    }
}

fn result_outcome(outcome: &StatementOutcome) -> ResultOutcome {
    let status = match outcome.status {
        StatementStatus::Completed => ResultStatus::Completed,
        StatementStatus::Failed => ResultStatus::Failed,
        StatementStatus::Cancelled => ResultStatus::Cancelled,
        StatementStatus::Truncated => ResultStatus::Truncated,
    };
    ResultOutcome {
        status,
        rows_received: outcome.rows_received,
        rows_affected: outcome.rows_affected,
        partial: outcome.partial,
        error: outcome.server_error.as_ref().map(database_error),
        cancel_error: outcome.cancel_error.clone(),
    }
}

/// Which statements one `Execute` runs, as spans into the script. The raw script never reaches
/// a driver: only one `Statement` text at a time becomes `UserSql` (docs/PROJECT.md 11.2).
pub(super) fn plan_statements(
    script: &str,
    target: ExecuteTarget,
    dialect: Dialect,
) -> Result<Vec<StatementSpan>, CoreError> {
    let spans: Vec<Span> = match target {
        ExecuteTarget::All => split_statements(script, dialect)
            .iter()
            .map(|statement| statement.span)
            .collect(),
        ExecuteTarget::Current { cursor } => {
            let statements = split_statements(script, dialect);
            statement_at(&statements, cursor)
                .map(|statement| vec![statement.span])
                .unwrap_or_default()
        }
        ExecuteTarget::Selection { start, end } => {
            let selected = script.get(start..end).ok_or_else(|| {
                CoreError::Configuration("the selection is not a valid range of the script".into())
            })?;
            split_statements(selected, dialect)
                .iter()
                .map(|statement| Span {
                    start: statement.span.start + start,
                    end: statement.span.end + start,
                })
                .collect()
        }
    };
    if spans.is_empty() {
        return Err(CoreError::EmptyScript);
    }
    let planned = spans
        .into_iter()
        .zip(0u32..)
        .map(|(span, index)| StatementSpan {
            index,
            start: span.start,
            end: span.end,
        })
        .collect();
    Ok(planned)
}

/// Drains the row sink into the result buffer and reports progress.
struct Collector {
    events: Sender<Event>,
    results: ResultStore,
    owner: (TabId, u64),
    result: ResultId,
    rows: Option<ResultRows>,
    started: bool,
    buffered: u64,
    last_progress: Option<Instant>,
    failure: Option<CoreError>,
}

impl Collector {
    fn new(
        events: Sender<Event>,
        results: ResultStore,
        owner: (TabId, u64),
        result: ResultId,
    ) -> Self {
        Self {
            events,
            results,
            owner,
            result,
            rows: None,
            started: false,
            buffered: 0,
            last_progress: None,
            failure: None,
        }
    }

    /// Reads until the outcome or until the sink closes. It keeps reading even after a buffer
    /// failure so the driver is never blocked on a full sink.
    async fn run(&mut self, receiver: &mut mpsc::Receiver<RowSinkMessage>) {
        let mut messages = Vec::with_capacity(BATCH_LIMIT);
        loop {
            messages.clear();
            if receiver.recv_many(&mut messages, BATCH_LIMIT).await == 0 {
                return;
            }
            let mut batch = Vec::new();
            let mut finished = false;
            for message in messages.drain(..) {
                match message {
                    RowSinkMessage::Columns(columns) => {
                        self.append(&mut batch).await;
                        self.start(columns).await;
                    }
                    RowSinkMessage::Row(row) => batch.push(row),
                    RowSinkMessage::Outcome(_) => {
                        finished = true;
                        break;
                    }
                }
            }
            self.append(&mut batch).await;
            if finished {
                return;
            }
        }
    }

    async fn start(&mut self, columns: Vec<ColumnMeta>) {
        if self.started {
            return;
        }
        self.started = true;
        // @NOTE: the buffer exists before the event, so a FetchRows after it always finds it.
        let (tab, generation) = self.owner;
        match self.results.create(tab, generation, self.result) {
            // @NOTE: `None` means the UI already released this result; rows are then dropped.
            Ok(rows) => self.rows = rows,
            Err(error) => self.failure = Some(error),
        }
        self.events
            .send(Event::ResultStarted {
                result: self.result,
                columns,
            })
            .await
            .ok();
    }

    async fn report_failure(&self) {
        let Some(error) = &self.failure else { return };
        let event = Event::Error {
            error: error_info(error),
        };
        emit(&self.events, event).await;
    }

    /// A failed statement still gets a `ResultStarted`, with empty columns (11.1).
    async fn ensure_started(&mut self) {
        self.start(Vec::new()).await;
    }

    async fn append(&mut self, batch: &mut Vec<Vec<Value>>) {
        if batch.is_empty() {
            return;
        }
        self.ensure_started().await;
        let Some(rows) = &self.rows else {
            batch.clear();
            return;
        };
        match append_rows(rows, batch) {
            Ok(buffered) => self.buffered = buffered,
            Err(error) => {
                self.failure = Some(error);
                batch.clear();
                return;
            }
        }
        self.report_progress();
    }

    /// First rows at once, then at most every 100 ms. Dropped when the event channel is full:
    /// the next progress or `ResultFinished` carries the truth.
    fn report_progress(&mut self) {
        let now = Instant::now();
        let due = self
            .last_progress
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        if !due {
            return;
        }
        self.last_progress = Some(now);
        self.events
            .try_send(Event::ResultProgress {
                result: self.result,
                rows_buffered: self.buffered,
            })
            .ok();
    }
}

// @NOTE: a free function because the actor holds a non-Sync session, so `&TabActor` cannot cross
// an await. A closed event channel means the UI is gone; the shutdown path handles the rest.
async fn emit(events: &Sender<Event>, event: Event) {
    events.send(event).await.ok();
}
