use async_trait::async_trait;
use dbased_commands::{ConnectionConfig, Value};

use crate::CoreError;

/// Backend-side identifier of a session (e.g. a PID), used to cancel running queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaInfo {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableInfo {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewInfo {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineInfo {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRequest {
    pub limit: u64,
    pub offset: u64,
}

#[async_trait]
pub trait Driver: Send + Sync {
    async fn connect(config: &ConnectionConfig) -> Result<Self, CoreError>
    where
        Self: Sized;

    async fn execute_query(&self, sql: &str) -> Result<QueryResult, CoreError>;
    async fn session_id(&self) -> Result<SessionId, CoreError>;
    async fn cancel_query(&self, session: &SessionId) -> Result<(), CoreError>;

    async fn list_schemas(&self) -> Result<Vec<SchemaInfo>, CoreError>;
    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>, CoreError>;
    async fn list_columns(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>, CoreError>;
    async fn list_views(&self, schema: &str) -> Result<Vec<ViewInfo>, CoreError>;
    async fn list_routines(&self, schema: &str) -> Result<Vec<RoutineInfo>, CoreError>;

    /// Quotes an identifier using the dialect's rules.
    fn quote_identifier(&self, identifier: &str) -> String;
    /// Wraps a query so only the requested page is returned.
    fn paginate(&self, sql: &str, page: PageRequest) -> String;
}
