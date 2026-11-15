//! Database logic. Must never depend on any UI crate.

mod driver;
pub mod drivers;
mod error;
mod service;
mod user_sql;

pub use driver::{
    CancelHandle, CancelOutcome, CancelReason, Canceller, CloseReason, Column, Driver,
    DriverFactory, PageRequest, Routine, RowSink, RowSinkMessage, Schema, Session, SessionHealth,
    SessionId, SessionLimits, StatementOutcome, StatementStatus, TableRef, ViewRef,
};
pub use error::{CoreError, DatabaseErrorInfo};
pub use service::{
    ConnectionStore, Core, CoreDependencies, DefaultConnector, DriverConnector,
    InMemorySecretStore, SecretStore, StoredConnection, error_info,
};
pub use user_sql::UserSql;
