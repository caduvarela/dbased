use std::time::Duration;

use sqlx::MySqlConnection;
use tokio::time::timeout;

use super::errors::{TRANSACTION_IN_PROGRESS, error_number};
use super::sql::leading_keyword;

/// Statements that never begin or end a transaction on their own. Anything else (BEGIN, COMMIT,
/// CALL, XA, SAVEPOINT, ...) makes the state unknown until the next probe.
const STATE_PRESERVING_KEYWORDS: &[&str] = &[
    "select", "insert", "update", "delete", "replace", "with", "values", "table", "show",
    "explain", "describe", "desc", "use", "set",
];

/// Statements that implicitly COMMIT an open transaction and never open one. Outside a
/// transaction they keep the state; inside one they end it.
const IMPLICIT_COMMIT_KEYWORDS: &[&str] = &[
    "create", "alter", "drop", "truncate", "rename", "grant", "revoke", "lock", "unlock",
    "optimize", "analyze", "repair", "flush",
];

/// MySQL reports no transaction timestamp, and sqlx keeps the server status flag
/// (`SERVER_STATUS_IN_TRANS`) private. The one side-effect-light statement that fails only
/// inside a transaction is `SET TRANSACTION`, which the server rejects with error 1568 while a
/// transaction is active (an `autocommit=0` session counts once it touched a table).
///
/// @NOTE: it sets the characteristics of the NEXT transaction, so it re-states the session's
/// current read-only default; a plain `READ WRITE` would silently undo a user's
/// `SET SESSION TRANSACTION READ ONLY`. The text is static: the prepared statement only
/// chooses between two literals, no user input reaches it.
const PROBE_SQL: &str = "SET @dbased_probe = IF(@@session.transaction_read_only, \
                         'SET TRANSACTION READ ONLY', 'SET TRANSACTION READ WRITE'); \
                         PREPARE dbased_probe FROM @dbased_probe; \
                         EXECUTE dbased_probe; \
                         DEALLOCATE PREPARE dbased_probe";

/// Whether the session sits inside a user transaction. Tracked from the statement text and
/// resolved by a probe whenever the text cannot prove the answer (docs/PROJECT.md 3.2.4).
///
/// @NOTE: unlike Postgres, a killed MySQL statement never aborts the surrounding transaction,
/// but a Core-initiated stop still discards instead of killing inside a transaction: a killed
/// `CALL` would leave the user's work half done, while a discard lets it finish as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TransactionState {
    Idle,
    InTransaction,
    /// A statement may have changed the state and the probe has not run yet.
    Unknown,
}

#[derive(Debug)]
pub(super) struct TransactionTracker {
    state: TransactionState,
    /// Once a statement mentions `autocommit`, any statement may open a transaction (with
    /// `autocommit=0` the first table access does), so the text alone proves nothing anymore.
    autocommit_in_play: bool,
}

impl TransactionTracker {
    pub(super) fn new() -> Self {
        Self {
            state: TransactionState::Idle,
            autocommit_in_play: false,
        }
    }

    /// Unknown counts as inside a transaction: skipping a kill only costs time, killing
    /// could cut the user's work in half.
    pub(super) fn protects_user_work(&self) -> bool {
        self.state != TransactionState::Idle
    }

    pub(super) fn needs_probe(&self) -> bool {
        self.state == TransactionState::Unknown
    }

    pub(super) fn resolve(&mut self, state: TransactionState) {
        self.state = state;
    }

    /// The state after `sql` ran, without a round trip when the text proves it cannot matter.
    /// `failed` is true when the server answered with an error (a deadlock, for one, rolls the
    /// whole transaction back).
    pub(super) fn after_statement(&mut self, sql: &str, failed: bool) {
        if mentions_autocommit(sql) {
            self.autocommit_in_play = true;
        }
        self.state = self.next_state(sql, failed);
    }

    fn next_state(&self, sql: &str, failed: bool) -> TransactionState {
        let current = self.state;
        if self.autocommit_in_play || has_inner_semicolon(sql) {
            return TransactionState::Unknown;
        }
        if failed && current != TransactionState::Idle {
            return TransactionState::Unknown;
        }
        let keyword = leading_keyword(sql);
        if STATE_PRESERVING_KEYWORDS.contains(&keyword.as_str()) {
            return current;
        }
        if IMPLICIT_COMMIT_KEYWORDS.contains(&keyword.as_str()) && current == TransactionState::Idle
        {
            return current;
        }
        TransactionState::Unknown
    }
}

fn mentions_autocommit(sql: &str) -> bool {
    sql.to_ascii_lowercase().contains("autocommit")
}

fn has_inner_semicolon(sql: &str) -> bool {
    let trimmed = sql.trim_end().trim_end_matches(';');
    trimmed.contains(';')
}

/// Asks the server. `Err` means the connection itself failed and must be released.
pub(super) async fn probe_transaction_state(
    connection: &mut MySqlConnection,
    limit: Duration,
) -> Result<TransactionState, ()> {
    let probing = sqlx::raw_sql(PROBE_SQL).execute(&mut *connection);
    match timeout(limit, probing).await {
        Ok(Ok(_)) => Ok(TransactionState::Idle),
        Ok(Err(sqlx::Error::Database(error))) => {
            if error_number(error.as_ref()) == Some(TRANSACTION_IN_PROGRESS) {
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

    fn tracker_in(state: TransactionState) -> TransactionTracker {
        let mut tracker = TransactionTracker::new();
        tracker.resolve(state);
        tracker
    }

    fn state_after(state: TransactionState, sql: &str, failed: bool) -> TransactionState {
        let mut tracker = tracker_in(state);
        tracker.after_statement(sql, failed);
        tracker.state
    }

    #[test]
    fn plain_statements_keep_the_state() {
        for state in [TransactionState::Idle, TransactionState::InTransaction] {
            for sql in [
                "SELECT 1",
                "  select 1;",
                "-- c\nUPDATE t SET a = 1;  ",
                "with x as (select 1) select * from x",
                "set transaction isolation level serializable",
                "# c\nselect 1",
            ] {
                assert_eq!(state_after(state, sql, false), state, "{sql}");
            }
        }
    }

    #[test]
    fn ddl_keeps_idle_but_ends_a_transaction() {
        assert_eq!(
            state_after(TransactionState::Idle, "create table t(a int)", false),
            TransactionState::Idle
        );
        assert_eq!(
            state_after(TransactionState::InTransaction, "drop table t", false),
            TransactionState::Unknown
        );
    }

    #[test]
    fn transaction_control_and_scripts_make_the_state_unknown() {
        for sql in [
            "BEGIN",
            "commit",
            "ROLLBACK TO SAVEPOINT s",
            "start transaction",
            "CALL p()",
            "XA START 'a'",
            "select 1; commit",
            "",
            "-- only a comment",
        ] {
            assert_eq!(
                state_after(TransactionState::Idle, sql, false),
                TransactionState::Unknown,
                "{sql}"
            );
        }
    }

    #[test]
    fn a_failure_inside_a_transaction_is_unknown() {
        assert_eq!(
            state_after(TransactionState::InTransaction, "update t set a = 1", true),
            TransactionState::Unknown
        );
        assert_eq!(
            state_after(TransactionState::Idle, "update t set a = 1", true),
            TransactionState::Idle
        );
    }

    #[test]
    fn touching_autocommit_makes_every_statement_unknown() {
        for sql in ["set autocommit = 0", "SET @@SESSION.AUTOCOMMIT=1"] {
            let mut tracker = tracker_in(TransactionState::Idle);
            tracker.after_statement(sql, false);
            assert_eq!(tracker.state, TransactionState::Unknown, "{sql}");
            tracker.resolve(TransactionState::Idle);
            tracker.after_statement("select 1", false);
            assert_eq!(tracker.state, TransactionState::Unknown, "{sql}");
        }
    }

    #[test]
    fn unknown_is_protective_and_sticky() {
        assert!(tracker_in(TransactionState::Unknown).protects_user_work());
        assert!(!tracker_in(TransactionState::Idle).protects_user_work());
        assert_eq!(
            state_after(TransactionState::Unknown, "select 1", false),
            TransactionState::Unknown
        );
    }
}
