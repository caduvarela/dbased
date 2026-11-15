use thiserror::Error;

use crate::driver::CloseReason;

/// A database-reported error, with everything the editor needs to explain it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseErrorInfo {
    pub sqlstate: String,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
    /// 1-based character offset into the statement, when the server gives one.
    pub position: Option<u32>,
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("could not connect: {0}")]
    Connection(String),
    #[error("authentication failed: {0}")]
    Authentication(String),
    #[error("TLS failure: {0}")]
    Tls(String),
    #[error("timed out: {0}")]
    Timeout(&'static str),
    #[error("invalid configuration: {0}")]
    Configuration(String),
    #[error("{} (SQLSTATE {})", .0.message, .0.sqlstate)]
    Database(DatabaseErrorInfo),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("the session is closed ({0:?})")]
    SessionClosed(CloseReason),
    #[error("cancel failed: {0}")]
    CancelFailed(String),
    #[error("not implemented yet: {0}")]
    NotImplemented(&'static str),
    #[error("operation not supported by this driver: {0}")]
    Unsupported(&'static str),
}

impl CoreError {
    /// Stable machine-readable category, mirrored by the `commands` error shape (3.4).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Connection(_) => "connection",
            Self::Authentication(_) => "authentication",
            Self::Tls(_) => "tls",
            Self::Timeout(_) => "timeout",
            Self::Configuration(_) => "configuration",
            Self::Database(_) => "database",
            Self::Protocol(_) => "protocol",
            Self::SessionClosed(_) => "session_closed",
            Self::CancelFailed(_) => "cancel_failed",
            Self::NotImplemented(_) => "not_implemented",
            Self::Unsupported(_) => "unsupported",
        }
    }
}
