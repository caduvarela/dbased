//! Database logic. Must never depend on any UI crate.

mod driver;
mod error;

pub use driver::{
    ColumnInfo, Driver, PageRequest, QueryResult, RoutineInfo, SchemaInfo, SessionId, TableInfo,
    ViewInfo,
};
pub use error::CoreError;
