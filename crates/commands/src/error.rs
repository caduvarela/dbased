/// One variant per `CoreError` variant (docs/PROJECT.md 11.2); the mapping lives in `core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    Connection,
    Authentication,
    Tls,
    Timeout,
    Configuration,
    Database,
    Protocol,
    SessionClosed,
    CancelFailed,
    NotImplemented,
    Unsupported,
    PasswordRequired,
    UnknownConnection,
    UnknownTab,
    TabBusy,
    EmptyScript,
    Storage,
    SecretStore,
    Internal,
}

/// Server-side error details; `position` is the 1-based character offset into the statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseError {
    pub sqlstate: String,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
    pub position: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorInfo {
    pub kind: ErrorKind,
    pub message: String,
    pub database: Option<DatabaseError>,
}
