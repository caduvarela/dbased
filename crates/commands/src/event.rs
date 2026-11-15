use serde::{Deserialize, Serialize};

use crate::{JobId, Value};

/// Notifications the core sends back to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Event {
    QueryStarted { job: JobId },
    RowsReceived { job: JobId, rows: Vec<Vec<Value>> },
    QueryFinished { job: JobId },
    Error { job: JobId, message: String },
}
