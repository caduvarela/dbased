use sqlx::AssertSqlSafe;

use crate::UserSql;

/// The single place user SQL becomes executable text. SQL assembled by DBased
/// must never pass through here.
pub(super) fn assert_user_sql(sql: &UserSql) -> AssertSqlSafe<String> {
    AssertSqlSafe(sql.as_str().to_owned())
}

/// The first keyword of a statement, lowercased, skipping blanks, comments and parentheses.
pub(super) fn leading_keyword(sql: &str) -> String {
    let mut rest = sql;
    loop {
        rest = rest
            .trim_start_matches(|character: char| character.is_whitespace() || character == '(');
        if let Some(after) = rest.strip_prefix("--") {
            rest = after.split_once('\n').map_or("", |(_, tail)| tail);
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, tail)| tail);
        } else {
            break;
        }
    }
    rest.chars()
        .take_while(|character| character.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Cheap guess whether a statement returns rows, used only to decide if a
/// prepare is worth it to get headers for an empty result (docs/PROJECT.md 2.3).
pub(super) fn looks_row_returning(sql: &str) -> bool {
    matches!(
        leading_keyword(sql).as_str(),
        "select" | "with" | "values" | "table" | "show" | "explain" | "fetch"
    )
}

#[cfg(test)]
mod tests {
    use sqlx::SqlSafeStr;

    use super::*;

    #[test]
    fn conversion_keeps_the_text_exactly() {
        let sql = UserSql::new("select 'a'; -- tail");
        assert_eq!(
            assert_user_sql(&sql).into_sql_str().as_str(),
            "select 'a'; -- tail"
        );
    }

    #[test]
    fn detects_row_returning_statements() {
        for sql in [
            "select 1",
            "  SELECT 1",
            "(select 1)",
            "-- note\nwith x as (select 1) select * from x",
            "/* c */ values (1)",
            "show search_path",
        ] {
            assert!(looks_row_returning(sql), "{sql}");
        }
    }

    #[test]
    fn rejects_statements_without_rows() {
        for sql in [
            "insert into t values (1)",
            "create table t()",
            "",
            "-- only comment",
        ] {
            assert!(!looks_row_returning(sql), "{sql}");
        }
    }
}
