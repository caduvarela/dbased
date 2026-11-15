use dbased_commands::{
    ConnectionId, ConnectionState, ExecutionStatus, RequestId, ResultId, SessionResetReason,
    SessionState, StatementSpan, TabId,
};

/// Where a tab is in its run cycle. The request is the `Execute` being served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    /// `Execute` sent, `ExecutionStarted` not yet seen.
    Starting(RequestId),
    Running(RequestId),
    /// `Cancel` sent; the execution still ends with its own `ExecutionFinished`.
    Cancelling(RequestId),
}

impl Activity {
    pub fn request(&self) -> Option<RequestId> {
        match self {
            Activity::Idle => None,
            Activity::Starting(request)
            | Activity::Running(request)
            | Activity::Cancelling(request) => Some(*request),
        }
    }

    pub fn is_busy(&self) -> bool {
        !matches!(self, Activity::Idle)
    }
}

/// How the last execution of a tab ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LastRun {
    Completed,
    StoppedOnError { statement: u32 },
    Cancelled { statement: u32 },
    Failed { message: String },
}

impl LastRun {
    pub fn from_status(status: &ExecutionStatus) -> Self {
        match status {
            ExecutionStatus::Completed => LastRun::Completed,
            ExecutionStatus::StoppedOnError { statement } => LastRun::StoppedOnError {
                statement: *statement,
            },
            ExecutionStatus::Cancelled { statement } => LastRun::Cancelled {
                statement: *statement,
            },
            ExecutionStatus::Aborted { error, .. } => LastRun::Failed {
                message: error.message.clone(),
            },
        }
    }

    pub fn label(&self) -> String {
        match self {
            LastRun::Completed => "Done".to_string(),
            LastRun::StoppedOnError { statement } => {
                format!("Stopped at statement {}", statement + 1)
            }
            LastRun::Cancelled { statement } => {
                format!("Cancelled at statement {}", statement + 1)
            }
            LastRun::Failed { message } => format!("Failed: {message}"),
        }
    }
}

/// What the tab's status chip says (docs/PROJECT.md 9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabStatus {
    NotConnected,
    Connecting,
    Connected,
    Running,
    Cancelling,
    TransactionAborted,
    SessionReset(SessionResetReason),
}

impl TabStatus {
    pub fn label(&self) -> &'static str {
        match self {
            TabStatus::NotConnected => "Not connected",
            TabStatus::Connecting => "Connecting...",
            TabStatus::Connected => "Connected",
            TabStatus::Running => "Running...",
            TabStatus::Cancelling => "Cancelling...",
            TabStatus::TransactionAborted => "Transaction aborted",
            TabStatus::SessionReset(_) => "Session reset",
        }
    }

    pub fn derive(
        activity: Activity,
        session: SessionState,
        connection: Option<ConnectionState>,
    ) -> Self {
        let connecting = connection == Some(ConnectionState::Connecting);
        match activity {
            Activity::Cancelling(_) => return TabStatus::Cancelling,
            Activity::Starting(_) | Activity::Running(_) if connecting => {
                return TabStatus::Connecting;
            }
            Activity::Starting(_) | Activity::Running(_) => return TabStatus::Running,
            Activity::Idle => {}
        }
        match session {
            SessionState::NotOpen if connecting => TabStatus::Connecting,
            SessionState::NotOpen => TabStatus::NotConnected,
            SessionState::Open => TabStatus::Connected,
            SessionState::TransactionAborted => TabStatus::TransactionAborted,
            SessionState::Reset { reason } => TabStatus::SessionReset(reason),
        }
    }
}

/// One editor tab, bound to one connection with its own session (docs/PROJECT.md 9.1).
#[derive(Debug, Clone, PartialEq)]
pub struct TabModel {
    pub id: TabId,
    pub connection: ConnectionId,
    pub title: String,
    pub session: SessionState,
    pub activity: Activity,
    pub statements: Vec<StatementSpan>,
    /// Results of the current (or last) execution, in statement order.
    pub results: Vec<ResultId>,
    pub last_run: Option<LastRun>,
}

impl TabModel {
    pub fn new(id: TabId, connection: ConnectionId, title: String) -> Self {
        Self {
            id,
            connection,
            title,
            session: SessionState::NotOpen,
            activity: Activity::Idle,
            statements: Vec::new(),
            results: Vec::new(),
            last_run: None,
        }
    }
}

/// The open editor tabs, in tab-bar order.
#[derive(Debug, Default)]
pub struct TabsModel {
    tabs: Vec<TabModel>,
    active: Option<TabId>,
    titled: u64,
}

impl TabsModel {
    pub fn open(&mut self, id: TabId, connection: ConnectionId) {
        self.titled += 1;
        let title = format!("Query {}", self.titled);
        self.tabs.push(TabModel::new(id, connection, title));
        self.active = Some(id);
    }

    pub fn close(&mut self, id: TabId) -> Option<TabModel> {
        let position = self.tabs.iter().position(|tab| tab.id == id)?;
        let closed = self.tabs.remove(position);
        if self.active == Some(id) {
            let neighbour = position.min(self.tabs.len().saturating_sub(1));
            self.active = self.tabs.get(neighbour).map(|tab| tab.id);
        }
        Some(closed)
    }

    pub fn get(&self, id: TabId) -> Option<&TabModel> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn get_mut(&mut self, id: TabId) -> Option<&mut TabModel> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &TabModel> {
        self.tabs.iter()
    }

    pub fn iter_ids(&self) -> Vec<TabId> {
        self.tabs.iter().map(|tab| tab.id).collect()
    }

    /// Gives the tab a new id and connection, keeping its place and title. The old model
    /// is returned so the caller can release what it held.
    pub fn rebind(
        &mut self,
        old: TabId,
        fresh: TabId,
        connection: ConnectionId,
    ) -> Option<TabModel> {
        let tab = self.get_mut(old)?;
        let previous = tab.clone();
        tab.id = fresh;
        tab.connection = connection;
        tab.session = SessionState::NotOpen;
        tab.statements.clear();
        tab.results.clear();
        tab.last_run = None;
        if self.active == Some(old) {
            self.active = Some(fresh);
        }
        Some(previous)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active(&self) -> Option<&TabModel> {
        self.active.and_then(|id| self.get(id))
    }

    pub fn active_id(&self) -> Option<TabId> {
        self.active
    }

    pub fn set_active(&mut self, id: TabId) {
        if self.get(id).is_some() {
            self.active = Some(id);
        }
    }

    /// The tab whose current execution is `request`.
    pub fn by_request_mut(&mut self, request: RequestId) -> Option<&mut TabModel> {
        self.tabs
            .iter_mut()
            .find(|tab| tab.activity.request() == Some(request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_activates_and_titles_count_up() {
        let mut tabs = TabsModel::default();
        tabs.open(TabId(1), ConnectionId(1));
        tabs.open(TabId(2), ConnectionId(1));
        assert_eq!(tabs.active_id(), Some(TabId(2)));
        assert_eq!(
            tabs.get(TabId(1)).map(|tab| tab.title.as_str()),
            Some("Query 1")
        );
        assert_eq!(
            tabs.get(TabId(2)).map(|tab| tab.title.as_str()),
            Some("Query 2")
        );
    }

    #[test]
    fn closing_the_active_tab_activates_a_neighbour() {
        let mut tabs = TabsModel::default();
        tabs.open(TabId(1), ConnectionId(1));
        tabs.open(TabId(2), ConnectionId(1));
        tabs.open(TabId(3), ConnectionId(1));
        tabs.set_active(TabId(2));
        tabs.close(TabId(2));
        assert_eq!(tabs.active_id(), Some(TabId(3)));
        tabs.close(TabId(3));
        assert_eq!(tabs.active_id(), Some(TabId(1)));
        tabs.close(TabId(1));
        assert_eq!(tabs.active_id(), None);
    }

    #[test]
    fn status_follows_activity_then_session() {
        let idle = Activity::Idle;
        let running = Activity::Running(RequestId(1));
        assert_eq!(
            TabStatus::derive(
                idle,
                SessionState::NotOpen,
                Some(ConnectionState::Disconnected)
            ),
            TabStatus::NotConnected
        );
        assert_eq!(
            TabStatus::derive(idle, SessionState::Open, Some(ConnectionState::Connected)),
            TabStatus::Connected
        );
        assert_eq!(
            TabStatus::derive(
                running,
                SessionState::Open,
                Some(ConnectionState::Connected)
            ),
            TabStatus::Running
        );
        assert_eq!(
            TabStatus::derive(
                running,
                SessionState::NotOpen,
                Some(ConnectionState::Connecting)
            ),
            TabStatus::Connecting
        );
        assert_eq!(
            TabStatus::derive(Activity::Cancelling(RequestId(1)), SessionState::Open, None),
            TabStatus::Cancelling
        );
        assert_eq!(
            TabStatus::derive(idle, SessionState::TransactionAborted, None),
            TabStatus::TransactionAborted
        );
    }

    #[test]
    fn last_run_labels_are_one_based() {
        assert_eq!(
            LastRun::StoppedOnError { statement: 1 }.label(),
            "Stopped at statement 2"
        );
    }
}
