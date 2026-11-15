use crate::block::BlockTracker;
use crate::scanner::Scanner;
use crate::token::{Dialect, Span, Token, TokenKind};

/// One statement of a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Statement {
    /// Trimmed range without the terminator; leading and inner comments are kept.
    pub span: Span,
    /// The `;` (or custom delimiter) that ended it; `None` for a trailing statement.
    pub terminator: Option<Span>,
}

impl Statement {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        self.span.text(source)
    }
}

/// Splits a script on top-level terminators, skipping statements that hold only
/// whitespace or comments. Works on half-typed SQL: an unterminated string swallows the rest.
pub fn split_statements(source: &str, dialect: Dialect) -> Vec<Statement> {
    let mut scanner = Scanner::new(source, dialect);
    let mut statements = Vec::new();
    let mut current = Current::new(dialect);
    while let Some(token) = scanner.next() {
        if token.kind == TokenKind::Whitespace {
            continue;
        }
        let directive = (dialect == Dialect::MySql && current.is_blank())
            .then(|| delimiter_directive(source, &token))
            .flatten();
        if let Some(directive) = directive {
            current = Current::new(dialect);
            scanner.skip_to(directive.line_end);
            scanner.set_delimiter(directive.delimiter);
            continue;
        }
        let ends_statement = match token.kind {
            TokenKind::Delimiter => true,
            TokenKind::Punctuation => {
                token.text(source) == ";"
                    && !scanner.has_custom_delimiter()
                    && !current.tracker.in_block()
            }
            _ => false,
        };
        if ends_statement {
            statements.extend(current.finish(Some(token.span)));
            current = Current::new(dialect);
        } else {
            current.push(&token, source);
        }
    }
    statements.extend(current.finish(None));
    statements
}

/// The statement a cursor belongs to: the last one that starts at or before the cursor (so a
/// cursor inside it, on its terminator or in the gap after it all pick it), else the first one
/// after the cursor. `None` only when there are no statements.
pub fn statement_at(statements: &[Statement], cursor: usize) -> Option<&Statement> {
    let starting_before = statements
        .iter()
        .rev()
        .find(|statement| statement.span.start <= cursor);
    starting_before.or_else(|| statements.first())
}

struct Current {
    start: Option<usize>,
    end: usize,
    significant: bool,
    tracker: BlockTracker,
}

impl Current {
    fn new(dialect: Dialect) -> Self {
        Self {
            start: None,
            end: 0,
            significant: false,
            tracker: BlockTracker::new(dialect),
        }
    }

    fn is_blank(&self) -> bool {
        !self.significant
    }

    fn push(&mut self, token: &Token, source: &str) {
        self.start.get_or_insert(token.span.start);
        self.end = token.span.end;
        if !token.kind.is_trivia() {
            self.significant = true;
            self.tracker.feed(token, source);
        }
    }

    fn finish(self, terminator: Option<Span>) -> Option<Statement> {
        let start = self.start?;
        if !self.significant {
            return None;
        }
        Some(Statement {
            span: Span {
                start,
                end: self.end,
            },
            terminator,
        })
    }
}

struct DelimiterDirective {
    /// `None` means back to the default `;`.
    delimiter: Option<String>,
    line_end: usize,
}

// @NOTE: `DELIMITER` is a client-side command of the mysql client; it is honored only at the
// start of a line and is never part of a statement sent to the server.
fn delimiter_directive(source: &str, token: &Token) -> Option<DelimiterDirective> {
    if token.kind != TokenKind::Word || !token.text(source).eq_ignore_ascii_case("DELIMITER") {
        return None;
    }
    let line_start = source[..token.span.start].rfind('\n').map_or(0, |i| i + 1);
    if !source[line_start..token.span.start].trim().is_empty() {
        return None;
    }
    let line_end = source[token.span.end..]
        .find('\n')
        .map_or(source.len(), |i| token.span.end + i);
    let argument = source[token.span.end..line_end].split_whitespace().next()?;
    let delimiter = (argument != ";").then(|| argument.to_string());
    Some(DelimiterDirective {
        delimiter,
        line_end,
    })
}
