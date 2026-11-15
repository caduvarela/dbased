use secrecy::SecretString;

use crate::{ConnectionId, DatabaseError, ErrorInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseKind {
    Postgres,
    MySql,
}

/// The verify-* modes arrive with docs/PROJECT.md 5.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    Disable,
    Prefer,
    Require,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSettings {
    pub name: String,
    pub kind: DatabaseKind,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    pub tls: TlsMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSummary {
    pub id: ConnectionId,
    pub settings: ConnectionSettings,
    pub password_saved: bool,
    pub state: ConnectionState,
}

/// Used by `SaveConnection`.
#[derive(Debug, Clone)]
pub enum PasswordUpdate {
    Keep,
    Save(SecretString),
    Forget,
}

/// Used by `TestConnection` and `Connect`.
#[derive(Debug, Clone)]
pub enum PasswordSource {
    Saved,
    Typed(SecretString),
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecuteTarget {
    /// Byte offset into the script.
    Current {
        cursor: usize,
    },
    /// Byte range into the script.
    Selection {
        start: usize,
        end: usize,
    },
    All,
}

/// Byte range of one statement in the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatementSpan {
    pub index: u32,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataNode {
    Connection,
    Schema { schema: String },
    Table { schema: String, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataItemKind {
    Schema,
    Table,
    View,
    Column { type_name: String, nullable: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataItem {
    pub kind: MetadataItemKind,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionResetReason {
    DrainTimeout,
    HealthCheckFailed,
    ConnectionError,
    FutureDropped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    NotOpen,
    Open,
    TransactionAborted,
    Reset { reason: SessionResetReason },
}

/// Mirrors the core's statement status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStatus {
    Completed,
    Failed,
    Cancelled,
    Truncated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultOutcome {
    pub status: ResultStatus,
    pub rows_received: u64,
    pub rows_affected: Option<u64>,
    pub partial: bool,
    pub error: Option<DatabaseError>,
    pub cancel_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionStatus {
    Completed,
    StoppedOnError { statement: u32 },
    Cancelled { statement: u32 },
    Aborted { statement: u32, error: ErrorInfo },
}

/// How an export writes its file (docs/PROJECT.md 3.3). The default is comma, header, no BOM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsvOptions {
    /// One ASCII character other than a quote or a line break.
    pub delimiter: char,
    pub header: bool,
    /// A UTF-8 byte order mark, for spreadsheets that need it to detect the encoding.
    pub bom: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: ',',
            header: true,
            bom: false,
        }
    }
}

/// How an export that did not fail ended. Failures end with `RequestFailed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportStatus {
    Completed,
    /// Stopped on request; the partial file was removed.
    Cancelled,
}
