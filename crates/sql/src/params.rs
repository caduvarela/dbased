use crate::scanner::Scanner;
use crate::token::{Dialect, ParameterKind, Span, TokenKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parameter {
    pub kind: ParameterKind,
    /// Covers the whole marker: `?`, `$1` or `:name` (colon included).
    pub span: Span,
}

impl Parameter {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        self.span.text(source)
    }

    /// The name without the colon for `:name`; `None` for the other kinds.
    pub fn name<'a>(&self, source: &'a str) -> Option<&'a str> {
        match self.kind {
            ParameterKind::Named => self.text(source).strip_prefix(':'),
            _ => None,
        }
    }
}

/// Lists parameter markers in source order, ignoring strings, comments, dollar quotes,
/// `::` casts and `:name` inside `[...]`. A caller decides what to do with each kind:
/// `?` is a jsonb operator on Postgres, so anonymous markers are opt-in there (9.3).
pub fn find_parameters(source: &str, dialect: Dialect) -> Vec<Parameter> {
    Scanner::new(source, dialect)
        .filter_map(|token| match token.kind {
            TokenKind::Parameter(kind) => Some(Parameter {
                kind,
                span: token.span,
            }),
            _ => None,
        })
        .collect()
}
