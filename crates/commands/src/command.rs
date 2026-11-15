use crate::{
    ConnectionId, ConnectionSettings, ExecuteTarget, MetadataNode, PasswordSource, PasswordUpdate,
    RequestId, ResultId, TabId,
};

/// Requests the UI sends to the Core. No `PartialEq`: secrets cannot be compared.
#[derive(Debug, Clone)]
pub enum Command {
    SaveConnection {
        request: RequestId,
        id: Option<ConnectionId>,
        settings: ConnectionSettings,
        password: PasswordUpdate,
    },
    DeleteConnection {
        request: RequestId,
        id: ConnectionId,
    },
    TestConnection {
        request: RequestId,
        id: Option<ConnectionId>,
        settings: ConnectionSettings,
        password: PasswordSource,
    },
    Connect {
        request: RequestId,
        id: ConnectionId,
        password: PasswordSource,
    },
    Disconnect {
        request: RequestId,
        id: ConnectionId,
    },
    OpenTab {
        tab: TabId,
        connection: ConnectionId,
    },
    CloseTab {
        tab: TabId,
    },
    Execute {
        request: RequestId,
        tab: TabId,
        script: String,
        target: ExecuteTarget,
    },
    /// `request` is the `Execute` request to stop.
    Cancel {
        tab: TabId,
        request: RequestId,
    },
    FetchRows {
        result: ResultId,
        start: u64,
        count: u32,
    },
    ReleaseResult {
        result: ResultId,
    },
    LoadChildren {
        request: RequestId,
        connection: ConnectionId,
        node: MetadataNode,
    },
}
