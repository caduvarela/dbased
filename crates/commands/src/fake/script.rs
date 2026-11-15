use dbased_sql::{Dialect, split_statements};

use crate::{DatabaseKind, ExecuteTarget};

/// What a statement's text asks the fake to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    Select { rows: u64 },
    Fail { position: u32 },
    Sleep,
    Other,
}

pub fn parse_cue(statement: &str) -> Cue {
    let mut words = statement.split_whitespace();
    let first = words.next().unwrap_or("");
    if first.eq_ignore_ascii_case("select") {
        let rows = words.next().and_then(|word| word.parse::<u64>().ok());
        return rows.map_or(Cue::Other, |rows| Cue::Select { rows });
    }
    if first.eq_ignore_ascii_case("fail") {
        return Cue::Fail {
            position: fail_position(statement),
        };
    }
    if first.eq_ignore_ascii_case("sleep") {
        return Cue::Sleep;
    }
    Cue::Other
}

/// 1-based character offset of the word after `fail`, or 1 when there is none.
fn fail_position(statement: &str) -> u32 {
    let after_keyword = statement.trim_start().get(4..).unwrap_or("").trim_start();
    if after_keyword.is_empty() {
        return 1;
    }
    let prefix_chars = statement.chars().count() - after_keyword.chars().count();
    (prefix_chars + 1) as u32
}

/// A statement of the script: byte range into the whole script, plus its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptStatement {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

fn dialect_of(kind: DatabaseKind) -> Dialect {
    match kind {
        DatabaseKind::Postgres => Dialect::Postgres,
        DatabaseKind::MySql => Dialect::MySql,
    }
}

fn split_at_offset(
    script: &str,
    offset: usize,
    dialect: Dialect,
) -> Vec<(ScriptStatement, Option<usize>)> {
    split_statements(script, dialect)
        .into_iter()
        .map(|statement| {
            let terminator_end = statement.terminator.map(|span| span.end + offset);
            let resolved = ScriptStatement {
                start: statement.span.start + offset,
                end: statement.span.end + offset,
                text: statement.text(script).to_string(),
            };
            (resolved, terminator_end)
        })
        .collect()
}

/// The statements `target` selects; empty means nothing to run.
pub fn resolve_statements(
    script: &str,
    target: ExecuteTarget,
    kind: DatabaseKind,
) -> Vec<ScriptStatement> {
    let dialect = dialect_of(kind);
    match target {
        ExecuteTarget::All => split_at_offset(script, 0, dialect)
            .into_iter()
            .map(|(statement, _)| statement)
            .collect(),
        ExecuteTarget::Selection { start, end } => {
            let Some(selected) = script.get(start..end) else {
                return Vec::new();
            };
            split_at_offset(selected, start, dialect)
                .into_iter()
                .map(|(statement, _)| statement)
                .collect()
        }
        ExecuteTarget::Current { cursor } => {
            let statements = split_at_offset(script, 0, dialect);
            statement_at(statements, cursor).into_iter().collect()
        }
    }
}

/// The statement holding `cursor` (terminator included), else the nearest one before it.
fn statement_at(
    statements: Vec<(ScriptStatement, Option<usize>)>,
    cursor: usize,
) -> Option<ScriptStatement> {
    let containing = statements.iter().position(|(statement, terminator_end)| {
        let end = terminator_end.unwrap_or(statement.end);
        statement.start <= cursor && cursor <= end
    });
    let before = statements
        .iter()
        .rposition(|(statement, _)| statement.end <= cursor);
    let index = containing
        .or(before)
        .or(if statements.is_empty() { None } else { Some(0) })?;
    statements
        .into_iter()
        .nth(index)
        .map(|(statement, _)| statement)
}
