use std::time::Duration;

use sqlx::{PgConnection, Row as _};
use tokio::time::timeout;

use super::errors::TRANSACTION_ABORTED;
use super::sql::leading_keyword;

/// Statements that can never begin or end a transaction block on their own. Anything else
/// (BEGIN, COMMIT, DO, CALL, EXECUTE, ...) makes the state unknown until the next probe.
const STATE_PRESERVING_KEYWORDS: &[&str] = &[
    "select", "insert", "update", "delete", "with", "values", "table", "show", "explain", "merge",
    "set", "reset", "create", "alter", "drop", "truncate", "grant", "revoke", "comment", "lock",
];

/// `transaction_timestamp()` is fixed when a transaction starts, while `statement_timestamp()`
/// is the arrival time of the current command. A simple-protocol command outside a transaction
/// block starts its own transaction, so the two are equal; inside a block they differ.
/// @NOTE: it must run through the simple protocol; the extended one stamps Parse and Execute
/// separately and would always differ.
const PROBE_SQL: &str = "SELECT CASE WHEN transaction_timestamp() = statement_timestamp() \
                         THEN 0 ELSE 1 END";

/// Whether the session sits inside a user transaction block. sqlx keeps the server's
/// ReadyForQuery status private, so the driver tracks it (docs/PROJECT.md 3.2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TransactionState {
    Idle,
    InTransaction,
    /// A statement may have changed the state and the probe has not run yet.
    Unknown,
}

impl TransactionState {
    /// Unknown counts as inside a transaction: skipping a cancel only costs time, cancelling
    /// would abort the user's work.
    pub(super) fn protects_user_work(self) -> bool {
        self != Self::Idle
    }

    pub(super) fn needs_probe(self) -> bool {
        self == Self::Unknown
    }

    /// The state after `sql` ran, without a round trip when the text proves it cannot matter.
    pub(super) fn after_statement(self, sql: &str) -> Self {
        let single_statement = !has_inner_semicolon(sql);
        let keyword = leading_keyword(sql);
        if single_statement && STATE_PRESERVING_KEYWORDS.contains(&keyword.as_str()) {
            return self;
        }
        Self::Unknown
    }
}

fn has_inner_semicolon(sql: &str) -> bool {
    let trimmed = sql.trim_end().trim_end_matches(';');
    trimmed.contains(';')
}

/// Asks the server. `Err` means the connection itself failed and must be released.
pub(super) async fn probe_transaction_state(
    connection: &mut PgConnection,
    limit: Duration,
) -> Result<TransactionState, ()> {
    let probing = sqlx::raw_sql(PROBE_SQL).fetch_one(&mut *connection);
    match timeout(limit, probing).await {
        Ok(Ok(row)) => match row.try_get::<i32, _>(0) {
            Ok(0) => Ok(TransactionState::Idle),
            Ok(_) => Ok(TransactionState::InTransaction),
            Err(_) => Ok(TransactionState::Unknown),
        },
        Ok(Err(sqlx::Error::Database(error))) => {
            if error.code().as_deref() == Some(TRANSACTION_ABORTED) {
                return Ok(TransactionState::InTransaction);
            }
            Ok(TransactionState::Unknown)
        }
        Ok(Err(_)) | Err(_) => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_statements_keep_the_state() {
        for state in [TransactionState::Idle, TransactionState::InTransaction] {
            for sql in [
                "SELECT 1",
                "  select 1;",
                "-- c\nUPDATE t SET a = 1;  ",
                "with x as (select 1) select * from x",
                "create table t()",
            ] {
                assert_eq!(state.after_statement(sql), state, "{sql}");
            }
        }
    }

    #[test]
    fn transaction_control_and_scripts_make_the_state_unknown() {
        for sql in [
            "BEGIN",
            "commit",
            "ROLLBACK TO SAVEPOINT s",
            "start transaction",
            "DO $$ BEGIN COMMIT; END $$",
            "CALL p()",
            "EXECUTE s",
            "select 1; commit",
            "",
            "-- only a comment",
        ] {
            assert_eq!(
                TransactionState::Idle.after_statement(sql),
                TransactionState::Unknown,
                "{sql}"
            );
        }
    }

    #[test]
    fn unknown_is_protective_and_sticky() {
        assert!(TransactionState::Unknown.protects_user_work());
        assert!(!TransactionState::Idle.protects_user_work());
        assert_eq!(
            TransactionState::Unknown.after_statement("select 1"),
            TransactionState::Unknown
        );
    }
}
