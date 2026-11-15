//! Contract between the UI and the core: commands going in, events coming out.
//! This crate depends on neither of them.

mod bridge;
mod command;
mod dto;
mod error;
mod event;
mod ids;
mod value;

pub use bridge::{CoreLink, UiLink, bridge};
pub use command::Command;
pub use dto::{
    ConnectionSettings, ConnectionState, ConnectionSummary, DatabaseKind, ExecuteTarget,
    ExecutionStatus, MetadataItem, MetadataItemKind, MetadataNode, PasswordSource, PasswordUpdate,
    ResultOutcome, ResultStatus, SessionResetReason, SessionState, StatementSpan, TlsMode,
};
pub use error::{DatabaseError, ErrorInfo, ErrorKind};
pub use event::Event;
pub use ids::{ConnectionId, RequestId, ResultId, TabId};
pub use value::{ColumnMeta, Value};

/// Channel capacities from docs/PROJECT.md 11.1.
pub const COMMAND_CAPACITY: usize = 256;
pub const EVENT_CAPACITY: usize = 1024;

#[cfg(test)]
mod tests;
