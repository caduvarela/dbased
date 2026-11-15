use dbased_commands::{DatabaseError, ErrorInfo, ErrorKind};

use crate::{CoreError, DatabaseErrorInfo};

/// The only mapping from `CoreError` to the contract's error shape (docs/PROJECT.md 3.4).
/// Messages come from `Display`, which never includes secrets or user SQL.
pub fn error_info(error: &CoreError) -> ErrorInfo {
    let database = match error {
        CoreError::Database(info) => Some(database_error(info)),
        _ => None,
    };
    ErrorInfo {
        kind: error_kind(error),
        message: error.to_string(),
        database,
    }
}

pub(crate) fn database_error(info: &DatabaseErrorInfo) -> DatabaseError {
    DatabaseError {
        sqlstate: info.sqlstate.clone(),
        message: info.message.clone(),
        detail: info.detail.clone(),
        hint: info.hint.clone(),
        position: info.position,
    }
}

fn error_kind(error: &CoreError) -> ErrorKind {
    match error {
        CoreError::Connection(_) => ErrorKind::Connection,
        CoreError::Authentication(_) => ErrorKind::Authentication,
        CoreError::Tls(_) => ErrorKind::Tls,
        CoreError::Timeout(_) => ErrorKind::Timeout,
        CoreError::Configuration(_) => ErrorKind::Configuration,
        CoreError::Database(_) => ErrorKind::Database,
        CoreError::Protocol(_) => ErrorKind::Protocol,
        CoreError::SessionClosed(_) => ErrorKind::SessionClosed,
        CoreError::CancelFailed(_) => ErrorKind::CancelFailed,
        CoreError::NotImplemented(_) => ErrorKind::NotImplemented,
        CoreError::Unsupported(_) => ErrorKind::Unsupported,
        CoreError::PasswordRequired => ErrorKind::PasswordRequired,
        CoreError::UnknownConnection => ErrorKind::UnknownConnection,
        CoreError::UnknownTab => ErrorKind::UnknownTab,
        CoreError::TabBusy => ErrorKind::TabBusy,
        CoreError::EmptyScript => ErrorKind::EmptyScript,
        CoreError::Storage(_) => ErrorKind::Storage,
        CoreError::SecretStore(_) => ErrorKind::SecretStore,
        CoreError::Internal(_) => ErrorKind::Internal,
    }
}
