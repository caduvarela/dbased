//! Contract between the UI and the core: commands going in, events coming out.
//! This crate depends on neither of them.

mod command;
mod event;
mod value;

pub use command::{Command, ConnectionConfig, JobId};
pub use event::Event;
pub use value::{ColumnMeta, Value};
