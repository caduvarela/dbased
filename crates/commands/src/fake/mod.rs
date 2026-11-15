//! Fake Core for UI development and tests (docs/PROJECT.md 11.4): a synchronous state
//! machine behind the same `Command`/`Event` contract. Time never passes inside it.

mod catalog;
pub mod contract;
mod rows;
mod script;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use crate::{
    Command, ConnectionId, ConnectionState, ConnectionSummary, CoreLink, DatabaseError, ErrorInfo,
    ErrorKind, Event, ExecuteTarget, ExecutionStatus, MetadataNode, PasswordSource, PasswordUpdate,
    RequestId, ResultId, ResultOutcome, ResultStatus, SessionState, StatementSpan, TabId, Value,
};
pub use rows::MAX_ROWS;
use rows::{select_columns, select_row};
use script::{Cue, ScriptStatement, parse_cue, resolve_statements};

struct Running {
    request: RequestId,
    statement: u32,
    result: ResultId,
}

struct TabState {
    connection: ConnectionId,
    session_open: bool,
    running: Option<Running>,
}

struct ResultState {
    tab: TabId,
    rows: u64,
}

pub struct FakeCore {
    connections: Vec<ConnectionSummary>,
    tabs: HashMap<TabId, TabState>,
    results: HashMap<ResultId, ResultState>,
}

impl Default for FakeCore {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeCore {
    pub fn new() -> Self {
        Self {
            connections: catalog::initial_connections(),
            tabs: HashMap::new(),
            results: HashMap::new(),
        }
    }

    /// What the real Core sends once its configuration is loaded.
    pub fn initial_events(&self) -> Vec<Event> {
        vec![self.connections_changed()]
    }

    /// Runs until the UI drops its command sender or its event receiver.
    pub async fn serve(mut self, link: CoreLink) {
        for event in self.initial_events() {
            if link.events.send(event).await.is_err() {
                return;
            }
        }
        while let Ok(command) = link.commands.recv().await {
            for event in self.handle(command) {
                if link.events.send(event).await.is_err() {
                    return;
                }
            }
        }
    }

    pub fn handle(&mut self, command: Command) -> Vec<Event> {
        match command {
            Command::SaveConnection {
                request,
                id,
                settings,
                password,
            } => self.save_connection(request, id, settings, password),
            Command::DeleteConnection { request, id } => self.delete_connection(request, id),
            Command::TestConnection {
                request,
                id,
                password,
                ..
            } => self.test_connection(request, id, &password),
            Command::Connect {
                request,
                id,
                password,
            } => self.connect(request, id, &password),
            Command::Disconnect { request, id } => self.disconnect(request, id),
            Command::OpenTab { tab, connection } => self.open_tab(tab, connection),
            Command::CloseTab { tab } => self.close_tab(tab),
            Command::Execute {
                request,
                tab,
                script,
                target,
            } => self.execute(request, tab, &script, target),
            Command::Cancel { tab, request } => self.cancel(tab, request),
            Command::FetchRows {
                result,
                start,
                count,
            } => self.fetch_rows(result, start, count),
            Command::ReleaseResult { result } => {
                self.results.remove(&result);
                Vec::new()
            }
            Command::LoadChildren {
                request,
                connection,
                node,
            } => self.load_children(request, connection, node),
        }
    }

    fn connections_changed(&self) -> Event {
        Event::ConnectionsChanged {
            connections: self.connections.clone(),
        }
    }

    fn connection_mut(&mut self, id: ConnectionId) -> Option<&mut ConnectionSummary> {
        self.connections
            .iter_mut()
            .find(|connection| connection.id == id)
    }

    fn save_connection(
        &mut self,
        request: RequestId,
        id: Option<ConnectionId>,
        settings: crate::ConnectionSettings,
        password: PasswordUpdate,
    ) -> Vec<Event> {
        let saved_id = match id {
            Some(id) => {
                let Some(existing) = self.connection_mut(id) else {
                    return vec![failed(
                        request,
                        ErrorKind::UnknownConnection,
                        "unknown connection",
                    )];
                };
                existing.settings = settings;
                apply_password_update(existing, &password);
                id
            }
            None => {
                let next = self
                    .connections
                    .iter()
                    .map(|connection| connection.id.0)
                    .max()
                    .unwrap_or(0)
                    + 1;
                let mut created = ConnectionSummary {
                    id: ConnectionId(next),
                    settings,
                    password_saved: false,
                    state: ConnectionState::Disconnected,
                };
                apply_password_update(&mut created, &password);
                self.connections.push(created);
                ConnectionId(next)
            }
        };
        vec![
            self.connections_changed(),
            Event::ConnectionSaved {
                request,
                id: saved_id,
            },
        ]
    }

    fn delete_connection(&mut self, request: RequestId, id: ConnectionId) -> Vec<Event> {
        let before = self.connections.len();
        self.connections.retain(|connection| connection.id != id);
        if self.connections.len() == before {
            return vec![unknown_connection(request)];
        }
        vec![
            self.connections_changed(),
            Event::RequestCompleted { request },
        ]
    }

    fn test_connection(
        &self,
        request: RequestId,
        id: Option<ConnectionId>,
        password: &PasswordSource,
    ) -> Vec<Event> {
        if matches!(password, PasswordSource::Saved) && !self.has_saved_password(id) {
            return vec![password_required(request)];
        }
        vec![Event::RequestCompleted { request }]
    }

    fn has_saved_password(&self, id: Option<ConnectionId>) -> bool {
        self.connections
            .iter()
            .any(|connection| Some(connection.id) == id && connection.password_saved)
    }

    fn connect(
        &mut self,
        request: RequestId,
        id: ConnectionId,
        password: &PasswordSource,
    ) -> Vec<Event> {
        if !self
            .connections
            .iter()
            .any(|connection| connection.id == id)
        {
            return vec![unknown_connection(request)];
        }
        if matches!(password, PasswordSource::Saved) && !self.has_saved_password(Some(id)) {
            return vec![password_required(request)];
        }
        let mut events = self.set_state(id, ConnectionState::Connecting);
        events.extend(self.set_state(id, ConnectionState::Connected));
        events.push(Event::RequestCompleted { request });
        events
    }

    fn disconnect(&mut self, request: RequestId, id: ConnectionId) -> Vec<Event> {
        if !self
            .connections
            .iter()
            .any(|connection| connection.id == id)
        {
            return vec![unknown_connection(request)];
        }
        let mut events = self.set_state(id, ConnectionState::Disconnected);
        events.push(Event::RequestCompleted { request });
        events
    }

    fn set_state(&mut self, id: ConnectionId, state: ConnectionState) -> Vec<Event> {
        if let Some(connection) = self.connection_mut(id) {
            connection.state = state;
        }
        vec![Event::ConnectionStateChanged { id, state }]
    }

    fn open_tab(&mut self, tab: TabId, connection: ConnectionId) -> Vec<Event> {
        if !self.connections.iter().any(|known| known.id == connection) {
            return vec![Event::Error {
                error: error_info(ErrorKind::UnknownConnection, "unknown connection"),
            }];
        }
        self.tabs.insert(
            tab,
            TabState {
                connection,
                session_open: false,
                running: None,
            },
        );
        Vec::new()
    }

    fn close_tab(&mut self, tab: TabId) -> Vec<Event> {
        let Some(state) = self.tabs.remove(&tab) else {
            return vec![Event::Error {
                error: error_info(ErrorKind::UnknownTab, "unknown tab"),
            }];
        };
        self.results.retain(|_, result| result.tab != tab);
        let mut events = state.running.map(cancelled_events).unwrap_or_default();
        events.push(Event::TabClosed { tab });
        events
    }

    fn cancel(&mut self, tab: TabId, request: RequestId) -> Vec<Event> {
        let Some(state) = self.tabs.get_mut(&tab) else {
            return Vec::new();
        };
        let matches_request = state
            .running
            .as_ref()
            .is_some_and(|running| running.request == request);
        if !matches_request {
            return Vec::new();
        }
        state
            .running
            .take()
            .map(cancelled_events)
            .unwrap_or_default()
    }

    fn execute(
        &mut self,
        request: RequestId,
        tab: TabId,
        script: &str,
        target: ExecuteTarget,
    ) -> Vec<Event> {
        let Some(state) = self.tabs.get(&tab) else {
            return vec![failed(request, ErrorKind::UnknownTab, "unknown tab")];
        };
        if state.running.is_some() {
            return vec![failed(
                request,
                ErrorKind::TabBusy,
                "the tab is already running",
            )];
        }
        let connection_id = state.connection;
        let Some(connection) = self
            .connections
            .iter()
            .find(|connection| connection.id == connection_id)
        else {
            return vec![unknown_connection(request)];
        };
        let statements = resolve_statements(script, target, connection.settings.kind);
        if statements.is_empty() {
            return vec![failed(
                request,
                ErrorKind::EmptyScript,
                "nothing to execute",
            )];
        }
        let needs_connect = connection.state != ConnectionState::Connected;

        let mut events = Vec::new();
        if needs_connect {
            events.extend(self.set_state(connection_id, ConnectionState::Connecting));
            events.extend(self.set_state(connection_id, ConnectionState::Connected));
        }
        if let Some(state) = self.tabs.get_mut(&tab)
            && !state.session_open
        {
            state.session_open = true;
            events.push(Event::SessionStateChanged {
                tab,
                state: SessionState::Open,
            });
        }
        let run = run_statements(request, tab, &statements);
        events.extend(run.events);
        for (result, rows) in run.results {
            self.results.insert(result, ResultState { tab, rows });
        }
        if let Some(state) = self.tabs.get_mut(&tab) {
            state.running = run.running;
        }
        events
    }

    fn fetch_rows(&self, result: ResultId, start: u64, count: u32) -> Vec<Event> {
        let Some(state) = self.results.get(&result) else {
            return vec![Event::Error {
                error: error_info(ErrorKind::Internal, "unknown result"),
            }];
        };
        let first = start.min(state.rows);
        let end = start.saturating_add(u64::from(count)).min(state.rows);
        let rows: Vec<Vec<Value>> = (first..end).map(select_row).collect();
        vec![Event::RowsWindow {
            result,
            start,
            rows,
        }]
    }

    fn load_children(
        &self,
        request: RequestId,
        connection: ConnectionId,
        node: MetadataNode,
    ) -> Vec<Event> {
        if !self.connections.iter().any(|known| known.id == connection) {
            return vec![unknown_connection(request)];
        }
        let items = catalog::children(&node);
        vec![Event::ChildrenLoaded {
            request,
            connection,
            node,
            items,
        }]
    }
}

struct Run {
    events: Vec<Event>,
    results: Vec<(ResultId, u64)>,
    running: Option<Running>,
}

/// Plays the whole script: it ends at the first `fail` or `sleep`, else completes.
fn run_statements(request: RequestId, tab: TabId, statements: &[ScriptStatement]) -> Run {
    let spans = statements
        .iter()
        .enumerate()
        .map(|(index, statement)| StatementSpan {
            index: index as u32,
            start: statement.start,
            end: statement.end,
        })
        .collect();
    let mut run = Run {
        events: vec![Event::ExecutionStarted {
            request,
            tab,
            statements: spans,
        }],
        results: Vec::new(),
        running: None,
    };
    for (index, statement) in statements.iter().enumerate() {
        let index = index as u32;
        let result = ResultId {
            request,
            statement: index,
        };
        match parse_cue(&statement.text) {
            Cue::Select { rows } => {
                let (outcome_status, count) = if rows > MAX_ROWS {
                    (ResultStatus::Truncated, MAX_ROWS)
                } else {
                    (ResultStatus::Completed, rows)
                };
                run.results.push((result, count));
                run.events.push(Event::ResultStarted {
                    result,
                    columns: select_columns(),
                });
                run.events.push(Event::ResultProgress {
                    result,
                    rows_buffered: count,
                });
                run.events.push(Event::ResultFinished {
                    result,
                    outcome: ResultOutcome {
                        status: outcome_status,
                        rows_received: count,
                        rows_affected: None,
                        partial: outcome_status == ResultStatus::Truncated,
                        error: None,
                        cancel_error: None,
                    },
                });
            }
            Cue::Other => {
                run.events.push(started_without_columns(result));
                run.events.push(Event::ResultFinished {
                    result,
                    outcome: ResultOutcome {
                        status: ResultStatus::Completed,
                        rows_received: 0,
                        rows_affected: Some(0),
                        partial: false,
                        error: None,
                        cancel_error: None,
                    },
                });
            }
            Cue::Fail { position } => {
                run.events.push(started_without_columns(result));
                run.events.push(Event::ResultFinished {
                    result,
                    outcome: ResultOutcome {
                        status: ResultStatus::Failed,
                        rows_received: 0,
                        rows_affected: None,
                        partial: false,
                        error: Some(DatabaseError {
                            sqlstate: "42601".to_string(),
                            message: "syntax error".to_string(),
                            detail: None,
                            hint: None,
                            position: Some(position),
                        }),
                        cancel_error: None,
                    },
                });
                run.events.push(Event::ExecutionFinished {
                    request,
                    status: ExecutionStatus::StoppedOnError { statement: index },
                });
                return run;
            }
            Cue::Sleep => {
                run.events.push(started_without_columns(result));
                run.running = Some(Running {
                    request,
                    statement: index,
                    result,
                });
                return run;
            }
        }
    }
    run.events.push(Event::ExecutionFinished {
        request,
        status: ExecutionStatus::Completed,
    });
    run
}

fn started_without_columns(result: ResultId) -> Event {
    Event::ResultStarted {
        result,
        columns: Vec::new(),
    }
}

fn cancelled_events(running: Running) -> Vec<Event> {
    vec![
        Event::ResultFinished {
            result: running.result,
            outcome: ResultOutcome {
                status: ResultStatus::Cancelled,
                rows_received: 0,
                rows_affected: None,
                partial: false,
                error: None,
                cancel_error: None,
            },
        },
        Event::ExecutionFinished {
            request: running.request,
            status: ExecutionStatus::Cancelled {
                statement: running.statement,
            },
        },
    ]
}

fn apply_password_update(connection: &mut ConnectionSummary, update: &PasswordUpdate) {
    match update {
        PasswordUpdate::Keep => {}
        PasswordUpdate::Save(_) => connection.password_saved = true,
        PasswordUpdate::Forget => connection.password_saved = false,
    }
}

fn error_info(kind: ErrorKind, message: &str) -> ErrorInfo {
    ErrorInfo {
        kind,
        message: message.to_string(),
        database: None,
    }
}

fn failed(request: RequestId, kind: ErrorKind, message: &str) -> Event {
    Event::RequestFailed {
        request,
        error: error_info(kind, message),
    }
}

fn unknown_connection(request: RequestId) -> Event {
    failed(request, ErrorKind::UnknownConnection, "unknown connection")
}

fn password_required(request: RequestId) -> Event {
    failed(
        request,
        ErrorKind::PasswordRequired,
        "a password is required",
    )
}
