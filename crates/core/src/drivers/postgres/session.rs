use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dbased_commands::ColumnMeta;
use futures_util::StreamExt as _;
use sqlx::postgres::types::Oid;
use sqlx::postgres::{PgPool, PgRow};
use sqlx::{Column as _, Connection as _, Executor as _, PgConnection, Row as _, Statement as _};
use sqlx::{Either, TypeInfo as _};
use tokio::time::{Instant, timeout, timeout_at};
use tokio_util::sync::CancellationToken;

use super::errors::{QUERY_CANCELED, TRANSACTION_ABORTED, database_error_info, map_sqlx_error};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Open,
    Closed(CloseReason),
}

/// Marks the session closed if the `execute` future is dropped mid-operation:
/// the protocol state is then unknown (docs/PROJECT.md 3.2).
struct PoisonGuard<'a> {
    lifecycle: &'a mut Lifecycle,
    armed: bool,
}

impl<'a> PoisonGuard<'a> {
    fn arm(lifecycle: &'a mut Lifecycle) -> Self {
        Self {
            lifecycle,
            armed: true,
        }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for PoisonGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            *self.lifecycle = Lifecycle::Closed(CloseReason::FutureDropped);
        }
    }
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
    async fn release_connection(&mut self, reason: CloseReason) {
        if self.lifecycle == Lifecycle::Open {
            self.lifecycle = Lifecycle::Closed(reason);
        }
        let Some(connection) = self.connection.take() else {
            return;
        };
        if reason == CloseReason::FutureDropped {
            // @NOTE: the abandoned statement may still run; this is the last moment the
            // connection is held, so a late cancel cannot hit a stranger.
            self.canceller.cancel(self.id).await.ok();
        }
        connection.close_hard().await.ok();
    }

    async fn ensure_open(&mut self) -> Result<(), CoreError> {
        match self.lifecycle {
            Lifecycle::Open if self.connection.is_some() => Ok(()),
            Lifecycle::Open => Err(CoreError::SessionClosed(CloseReason::ConnectionError)),
            Lifecycle::Closed(reason) => {
                self.release_connection(reason).await;
                Err(CoreError::SessionClosed(reason))
            }
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
        let Some(connection) = self.connection.as_mut() else {
            return Err(CoreError::SessionClosed(CloseReason::ConnectionError));
        };
        let guard = PoisonGuard::arm(&mut self.lifecycle);
        let context = RunContext {
            session_id: self.id,
            canceller: self.canceller.as_ref(),
            settings: &self.settings,
            type_names: &self.type_names,
            pool: &self.pool,
        };
        let run = run_statement(&context, connection, &sql, &sink, &token).await;
        guard.disarm();

        let report = match run {
            Ok(report) => report,
            Err(error) => {
                self.release_connection(CloseReason::ConnectionError).await;
                return Err(error);
            }
        };
        let health = if report.drain_timed_out {
            self.release_connection(CloseReason::DrainTimeout).await;
            SessionHealth::Closed(CloseReason::DrainTimeout)
        } else if report.cancel.is_some() {
            self.probe().await
        } else if report.server_error_has_code(TRANSACTION_ABORTED) {
            SessionHealth::TransactionAborted
        } else {
            SessionHealth::Healthy
        };
        let outcome = report.into_outcome(health);
        // @NOTE: a receiver that is gone already stopped listening; the outcome is returned anyway.
        sink.send(RowSinkMessage::Outcome(outcome.clone()))
            .await
            .ok();
        Ok(outcome)
    }

    async fn health_check(&mut self) -> Result<SessionHealth, CoreError> {
        if let Lifecycle::Closed(reason) = self.lifecycle {
            self.release_connection(reason).await;
            return Ok(SessionHealth::Closed(reason));
        }
        Ok(self.probe().await)
    }

    async fn close(mut self: Box<Self>) {
        self.release_connection(CloseReason::ClosedByCaller).await;
    }
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
}

impl RunReport {
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
    drain_deadline: Option<Instant>,
}

impl CancelState {
    async fn request(&mut self, context: &RunContext<'_>, reason: CancelReason) {
        if self.reason.is_some() {
            return;
        }
        self.reason = Some(reason);
        self.drain_deadline = Some(Instant::now() + context.settings.drain_timeout);
        // @NOTE: only called while the statement's connection is still held by this session.
        if let Err(error) = context.canceller.cancel(context.session_id).await {
            self.error = Some(error.to_string());
        }
    }
}

async fn run_statement(
    context: &RunContext<'_>,
    connection: &mut PgConnection,
    sql: &UserSql,
    sink: &RowSink,
    token: &CancellationToken,
) -> Result<RunReport, CoreError> {
    let mut report = RunReport::default();
    let mut cancel = CancelState {
        reason: None,
        error: None,
        drain_deadline: None,
    };
    let mut columns_sent = false;
    let mut bytes_buffered = 0usize;
    let mut results_seen = 0u32;

    {
        let mut stream = sqlx::raw_sql(assert_user_sql(sql)).fetch_many(&mut *connection);
        loop {
            let item = match cancel.drain_deadline {
                Some(deadline) => match timeout_at(deadline, stream.next()).await {
                    Ok(item) => item,
                    Err(_) => {
                        report.drain_timed_out = true;
                        break;
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
                Err(error) => return Err(map_sqlx_error(error)),
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
                        let columns = column_metadata(context, &row).await;
                        send_or_cancel(
                            sink,
                            RowSinkMessage::Columns(columns),
                            token,
                            &mut cancel,
                            context,
                        )
                        .await;
                        columns_sent = true;
                        report.headers_known = true;
                    }
                    let values = convert_row(&row)?;
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
                    report.rows_received += 1;
                    bytes_buffered += size;
                    send_or_cancel(
                        sink,
                        RowSinkMessage::Row(values),
                        token,
                        &mut cancel,
                        context,
                    )
                    .await;
                }
            }
        }
    }

    report.cancel = cancel.reason;
    report.cancel_error = cancel.error;
    if report.drain_timed_out {
        return Ok(report);
    }
    if !columns_sent {
        let healthy = report.server_error.is_none() && report.cancel.is_none();
        let columns = if healthy && looks_row_returning(sql.as_str()) {
            describe_columns(context, connection, sql).await
        } else {
            Vec::new()
        };
        report.headers_known = !columns.is_empty();
        sink.send(RowSinkMessage::Columns(columns)).await.ok();
    }
    Ok(report)
}

/// Sends one message, but keeps honouring the token while the sink is full.
async fn send_or_cancel(
    sink: &RowSink,
    message: RowSinkMessage,
    token: &CancellationToken,
    cancel: &mut CancelState,
    context: &RunContext<'_>,
) {
    if cancel.reason.is_some() {
        return;
    }
    tokio::select! {
        sent = sink.send(message) => {
            if sent.is_err() {
                cancel.request(context, CancelReason::ConsumerGone).await;
            }
        }
        () = token.cancelled() => cancel.request(context, CancelReason::User).await,
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
    let lookup = sqlx::query_scalar::<_, String>("SELECT format_type($1, NULL)")
        .bind(Oid(oid))
        .fetch_one(context.pool);
    let name = timeout(Duration::from_secs(2), lookup).await.ok()?.ok()?;
    context.type_names.insert(oid, name.clone());
    Some(name)
}

/// Prepares (never executes) the statement to learn its headers; a failure just shows none.
async fn describe_columns(
    context: &RunContext<'_>,
    connection: &mut PgConnection,
    sql: &UserSql,
) -> Vec<ColumnMeta> {
    let preparing = connection.prepare(sqlx::SqlSafeStr::into_sql_str(assert_user_sql(sql)));
    let Ok(Ok(statement)) = timeout(context.settings.health_check_timeout, preparing).await else {
        return Vec::new();
    };
    statement
        .columns()
        .iter()
        .map(|column| ColumnMeta {
            name: column.name().to_owned(),
            type_name: column.type_info().name().to_owned(),
            nullable: None,
        })
        .collect()
}
