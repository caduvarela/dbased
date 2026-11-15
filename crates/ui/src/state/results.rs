use dbased_commands::{ColumnMeta, ResultId, ResultOutcome, ResultStatus, TabId};

use super::format::format_count;

/// One result set as the UI tracks it. Rows are not here: the grid pulls them in windows.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultModel {
    pub id: ResultId,
    pub tab: TabId,
    pub columns: Vec<ColumnMeta>,
    pub rows_buffered: u64,
    /// `None` while the statement is still running.
    pub outcome: Option<ResultOutcome>,
}

impl ResultModel {
    pub fn new(id: ResultId, tab: TabId, columns: Vec<ColumnMeta>) -> Self {
        Self {
            id,
            tab,
            columns,
            rows_buffered: 0,
            outcome: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.outcome.is_none()
    }

    pub fn status(&self) -> Option<ResultStatus> {
        self.outcome.as_ref().map(|outcome| outcome.status)
    }

    /// Statements without columns (DDL, errors, cancelled starts) show a message, not a grid.
    pub fn has_grid(&self) -> bool {
        !self.columns.is_empty()
    }

    /// Rows the grid may ask for.
    pub fn row_count(&self) -> u64 {
        match &self.outcome {
            Some(outcome) => outcome.rows_received,
            None => self.rows_buffered,
        }
    }

    /// 1-based, as shown to the user.
    pub fn title(&self) -> String {
        format!("Result {}", self.id.statement + 1)
    }

    pub fn status_line(&self) -> String {
        let Some(outcome) = &self.outcome else {
            return format!("Running... {} rows", format_count(self.rows_buffered));
        };
        let rows = format_count(outcome.rows_received);
        match outcome.status {
            ResultStatus::Completed => completed_line(outcome, &rows, self.has_grid()),
            ResultStatus::Truncated => format!("{rows} rows (truncated at the row limit)"),
            ResultStatus::Cancelled => format!("Cancelled after {rows} rows"),
            ResultStatus::Failed => failed_line(outcome),
        }
    }
}

fn completed_line(outcome: &ResultOutcome, rows: &str, has_grid: bool) -> String {
    if has_grid {
        return format!("{rows} rows");
    }
    match outcome.rows_affected {
        Some(affected) => format!("OK, {} rows affected", format_count(affected)),
        None => "OK".to_string(),
    }
}

fn failed_line(outcome: &ResultOutcome) -> String {
    let Some(error) = &outcome.error else {
        return "Failed".to_string();
    };
    match error.position {
        Some(position) => format!(
            "Failed: {} (SQLSTATE {}, position {position})",
            error.message, error.sqlstate
        ),
        None => format!("Failed: {} (SQLSTATE {})", error.message, error.sqlstate),
    }
}

#[cfg(test)]
mod tests {
    use dbased_commands::{DatabaseError, RequestId};

    use super::*;

    fn model(columns: usize) -> ResultModel {
        let columns = (0..columns)
            .map(|index| ColumnMeta {
                name: format!("c{index}"),
                type_name: "text".to_string(),
                nullable: None,
            })
            .collect();
        ResultModel::new(
            ResultId {
                request: RequestId(1),
                statement: 1,
            },
            TabId(1),
            columns,
        )
    }

    fn outcome(status: ResultStatus, rows: u64) -> ResultOutcome {
        ResultOutcome {
            status,
            rows_received: rows,
            rows_affected: None,
            partial: false,
            error: None,
            cancel_error: None,
        }
    }

    #[test]
    fn a_running_result_follows_the_progress() {
        let mut result = model(2);
        result.rows_buffered = 1_500;
        assert!(result.is_running());
        assert_eq!(result.row_count(), 1_500);
        assert_eq!(result.status_line(), "Running... 1,500 rows");
    }

    #[test]
    fn a_finished_result_uses_the_final_row_count() {
        let mut result = model(2);
        result.rows_buffered = 10;
        result.outcome = Some(outcome(ResultStatus::Completed, 12));
        assert_eq!(result.row_count(), 12);
        assert_eq!(result.status_line(), "12 rows");
    }

    #[test]
    fn titles_are_one_based() {
        assert_eq!(model(1).title(), "Result 2");
    }

    #[test]
    fn statements_without_columns_say_what_they_changed() {
        let mut result = model(0);
        let mut done = outcome(ResultStatus::Completed, 0);
        done.rows_affected = Some(3);
        result.outcome = Some(done);
        assert!(!result.has_grid());
        assert_eq!(result.status_line(), "OK, 3 rows affected");
    }

    #[test]
    fn truncated_and_cancelled_results_say_so() {
        let mut result = model(1);
        result.outcome = Some(outcome(ResultStatus::Truncated, 5_000_000));
        assert_eq!(
            result.status_line(),
            "5,000,000 rows (truncated at the row limit)"
        );
        result.outcome = Some(outcome(ResultStatus::Cancelled, 40));
        assert_eq!(result.status_line(), "Cancelled after 40 rows");
    }

    #[test]
    fn a_failure_names_the_error_and_position() {
        let mut result = model(0);
        let mut failed = outcome(ResultStatus::Failed, 0);
        failed.error = Some(DatabaseError {
            sqlstate: "42601".to_string(),
            message: "syntax error".to_string(),
            detail: None,
            hint: None,
            position: Some(6),
        });
        result.outcome = Some(failed);
        assert_eq!(
            result.status_line(),
            "Failed: syntax error (SQLSTATE 42601, position 6)"
        );
    }
}
