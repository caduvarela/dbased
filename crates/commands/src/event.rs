use crate::{
    ColumnMeta, ConnectionId, ConnectionState, ConnectionSummary, ErrorInfo, ExecutionStatus,
    ExportStatus, MetadataItem, MetadataNode, RequestId, ResultId, ResultOutcome, SessionState,
    StatementSpan, TabId, Value,
};

/// Notifications the Core sends back to the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Full list, after load and every change.
    ConnectionsChanged {
        connections: Vec<ConnectionSummary>,
    },
    ConnectionStateChanged {
        id: ConnectionId,
        state: ConnectionState,
    },
    ConnectionSaved {
        request: RequestId,
        id: ConnectionId,
    },
    /// Terminal event of delete, test, connect and disconnect.
    RequestCompleted {
        request: RequestId,
    },
    RequestFailed {
        request: RequestId,
        error: ErrorInfo,
    },
    TabClosed {
        tab: TabId,
    },
    SessionStateChanged {
        tab: TabId,
        state: SessionState,
    },
    ExecutionStarted {
        request: RequestId,
        tab: TabId,
        statements: Vec<StatementSpan>,
    },
    ResultStarted {
        result: ResultId,
        columns: Vec<ColumnMeta>,
    },
    ResultProgress {
        result: ResultId,
        rows_buffered: u64,
    },
    ResultFinished {
        result: ResultId,
        outcome: ResultOutcome,
    },
    ExecutionFinished {
        request: RequestId,
        status: ExecutionStatus,
    },
    RowsWindow {
        result: ResultId,
        start: u64,
        rows: Vec<Vec<Value>>,
    },
    ChildrenLoaded {
        request: RequestId,
        connection: ConnectionId,
        node: MetadataNode,
        items: Vec<MetadataItem>,
    },
    /// Dropped when the event channel is full; `ExportFinished` carries the final counts.
    ExportProgress {
        request: RequestId,
        rows_written: u64,
        bytes_written: u64,
    },
    /// Terminal event of an export that did not fail.
    ExportFinished {
        request: RequestId,
        status: ExportStatus,
        rows_written: u64,
        bytes_written: u64,
    },
    /// Not tied to a request (startup, config file).
    Error {
        error: ErrorInfo,
    },
}
