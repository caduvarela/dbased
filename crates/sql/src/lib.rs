//! Pure SQL text handling for Postgres and MySQL: a lossless tolerant scanner, statement
//! splitting and parameter marker detection. No I/O, no async, no UI.

mod block;
mod params;
mod scanner;
mod split;
mod token;

pub use params::{Parameter, find_parameters};
pub use scanner::{Scanner, tokenize};
pub use split::{Statement, split_statements, statement_at};
pub use token::{Dialect, ParameterKind, Span, Token, TokenKind};

#[cfg(test)]
mod tests;
