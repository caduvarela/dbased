use sqlx::error::DatabaseError;
use sqlx::postgres::{PgDatabaseError, PgErrorPosition};

use crate::{CoreError, DatabaseErrorInfo};

pub(super) const TRANSACTION_ABORTED: &str = "25P02";
pub(super) const QUERY_CANCELED: &str = "57014";

/// Connection exceptions (class 08) and operator shutdowns (57P01..57P05) end the session.
pub(super) fn sqlstate_means_session_lost(sqlstate: &str) -> bool {
    sqlstate.starts_with("08") || sqlstate.starts_with("57P0")
}

pub(super) fn database_error_info(error: &dyn DatabaseError) -> DatabaseErrorInfo {
    let postgres_error = error.try_downcast_ref::<PgDatabaseError>();
    let position = postgres_error
        .and_then(PgDatabaseError::position)
        .and_then(|position| match position {
            PgErrorPosition::Original(offset) => u32::try_from(offset).ok(),
            PgErrorPosition::Internal { .. } => None,
        });
    DatabaseErrorInfo {
        sqlstate: error
            .code()
            .map(|code| code.into_owned())
            .unwrap_or_default(),
        message: error.message().to_owned(),
        detail: postgres_error.and_then(|error| error.detail().map(str::to_owned)),
        hint: postgres_error.and_then(|error| error.hint().map(str::to_owned)),
        position,
    }
}

/// The only place sqlx errors become `CoreError` (docs/PROJECT.md 3.4).
pub(super) fn map_sqlx_error(error: sqlx::Error) -> CoreError {
    match error {
        sqlx::Error::Database(database_error) => {
            let info = database_error_info(database_error.as_ref());
            map_database_error(info)
        }
        sqlx::Error::Io(error) => CoreError::Connection(error.to_string()),
        sqlx::Error::Tls(error) => CoreError::Tls(error.to_string()),
        sqlx::Error::PoolTimedOut => CoreError::Timeout("acquiring a pool connection"),
        sqlx::Error::PoolClosed => CoreError::Connection("the connection pool is closed".into()),
        sqlx::Error::Configuration(error) => CoreError::Configuration(error.to_string()),
        other => CoreError::Protocol(other.to_string()),
    }
}

fn map_database_error(info: DatabaseErrorInfo) -> CoreError {
    // @NOTE: class 28 is authentication, class 08 and 57P0x mean the server went away.
    if info.sqlstate.starts_with("28") {
        return CoreError::Authentication(info.message);
    }
    if info.sqlstate.starts_with("08") || info.sqlstate.starts_with("57P0") {
        return CoreError::Connection(info.message);
    }
    CoreError::Database(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(sqlstate: &str) -> DatabaseErrorInfo {
        DatabaseErrorInfo {
            sqlstate: sqlstate.into(),
            message: "m".into(),
            detail: None,
            hint: None,
            position: None,
        }
    }

    #[test]
    fn classifies_database_errors_by_sqlstate() {
        assert!(matches!(
            map_database_error(info("28P01")),
            CoreError::Authentication(_)
        ));
        assert!(matches!(
            map_database_error(info("08006")),
            CoreError::Connection(_)
        ));
        assert!(matches!(
            map_database_error(info("57P01")),
            CoreError::Connection(_)
        ));
        assert!(matches!(
            map_database_error(info("42601")),
            CoreError::Database(_)
        ));
        assert!(matches!(
            map_database_error(info("57014")),
            CoreError::Database(_)
        ));
    }

    #[test]
    fn maps_non_database_errors_to_stable_kinds() {
        let io = sqlx::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionRefused));
        assert_eq!(map_sqlx_error(io).kind(), "connection");
        assert_eq!(map_sqlx_error(sqlx::Error::PoolTimedOut).kind(), "timeout");
        assert_eq!(
            map_sqlx_error(sqlx::Error::Protocol("x".into())).kind(),
            "protocol"
        );
    }
}
