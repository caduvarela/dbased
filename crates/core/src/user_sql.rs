use std::fmt;

/// SQL text typed by the user. It is the only SQL a driver may execute as-is
/// (docs/PROJECT.md 1.2); statements DBased assembles never become `UserSql`.
#[derive(Clone, PartialEq, Eq)]
pub struct UserSql(String);

impl UserSql {
    pub fn new(sql: impl Into<String>) -> Self {
        Self(sql.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// @NOTE: user SQL can carry secrets (`ALTER ROLE ... PASSWORD`), so it is never printed.
impl fmt::Debug for UserSql {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "UserSql({} bytes)", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_print_the_sql() {
        let sql = UserSql::new("ALTER ROLE app PASSWORD 'hunter2'");
        assert!(!format!("{sql:?}").contains("hunter2"));
    }

    #[test]
    fn keeps_the_text_untouched() {
        let sql = UserSql::new("  select 1 ;  ");
        assert_eq!(sql.as_str(), "  select 1 ;  ");
    }
}
