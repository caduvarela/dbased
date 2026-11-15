use serde::{Deserialize, Serialize};

/// Identifies one unit of asynchronous work so events can be matched to requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(pub u64);

/// Placeholder connection description; secrets never travel here, only a reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub name: String,
    pub url: String,
}

/// Requests the UI sends to the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    OpenConnection {
        job: JobId,
        config: ConnectionConfig,
    },
    ExecuteQuery {
        job: JobId,
        sql: String,
    },
    CancelQuery {
        job: JobId,
    },
}
