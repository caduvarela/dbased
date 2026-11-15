//! Pure SQL text handling for Postgres and MySQL: a lossless tolerant scanner, statement
//! splitting and parameter marker detection. No I/O, no async, no UI.

mod scanner;
mod token;

pub use scanner::{Scanner, tokenize};
pub use token::{Dialect, ParameterKind, Span, Token, TokenKind};
