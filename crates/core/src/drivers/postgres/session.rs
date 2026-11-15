use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dbased_commands::ColumnMeta;
use futures_util::StreamExt as _;
use sqlx::postgres::types::Oid;
use sqlx::postgres::{PgPool, PgRow};
use sqlx::{Column as _, Connection as _, Executor as _, PgConnection, Row as _, Statement as _};
use sqlx::{Either, TypeInfo as _};
use tokio::time::{Instant, sleep_until, timeout};
use tokio_util::sync::CancellationToken;

use super::errors::{
    QUERY_CANCELED, TRANSACTION_ABORTED, database_error_info, map_sqlx_error,
    sqlstate_means_session_lost,
};
use super::sql::{assert_user_sql, looks_row_returning};
use super::values::{convert_row, estimated_size};
use super::{PostgresConfig, TypeNameCache};
use crate::{
    CancelReason, Canceller, CloseReason, CoreError, DatabaseErrorInfo, RowSink, RowSinkMessage,
    Session, SessionHealth, SessionId, SessionLimits, StatementOutcome, StatementStatus, UserSql,
};

#[derive(Clone)]
pub(super) struct SessionSettings {
    limits: SessionLimits,
    drain_timeout: Duration,
    health_check_timeout: Duration,
}

impl SessionSettings {
    pub(super) fn from_config(config: &PostgresConfig) -> Self {
        Self {
            limits: config.limits,
            drain_timeout: config.drain_timeout,
            health_check_timeout: config.health_check_timeout,
        }
    }
}

/// How often a cancel is re-sent while draining, in case the first one arrived before the
/// statement started or was absorbed (docs/PROJECT.md 3.2).
const CANCEL_RESEND_INTERVAL: Duration = Duration::from_millis(500);
/// Bound for messages that must not block forever once the caller stopped listening.
const FINAL_SEND_TIMEOUT: Duration = Duration::from_secs(1);
const TYPE_LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Open,
    /// An operation is in flight. If a future is dropped mid-operation the state stays `Busy`,
    /// which the next call reads as "protocol state unknown" and closes the session
    /// (docs/PROJECT.md 3.2).
    Busy,
    Closed(CloseReason),
}

pub(super) struct PostgresSession {
    id: SessionId,
    connection: Option<PgConnection>,
    lifecycle: Lifecycle,
    canceller: Arc<dyn Canceller>,
    settings: SessionSettings,
    type_names: Arc<TypeNameCache>,
    pool: PgPool,
}

impl PostgresSession {
    pub(super) fn new(
        id: SessionId,
        connection: PgConnection,
        canceller: Arc<dyn Canceller>,
        settings: SessionSettings,
        type_names: Arc<TypeNameCache>,
        pool: PgPool,
    ) -> Self {
        Self {
            id,
            connection: Some(connection),
            lifecycle: Lifecycle::Open,
            canceller,
            settings,
            type_names,
            pool,
        }
    }

    /// Drops the physical connection once the session is no longer usable.
    ///
    /// @NOTE: a dropped `execute` future cannot cancel the backend by itself, so the cancel
    /// happens on the next call or on `close()`. A session dropped without `close()` leaves the
    /// abandoned statement running until the server notices the disconnect.
    async fn release_connection(&mut self, reason: CloseReason) {
        if matches!(self.lifecycle, Lifecycle::Open | Lifecycle::Busy) {
            self.lifecycle = Lifecycle::Closed(reason);
        }
        let Some(connection) = self.connection.take() else {
            return;
        };
        if reason != CloseReason::ClosedByCaller {
            // @NOTE: an abnormal close may leave a statement running; this is the last moment
            // the connection is held, so a late cancel cannot hit a stranger.
            self.canceller.cancel(self.id).await.ok();
        }
        connection.close_hard().await.ok();
    }

    async fn ensure_open(&mut self) -> Result<(), CoreError> {
        if self.lifecycle == Lifecycle::Busy {
            self.lifecycle = Lifecycle::Closed(CloseReason::FutureDropped);
        }
        match self.lifecycle {
            Lifecycle::Open if self.connection.is_some() => Ok(()),
            Lifecycle::Open | Lifecycle::Busy => {
                Err(CoreError::SessionClosed(CloseReason::ConnectionError))
            }
            Lifecycle::Closed(reason) => {
                self.release_connection(reason).await;
                Err(CoreError::SessionClosed(reason))
            }
        }
    }

    /// Ends an operation that finished without closing the session.
    fn finish_operation(&mut self) {
        if self.lifecycle == Lifecycle::Busy {
            self.lifecycle = Lifecycle::Open;
        }
    }

    async fn probe(&mut self) -> SessionHealth {
        let Some(connection) = self.connection.as_mut() else {
            return SessionHealth::Closed(CloseReason::ConnectionError);
        };
        let health = probe_connection(connection, self.settings.health_check_timeout).await;
        if let SessionHealth::Closed(reason) = health {
            self.release_connection(reason).await;
        }
        health
    }
}

#[async_trait]
impl Session for PostgresSession {
    fn session_id(&self) -> SessionId {
        self.id
    }

    async fn execute(
        &mut self,
        sql: UserSql,
        sink: RowSink,
        token: CancellationToken,
    ) -> Result<StatementOutcome, CoreError> {
        self.ensure_open().await?;
        if token.is_cancelled() {
            // @NOTE: a cancel that fired before the first poll would otherwise be lost.
            return Ok(cancelled_before_start(&sink).await);
        }
        self.lifecycle = Lifecycle::Busy;
        let Some(connection) = self.connection.as_mut() else {
            self.lifecycle = Lifecycle::Closed(CloseReason::ConnectionError);
            return Err(CoreError::SessionClosed(CloseReason::ConnectionError));
        };
        let context = RunContext {
            session_id: self.id,
            canceller: self.canceller.as_ref(),
            settings: &self.settings,
            type_names: &self.type_names,
            pool: &self.pool,
            token: &token,
        };
        let mut report = run_statement(&context, connection, &sql, &sink, &token).await;

        if let Some(error) = report.fatal.take() {
            self.release_connection(CloseReason::ConnectionError).await;
            report.session_failed = true;
            let outcome = report.into_outcome(SessionHealth::Closed(CloseReason::ConnectionError));
            send_best_effort(&sink, RowSinkMessage::Outcome(outcome)).await;
            return Err(error);
        }
        let health = if report.drain_timed_out {
            self.release_connection(CloseReason::DrainTimeout).await;
            SessionHealth::Closed(CloseReason::DrainTimeout)
        } else if report.describe_timed_out {
            // @NOTE: the describe future was dropped on the connection, so its state is unknown.
            self.release_connection(CloseReason::FutureDropped).await;
            SessionHealth::Closed(CloseReason::FutureDropped)
        } else if report.server_error_means_session_lost() {
            self.release_connection(CloseReason::ConnectionError).await;
            SessionHealth::Closed(CloseReason::ConnectionError)
        } else if report.cancel.is_some() {
            self.probe().await
        } else if report.server_error_has_code(TRANSACTION_ABORTED) {
            SessionHealth::TransactionAborted
        } else {
            SessionHealth::Healthy
        };
        self.finish_operation();
        let outcome = report.into_outcome(health);
        send_best_effort(&sink, RowSinkMessage::Outcome(outcome.clone())).await;
        Ok(outcome)
    }

    async fn health_check(&mut self) -> Result<SessionHealth, CoreError> {
        if self.lifecycle == Lifecycle::Busy {
            self.lifecycle = Lifecycle::Closed(CloseReason::FutureDropped);
        }
        if let Lifecycle::Closed(reason) = self.lifecycle {
            self.release_connection(reason).await;
            return Ok(SessionHealth::Closed(reason));
        }
        self.lifecycle = Lifecycle::Busy;
        let health = self.probe().await;
        self.finish_operation();
        Ok(health)
    }

    async fn close(mut self: Box<Self>) {
        self.release_connection(CloseReason::ClosedByCaller).await;
    }
}

/// A cancelled token before the statement was sent: nothing runs, nothing is lost.
async fn cancelled_before_start(sink: &RowSink) -> StatementOutcome {
    let report = RunReport {
        cancel: Some(CancelReason::User),
        ..RunReport::default()
    };
    let outcome = report.into_outcome(SessionHealth::Healthy);
    send_best_effort(sink, RowSinkMessage::Columns(Vec::new())).await;
    send_best_effort(sink, RowSinkMessage::Outcome(outcome.clone())).await;
    outcome
}

/// Delivers a message unless the consumer is gone or stuck; the caller never blocks forever.
async fn send_best_effort(sink: &RowSink, message: RowSinkMessage) -> bool {
    matches!(
        timeout(FINAL_SEND_TIMEOUT, sink.send(message)).await,
        Ok(Ok(()))
    )
}

/// `SELECT 1`, retried once because a cancel that arrived late may be absorbed by it.
async fn probe_connection(connection: &mut PgConnection, limit: Duration) -> SessionHealth {
    for attempt in 0..2 {
        let probe = sqlx::raw_sql("SELECT 1").execute(&mut *connection);
        match timeout(limit, probe).await {
            Ok(Ok(_)) => return SessionHealth::Healthy,
            Ok(Err(sqlx::Error::Database(error))) => match error.code().as_deref() {
                Some(TRANSACTION_ABORTED) => return SessionHealth::TransactionAborted,
                Some(QUERY_CANCELED) if attempt == 0 => continue,
                _ => return SessionHealth::Closed(CloseReason::HealthCheckFailed),
            },
            Ok(Err(_)) | Err(_) => return SessionHealth::Closed(CloseReason::HealthCheckFailed),
        }
    }
    SessionHealth::Closed(CloseReason::HealthCheckFailed)
}

struct RunContext<'a> {
    session_id: SessionId,
    canceller: &'a dyn Canceller,
    settings: &'a SessionSettings,
    type_names: &'a TypeNameCache,
    pool: &'a PgPool,
    token: &'a CancellationToken,
}

#[derive(Default)]
struct RunReport {
    rows_received: u64,
    first_result_rows_affected: Option<u64>,
    headers_known: bool,
    cancel: Option<CancelReason>,
    cancel_error: Option<String>,
    server_error: Option<DatabaseErrorInfo>,
    extra_statements: bool,
    drain_timed_out: bool,
    describe_timed_out: bool,
    /// A failure of the session itself (not a server error); the session must close.
    fatal: Option<CoreError>,
    session_failed: bool,
}

impl RunReport {
    fn server_error_means_session_lost(&self) -> bool {
        self.server_error
            .as_ref()
            .is_some_and(|error| sqlstate_means_session_lost(&error.sqlstate))
    }

    fn server_error_has_code(&self, sqlstate: &str) -> bool {
        self.server_error
            .as_ref()
            .is_some_and(|error| error.sqlstate == sqlstate)
    }

    fn into_outcome(self, session: SessionHealth) -> StatementOutcome {
        let status = match (self.cancel, &self.server_error) {
            (Some(CancelReason::RowCap | CancelReason::MemoryCap), _) => StatementStatus::Truncated,
            (Some(CancelReason::User | CancelReason::ConsumerGone), _) => {
                StatementStatus::Cancelled
            }
            (None, Some(_)) => StatementStatus::Failed,
            (None, None) if self.session_failed => StatementStatus::Failed,
            (None, None) => StatementStatus::Completed,
        };
        let partial = matches!(
            status,
            StatementStatus::Cancelled | StatementStatus::Truncated
        );
        let rows_affected = if self.rows_received == 0 && !self.headers_known {
            self.first_result_rows_affected
        } else {
            None
        };
        StatementOutcome {
            status,
            rows_received: self.rows_received,
            rows_affected,
            partial,
            server_error: self.server_error,
            cancel_error: self.cancel_error,
            extra_statements_discarded: self.extra_statements,
            session,
        }
    }
}

/// Tracks the explicit cancel of one statement (docs/PROJECT.md 3.2, rule 5).
struct CancelState {
    reason: Option<CancelReason>,
    error: Option<String>,
    delivered: bool,
    drain_deadline: Option<Instant>,
    next_resend: Instant,
}

impl CancelState {
    fn new() -> Self {
        Self {
            reason: None,
            error: None,
            delivered: false,
            drain_deadline: None,
            next_resend: Instant::now(),
        }
    }

    async fn request(&mut self, context: &RunContext<'_>, reason: CancelReason) {
        if self.reason.is_some() {
            return;
        }
        self.reason = Some(reason);
        // @NOTE: only called while the statement's connection is still held by this session.
        match context.canceller.cancel(context.session_id).await {
            Ok(()) => self.delivered = true,
            Err(error) => self.error = Some(error.to_string()),
        }
        // @NOTE: the drain clock starts after the cancel returns, so a slow cancel does not eat it.
        let now = Instant::now();
        self.drain_deadline = Some(now + context.settings.drain_timeout);
        self.next_resend = now + CANCEL_RESEND_INTERVAL;
    }

    /// Re-sends the cancel while the statement is still draining.
    async fn resend(&mut self, context: &RunContext<'_>) {
        self.next_resend = Instant::now() + CANCEL_RESEND_INTERVAL;
        let Some(deadline) = self.drain_deadline else {
            return;
        };
        let sending = context.canceller.cancel(context.session_id);
        match tokio::time::timeout_at(deadline, sending).await {
            Ok(Ok(())) if !self.delivered => {
                self.delivered = true;
                self.error = None;
            }
            Ok(Err(error)) if !self.delivered => self.error = Some(error.to_string()),
            _ => {}
        }
    }
}

async fn run_statement(
    context: &RunContext<'_>,
    connection: &mut PgConnection,
    sql: &UserSql,
    sink: &RowSink,
    token: &CancellationToken,
) -> RunReport {
    let mut report = RunReport::default();
    let mut cancel = CancelState::new();
    let mut columns_sent = false;
    let mut pending_columns: Option<Vec<ColumnMeta>> = None;
    let mut bytes_buffered = 0usize;
    let mut results_seen = 0u32;

    {
        let mut stream = sqlx::raw_sql(assert_user_sql(sql)).fetch_many(&mut *connection);
        loop {
            let item = match cancel.drain_deadline {
                Some(deadline) => tokio::select! {
                    item = stream.next() => item,
                    () = sleep_until(deadline) => {
                        report.drain_timed_out = true;
                        break;
                    }
                    () = sleep_until(cancel.next_resend) => {
                        cancel.resend(context).await;
                        continue;
                    }
                },
                None => tokio::select! {
                    () = token.cancelled() => {
                        cancel.request(context, CancelReason::User).await;
                        continue;
                    }
                    item = stream.next() => item,
                },
            };
            let Some(item) = item else {
                break;
            };
            match item {
                Err(sqlx::Error::Database(error)) => {
                    let info = database_error_info(error.as_ref());
                    report.server_error.get_or_insert(info);
                }
                Err(error) => {
                    report.fatal = Some(map_sqlx_error(error));
                    break;
                }
                Ok(Either::Left(result)) => {
                    results_seen += 1;
                    if results_seen == 1 {
                        report.first_result_rows_affected = Some(result.rows_affected());
                    } else {
                        report.extra_statements = true;
                    }
                }
                Ok(Either::Right(row)) => {
                    if results_seen > 0 {
                        report.extra_statements = true;
                        continue;
                    }
                    if cancel.reason.is_some() {
                        continue;
                    }
                    if !columns_sent {
                        let columns = match pending_columns.take() {
                            Some(columns) => columns,
                            None => column_metadata(context, &row).await,
                        };
                        let message = RowSinkMessage::Columns(columns.clone());
                        if !send_or_cancel(sink, message, token, &mut cancel, context).await {
                            pending_columns = Some(columns);
                            continue;
                        }
                        columns_sent = true;
                        report.headers_known = true;
                    }
                    let values = match convert_row(&row) {
                        Ok(values) => values,
                        Err(error) => {
                            report.fatal = Some(error);
                            break;
                        }
                    };
                    let size = estimated_size(&values);
                    let limits = context.settings.limits;
                    if report.rows_received >= limits.max_rows {
                        cancel.request(context, CancelReason::RowCap).await;
                        continue;
                    }
                    if report.rows_received > 0 && bytes_buffered + size > limits.max_bytes {
                        cancel.request(context, CancelReason::MemoryCap).await;
                        continue;
                    }
                    let message = RowSinkMessage::Row(values);
                    if send_or_cancel(sink, message, token, &mut cancel, context).await {
                        report.rows_received += 1;
                        bytes_buffered += size;
                    }
                }
            }
        }
    }

    report.cancel = cancel.reason;
    report.cancel_error = cancel.error;
    if !columns_sent {
        let usable = report.server_error.is_none()
            && report.cancel.is_none()
            && report.fatal.is_none()
            && !report.drain_timed_out;
        let columns = match pending_columns {
            Some(columns) => columns,
            None if usable && looks_row_returning(sql.as_str()) => {
                match describe_columns(context, connection, sql).await {
                    Some(columns) => columns,
                    None => {
                        report.describe_timed_out = true;
                        Vec::new()
                    }
                }
            }
            None => Vec::new(),
        };
        let has_columns = !columns.is_empty();
        let sent = send_best_effort(sink, RowSinkMessage::Columns(columns)).await;
        report.headers_known = sent && has_columns;
    }
    report
}

/// Sends one message, but keeps honouring the token while the sink is full.
/// Returns whether the message was actually delivered.
async fn send_or_cancel(
    sink: &RowSink,
    message: RowSinkMessage,
    token: &CancellationToken,
    cancel: &mut CancelState,
    context: &RunContext<'_>,
) -> bool {
    if cancel.reason.is_some() {
        return false;
    }
    tokio::select! {
        sent = sink.send(message) => {
            if sent.is_err() {
                cancel.request(context, CancelReason::ConsumerGone).await;
                return false;
            }
            true
        }
        () = token.cancelled() => {
            cancel.request(context, CancelReason::User).await;
            false
        }
    }
}

async fn column_metadata(context: &RunContext<'_>, row: &PgRow) -> Vec<ColumnMeta> {
    let mut columns = Vec::with_capacity(row.columns().len());
    for column in row.columns() {
        let type_info = column.type_info();
        let mut type_name = type_info.name().to_owned();
        if type_name == "?"
            && let Some(oid) = type_info.oid()
        {
            type_name = resolve_type_name(context, oid.0).await.unwrap_or(type_name);
        }
        columns.push(ColumnMeta {
            name: column.name().to_owned(),
            type_name,
            nullable: None,
        });
    }
    columns
}

async fn resolve_type_name(context: &RunContext<'_>, oid: u32) -> Option<String> {
    if let Some(name) = context.type_names.get(oid) {
        return Some(name);
    }
    let acquiring = timeout(TYPE_LOOKUP_TIMEOUT, context.pool.acquire());
    let mut connection = tokio::select! {
        acquired = acquiring => acquired.ok()?.ok()?,
        () = context.token.cancelled() => return None,
    };
    let finished = {
        let lookup = sqlx::query_scalar::<_, String>("SELECT format_type($1, NULL)")
            .bind(Oid(oid))
            .fetch_one(&mut *connection);
        tokio::select! {
            result = timeout(TYPE_LOOKUP_TIMEOUT, lookup) => result.ok(),
            () = context.token.cancelled() => None,
        }
    };
    let Some(result) = finished else {
        // @NOTE: the lookup future was dropped mid-query, so this connection must not return to the pool.
        connection.detach().close_hard().await.ok();
        return None;
    };
    let name = result.ok()?;
    context.type_names.insert(oid, name.clone());
    Some(name)
}

/// Prepares (never executes) the statement to learn its headers; a failure just shows none.
/// `None` means the prepare timed out and its future was dropped on the connection.
async fn describe_columns(
    context: &RunContext<'_>,
    connection: &mut PgConnection,
    sql: &UserSql,
) -> Option<Vec<ColumnMeta>> {
    let preparing = connection.prepare(sqlx::SqlSafeStr::into_sql_str(assert_user_sql(sql)));
    let Ok(prepared) = timeout(context.settings.health_check_timeout, preparing).await else {
        return None;
    };
    let Ok(statement) = prepared else {
        return Some(Vec::new());
    };
    Some(
        statement
            .columns()
            .iter()
            .map(|column| ColumnMeta {
                name: column.name().to_owned(),
                type_name: column.type_info().name().to_owned(),
                nullable: None,
            })
            .collect(),
    )
}
