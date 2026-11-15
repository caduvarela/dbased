use sqlx::error::DatabaseError;
use sqlx::mysql::MySqlDatabaseError;

use crate::{CoreError, DatabaseErrorInfo};

/// ER_QUERY_INTERRUPTED: what `KILL QUERY` produces when the server reports it as an error.
pub(super) const QUERY_INTERRUPTED: u16 = 1317;
/// ER_QUERY_TIMEOUT: `max_execution_time` stopped the statement.
pub(super) const QUERY_TIMEOUT: u16 = 3024;
/// ER_NO_SUCH_THREAD: the id given to `KILL` is not (or no longer) a connection.
pub(super) const NO_SUCH_THREAD: u16 = 1094;
/// ER_CANT_CHANGE_TX_CHARACTERISTICS: a transaction is in progress.
pub(super) const TRANSACTION_IN_PROGRESS: u16 = 1568;

const ACCESS_DENIED: u16 = 1045;
const HOST_NOT_ALLOWED: u16 = 1130;
const SERVER_SHUTDOWN: u16 = 1053;
const CONNECTION_KILLED: u16 = 1927;

/// The server's error number, which is far more specific than the SQLSTATE class.
pub(super) fn error_number(error: &dyn DatabaseError) -> Option<u16> {
    error
        .try_downcast_ref::<MySqlDatabaseError>()
        .map(MySqlDatabaseError::number)
}

/// A stopped statement, whether by `KILL QUERY` or by the server's own time limit.
pub(super) fn is_interruption(number: Option<u16>) -> bool {
    matches!(number, Some(QUERY_INTERRUPTED | QUERY_TIMEOUT))
}

/// Connection exceptions (class 08), a server shutdown and a killed connection end the session.
/// `KILL QUERY` (1317) shares SQLSTATE 70100 with 1927, so the number decides, not the class.
pub(super) fn error_means_session_lost(sqlstate: &str, number: Option<u16>) -> bool {
    sqlstate.starts_with("08") || matches!(number, Some(SERVER_SHUTDOWN | CONNECTION_KILLED))
}

pub(super) fn database_error_info(error: &dyn DatabaseError) -> DatabaseErrorInfo {
    // @NOTE: `DatabaseErrorInfo` has no error-number field, so the number rides in `detail`.
    let detail = error_number(error).map(|number| format!("MySQL error {number}"));
    DatabaseErrorInfo {
        sqlstate: error
            .code()
            .map(|code| code.into_owned())
            .unwrap_or_default(),
        message: error.message().to_owned(),
        detail,
        hint: None,
        // @NOTE: MySQL reports where a syntax error is only inside the message text.
        position: None,
    }
}

/// The only place sqlx errors become `CoreError` (docs/PROJECT.md 3.4).
pub(super) fn map_sqlx_error(error: sqlx::Error) -> CoreError {
    match error {
        sqlx::Error::Database(database_error) => {
            let number = error_number(database_error.as_ref());
            let info = database_error_info(database_error.as_ref());
            map_database_error(info, number)
        }
        sqlx::Error::Io(error) => CoreError::Connection(error.to_string()),
        sqlx::Error::Tls(error) => CoreError::Tls(error.to_string()),
        sqlx::Error::PoolTimedOut => CoreError::Timeout("acquiring a pool connection"),
        sqlx::Error::PoolClosed => CoreError::Connection("the connection pool is closed".into()),
        sqlx::Error::Configuration(error) => CoreError::Configuration(error.to_string()),
        other => CoreError::Protocol(other.to_string()),
    }
}

fn map_database_error(info: DatabaseErrorInfo, number: Option<u16>) -> CoreError {
    // @NOTE: class 28 and the host-not-allowed error are authentication, class 08 means the
    // server went away.
    if info.sqlstate.starts_with("28") || matches!(number, Some(ACCESS_DENIED | HOST_NOT_ALLOWED)) {
        return CoreError::Authentication(info.message);
    }
    if error_means_session_lost(&info.sqlstate, number) {
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
    fn classifies_database_errors_by_sqlstate_and_number() {
        assert!(matches!(
            map_database_error(info("28000"), Some(1045)),
            CoreError::Authentication(_)
        ));
        assert!(matches!(
            map_database_error(info("HY000"), Some(1130)),
            CoreError::Authentication(_)
        ));
        assert!(matches!(
            map_database_error(info("08S01"), Some(1152)),
            CoreError::Connection(_)
        ));
        assert!(matches!(
            map_database_error(info("08S01"), Some(1053)),
            CoreError::Connection(_)
        ));
        assert!(matches!(
            map_database_error(info("42000"), Some(1064)),
            CoreError::Database(_)
        ));
        assert!(matches!(
            map_database_error(info("70100"), Some(1317)),
            CoreError::Database(_)
        ));
    }

    #[test]
    fn a_query_interruption_does_not_lose_the_session() {
        assert!(!error_means_session_lost("70100", Some(QUERY_INTERRUPTED)));
        assert!(error_means_session_lost("70100", Some(1927)));
        assert!(error_means_session_lost("08S01", None));
        assert!(is_interruption(Some(QUERY_INTERRUPTED)));
        assert!(is_interruption(Some(QUERY_TIMEOUT)));
        assert!(!is_interruption(Some(1064)));
        assert!(!is_interruption(None));
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
