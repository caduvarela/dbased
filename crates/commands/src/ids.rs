/// Persistent connection id, allocated by the Core (docs/PROJECT.md 11.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnectionId(pub u64);

/// Tab id, allocated by the UI from a process-wide counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

/// Request id, allocated by the UI; every request ends with exactly one terminal event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub u64);

/// Derived from the request that produced it plus the statement's index in the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResultId {
    pub request: RequestId,
    pub statement: u32,
}
