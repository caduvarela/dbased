use std::sync::Arc;

use async_trait::async_trait;
use dbased_commands::{ColumnMeta, Value};
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{CoreError, DatabaseErrorInfo, UserSql};

/// Backend identifier captured when a session opens (`pg_backend_pid()` /
/// `CONNECTION_ID()`), used by cancellation (docs/PROJECT.md 2.1, 3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRef {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewRef {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routine {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRequest {
    pub limit: u64,
    pub offset: u64,
}

/// The two result caps of docs/PROJECT.md 2.3; whichever is reached first wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLimits {
    pub max_rows: u64,
    pub max_bytes: usize,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_rows: 200_000,
            max_bytes: 512 * 1024 * 1024,
        }
    }
}

/// Why the Core asked the server to stop a statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    User,
    RowCap,
    MemoryCap,
    /// The receiving end of the sink went away.
    ConsumerGone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    ClosedByCaller,
    /// The statement did not reach its terminal result within the drain timeout.
    DrainTimeout,
    HealthCheckFailed,
    ConnectionError,
    /// An `execute` future was dropped mid-operation, so protocol state is unknown.
    FutureDropped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionHealth {
    Healthy,
    /// SQLSTATE 25P02: alive, but the user's transaction needs a ROLLBACK.
    TransactionAborted,
    /// The session was closed; the next run needs a fresh one.
    Closed(CloseReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementStatus {
    Completed,
    /// The server reported an error and the Core did not cancel.
    Failed,
    /// The Core issued a cancel for the user. The server outcome is in
    /// `server_error`: `None` means the server reported success and the
    /// statement may have taken effect.
    Cancelled,
    /// A row or memory cap stopped the statement (2.3).
    Truncated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementOutcome {
    pub status: StatementStatus,
    pub rows_received: u64,
    /// Only for statements that return no rows (2.3).
    pub rows_affected: Option<u64>,
    /// Rows received so far are kept but the result is incomplete.
    pub partial: bool,
    pub server_error: Option<DatabaseErrorInfo>,
    /// Set when the side-connection cancel itself failed (3.2: reported, not swallowed).
    pub cancel_error: Option<String>,
    /// The call carried more than one statement; only the first one's rows were
    /// streamed. Callers must split scripts first (1.2 rule 2).
    pub extra_statements_discarded: bool,
    pub session: SessionHealth,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RowSinkMessage {
    /// Always first, exactly once (empty for statements without columns).
    Columns(Vec<ColumnMeta>),
    Row(Vec<Value>),
    /// Always last; also returned by `Session::execute`.
    Outcome(StatementOutcome),
}

/// Bounded channel the driver streams a result into; a full channel slows the read.
#[derive(Debug, Clone)]
pub struct RowSink {
    sender: mpsc::Sender<RowSinkMessage>,
}

impl RowSink {
    pub fn channel(capacity: usize) -> (Self, mpsc::Receiver<RowSinkMessage>) {
        let (sender, receiver) = mpsc::channel(capacity.max(1));
        (Self { sender }, receiver)
    }

    /// Fails only when the receiver is gone.
    pub async fn send(&self, message: RowSinkMessage) -> Result<(), CoreError> {
        self.sender
            .send(message)
            .await
            .map_err(|_| CoreError::Connection("the result receiver was dropped".to_owned()))
    }
}

#[async_trait]
pub trait Driver: Send + Sync {
    /// Opens a dedicated session (one physical connection, outside the pool) (2.2).
    async fn open_session(&self) -> Result<Box<dyn Session>, CoreError>;

    async fn list_schemas(&self) -> Result<Vec<Schema>, CoreError>;
    async fn list_tables(&self, schema: &str) -> Result<Vec<TableRef>, CoreError>;
    async fn list_columns(&self, table: &TableRef) -> Result<Vec<Column>, CoreError>;
    async fn list_views(&self, schema: &str) -> Result<Vec<ViewRef>, CoreError>;
    async fn list_routines(&self, schema: &str) -> Result<Vec<Routine>, CoreError>;

    /// Closes the pool. Callers close every session first: cancels use the pool (2.2).
    async fn close(&self);

    fn quote_identifier(&self, identifier: &str) -> String;
    fn paginate(&self, base_sql: &str, page: PageRequest) -> String;
    // @TODO: `rewrite_parameters` (2.1) arrives with the tokenizer and parameters (2.7, 9.3).
}

/// Free constructor kept off `Driver` so `dyn Driver` stays usable.
#[async_trait]
pub trait DriverFactory {
    type Config: Send;

    async fn connect(config: Self::Config) -> Result<Box<dyn Driver>, CoreError>;
}

#[async_trait]
pub trait Session: Send {
    fn session_id(&self) -> SessionId;

    /// A cloneable handle that cancels this session's running statement from a separate
    /// connection. It stops working once the session releases its connection (3.2).
    fn cancel_handle(&self) -> CancelHandle;

    /// Runs ONE statement and streams its rows into the sink. On cancel it
    /// drains to the terminal result; it never returns with the statement
    /// still in flight (3.2). Server errors come back in the outcome; `Err` is
    /// for failures of the session itself.
    async fn execute(
        &mut self,
        sql: UserSql,
        sink: RowSink,
        token: CancellationToken,
    ) -> Result<StatementOutcome, CoreError>;

    /// Cheap liveness probe (`SELECT 1`) used after a cancel (3.2).
    async fn health_check(&mut self) -> Result<SessionHealth, CoreError>;

    /// Closes the physical connection.
    async fn close(self: Box<Self>);
}

/// Driver-side cancel mechanism, addressed by backend id. Only `CancelHandle` calls it, and
/// only while the session still holds that backend's connection.
#[async_trait]
pub trait Canceller: Send + Sync {
    /// Issues a cancel from a SEPARATE connection (3.2).
    async fn cancel_backend(&self, backend: SessionId) -> Result<(), CoreError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The cancel was delivered to the server.
    Sent,
    /// The session already released its connection; nothing was sent.
    SessionClosed,
}

/// Cancels the statement of ONE live session. A cancel after the session released its
/// connection is a no-op, so it can never reach a stranger that reused the backend id.
#[derive(Clone)]
pub struct CancelHandle {
    backend: SessionId,
    canceller: Arc<dyn Canceller>,
    // @NOTE: `true` while the connection is held. Cancels hold the read side while sending and
    // the session takes the write side before dropping the socket, so none can follow the close.
    held: Arc<RwLock<bool>>,
}

impl CancelHandle {
    pub fn new(backend: SessionId, canceller: Arc<dyn Canceller>) -> Self {
        Self {
            backend,
            canceller,
            held: Arc::new(RwLock::new(true)),
        }
    }

    pub async fn cancel(&self) -> Result<CancelOutcome, CoreError> {
        let held = self.held.read().await;
        if !*held {
            return Ok(CancelOutcome::SessionClosed);
        }
        self.canceller.cancel_backend(self.backend).await?;
        Ok(CancelOutcome::Sent)
    }

    /// Called by the owning session right before it drops its connection.
    pub async fn mark_released(&self) {
        *self.held.write().await = false;
    }

    /// Cancel for the session's own abnormal-close path, which still holds the connection.
    pub async fn cancel_while_held(&self) -> Result<(), CoreError> {
        self.canceller.cancel_backend(self.backend).await
    }
}
