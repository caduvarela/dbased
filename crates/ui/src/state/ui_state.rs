use std::collections::HashMap;

use dbased_commands::{Command, ConnectionId, Event, ExecuteTarget, RequestId, ResultId, TabId};

use super::{
    connection_flow::{ConnectionFlow, Outcome, PendingExecute},
    connections::ConnectionsModel,
    explorer::{ExplorerModel, NodeKey},
    ids::IdAllocator,
    results::ResultModel,
    tabs::{Activity, LastRun, TabStatus, TabsModel},
};

/// Why a command could not be handed to the Core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendFailure {
    /// The command channel is full.
    Full,
    /// The Core end is gone.
    Closed,
}

/// All view models of the UI (docs/PROJECT.md 6.2), free of GPUI. User intents and Core
/// events go in; the `Command`s to send come out.
#[derive(Debug, Default)]
pub struct UiState {
    pub connections: ConnectionsModel,
    pub explorer: ExplorerModel,
    pub tabs: TabsModel,
    pub flow: ConnectionFlow,
    results: HashMap<ResultId, ResultModel>,
    /// The latest problem worth telling the user about.
    pub notice: Option<String>,
    /// False once the Core's event channel closed.
    pub core_stopped: bool,
    ids: IdAllocator,
    first_tab_opened: bool,
}

impl UiState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn result(&self, id: ResultId) -> Option<&ResultModel> {
        self.results.get(&id)
    }

    pub fn tab_status(&self, tab: TabId) -> Option<TabStatus> {
        let tab = self.tabs.get(tab)?;
        let connection = self.connections.get(tab.connection).map(|item| item.state);
        Some(TabStatus::derive(tab.activity, tab.session, connection))
    }

    pub fn can_run(&self, tab: TabId) -> bool {
        self.tabs
            .get(tab)
            .is_some_and(|tab| !tab.activity.is_busy())
    }

    pub fn can_cancel(&self, tab: TabId) -> bool {
        self.tabs
            .get(tab)
            .is_some_and(|tab| matches!(tab.activity, Activity::Starting(_) | Activity::Running(_)))
    }

    pub fn dismiss_notice(&mut self) {
        self.notice = None;
    }

    pub fn send_failed(&mut self, failure: SendFailure) {
        self.notice = Some(match failure {
            SendFailure::Full => "The Core is not responding.".to_string(),
            SendFailure::Closed => "The Core is not available.".to_string(),
        });
    }

    pub fn core_stopped(&mut self) {
        self.core_stopped = true;
        self.notice = Some("The Core stopped.".to_string());
    }

    // User intents.

    /// Opens a tab bound to `connection`.
    pub fn open_tab(&mut self, connection: ConnectionId) -> Option<(TabId, Vec<Command>)> {
        self.connections.get(connection)?;
        let tab = self.ids.tab();
        self.tabs.open(tab, connection);
        Some((tab, vec![Command::OpenTab { tab, connection }]))
    }

    /// The connection a new tab should use: the active tab's, else the first one.
    pub fn default_connection(&self) -> Option<ConnectionId> {
        self.tabs
            .active()
            .map(|tab| tab.connection)
            .or_else(|| self.connections.first_id())
    }

    pub fn close_tab(&mut self, tab: TabId) -> Vec<Command> {
        let Some(closed) = self.tabs.close(tab) else {
            return Vec::new();
        };
        for result in closed.results {
            self.results.remove(&result);
        }
        vec![Command::CloseTab { tab }]
    }

    /// Moves a tab to another connection. The tab gets a new id because the Core binds a
    /// tab to its connection for good (docs/PROJECT.md 9.1). Refused while it runs.
    pub fn rebind_tab(
        &mut self,
        tab: TabId,
        connection: ConnectionId,
    ) -> Option<(TabId, Vec<Command>)> {
        self.connections.get(connection)?;
        let current = self.tabs.get(tab)?;
        if current.activity.is_busy() || current.connection == connection {
            return None;
        }
        let fresh = self.ids.tab();
        let old = self.tabs.rebind(tab, fresh, connection)?;
        for result in old.results {
            self.results.remove(&result);
        }
        let commands = vec![
            Command::CloseTab { tab },
            Command::OpenTab {
                tab: fresh,
                connection,
            },
        ];
        Some((fresh, commands))
    }

    pub fn execute(&mut self, tab: TabId, script: String, target: ExecuteTarget) -> Vec<Command> {
        let request = self.ids.request();
        let Some(model) = self.tabs.get_mut(tab) else {
            return Vec::new();
        };
        if model.activity.is_busy() {
            return Vec::new();
        }
        self.flow.track_execute(
            request,
            PendingExecute {
                tab,
                script: script.clone(),
                target,
            },
        );
        let stale = std::mem::take(&mut model.results);
        model.statements.clear();
        model.last_run = None;
        model.activity = Activity::Starting(request);
        let mut commands = Vec::new();
        for result in stale {
            self.results.remove(&result);
            commands.push(Command::ReleaseResult { result });
        }
        commands.push(Command::Execute {
            request,
            tab,
            script,
            target,
        });
        commands
    }

    pub fn cancel(&mut self, tab: TabId) -> Vec<Command> {
        let Some(model) = self.tabs.get_mut(tab) else {
            return Vec::new();
        };
        let request = match model.activity {
            Activity::Starting(request) | Activity::Running(request) => request,
            Activity::Idle | Activity::Cancelling(_) => return Vec::new(),
        };
        model.activity = Activity::Cancelling(request);
        vec![Command::Cancel { tab, request }]
    }

    /// The user closed a result panel.
    pub fn release_result(&mut self, result: ResultId) -> Vec<Command> {
        if self.results.remove(&result).is_none() {
            return Vec::new();
        }
        for tab in self.tabs.iter_ids() {
            if let Some(model) = self.tabs.get_mut(tab) {
                model.results.retain(|known| *known != result);
            }
        }
        vec![Command::ReleaseResult { result }]
    }

    pub fn toggle_node(&mut self, key: &NodeKey) -> Vec<Command> {
        self.explorer.toggle(key, &self.connections, &mut self.ids)
    }

    // Core events.

    pub fn apply(&mut self, event: Event) -> Vec<Command> {
        match event {
            Event::ConnectionsChanged { connections } => self.connections_changed(connections),
            Event::ConnectionStateChanged { id, state } => {
                self.connections.set_state(id, state);
                if state == dbased_commands::ConnectionState::Disconnected {
                    self.explorer.forget_connection(id);
                }
                Vec::new()
            }
            Event::ConnectionSaved { request, id } => {
                self.flow.saved(request, id);
                Vec::new()
            }
            Event::RequestCompleted { request } => self.connection_request_completed(request),
            Event::RequestFailed { request, error } => {
                self.request_failed(request, error);
                Vec::new()
            }
            Event::TabClosed { tab } => {
                // @NOTE: a close the UI asked for was already applied when it was sent.
                self.close_tab(tab);
                Vec::new()
            }
            Event::SessionStateChanged { tab, state } => {
                if let Some(model) = self.tabs.get_mut(tab) {
                    model.session = state;
                }
                Vec::new()
            }
            Event::ExecutionStarted {
                request,
                tab,
                statements,
            } => {
                self.execution_started(request, tab, statements);
                Vec::new()
            }
            Event::ResultStarted { result, columns } => {
                self.result_started(result, columns);
                Vec::new()
            }
            Event::ResultProgress {
                result,
                rows_buffered,
            } => {
                if let Some(model) = self.results.get_mut(&result) {
                    model.rows_buffered = rows_buffered;
                }
                Vec::new()
            }
            Event::ResultFinished { result, outcome } => {
                if let Some(model) = self.results.get_mut(&result) {
                    model.outcome = Some(outcome);
                }
                Vec::new()
            }
            Event::ExecutionFinished { request, status } => {
                self.flow.take_execute(request);
                if let Some(model) = self.tabs.by_request_mut(request) {
                    model.activity = Activity::Idle;
                    model.last_run = Some(LastRun::from_status(&status));
                }
                Vec::new()
            }
            // @NOTE: rows belong to the grid, which pulled them; the view layer routes them.
            Event::RowsWindow { .. } => Vec::new(),
            Event::ChildrenLoaded {
                request,
                connection,
                node,
                items,
            } => {
                self.explorer
                    .apply_children(request, connection, &node, items);
                Vec::new()
            }
            Event::Error { error } => {
                self.notice = Some(error.message);
                Vec::new()
            }
        }
    }

    fn connections_changed(
        &mut self,
        connections: Vec<dbased_commands::ConnectionSummary>,
    ) -> Vec<Command> {
        self.connections.replace(connections);
        self.explorer.retain_connections(&self.connections);
        if self.first_tab_opened {
            return Vec::new();
        }
        let Some(first) = self.connections.first_id() else {
            return Vec::new();
        };
        self.first_tab_opened = true;
        self.open_tab(first)
            .map(|(_, commands)| commands)
            .unwrap_or_default()
    }

    fn request_failed(&mut self, request: RequestId, error: dbased_commands::ErrorInfo) {
        match self.flow.failed(request, &error) {
            Some(Outcome::Notice(message)) => self.notice = Some(message),
            Some(_) => {}
            None => self.execution_request_failed(request, error),
        }
    }

    fn execution_request_failed(&mut self, request: RequestId, error: dbased_commands::ErrorInfo) {
        let execute = self.flow.take_execute(request);
        if let Some(model) = self.tabs.by_request_mut(request) {
            if error.kind == dbased_commands::ErrorKind::PasswordRequired
                && let Some(execute) = execute
            {
                model.activity = Activity::Idle;
                let connection = model.connection;
                self.flow.ask_password(connection, Some(execute));
                return;
            }
            model.activity = Activity::Idle;
            model.last_run = Some(LastRun::Failed {
                message: error.message,
            });
            return;
        }
        if self.explorer.apply_failure(request, &error) {
            return;
        }
        self.notice = Some(error.message);
    }

    fn execution_started(
        &mut self,
        request: RequestId,
        tab: TabId,
        statements: Vec<dbased_commands::StatementSpan>,
    ) {
        let Some(model) = self.tabs.get_mut(tab) else {
            return;
        };
        if model.activity.request() != Some(request) {
            return;
        }
        if let Activity::Starting(_) = model.activity {
            model.activity = Activity::Running(request);
        }
        model.statements = statements;
    }

    fn result_started(&mut self, result: ResultId, columns: Vec<dbased_commands::ColumnMeta>) {
        let Some(tab) = self.tabs.by_request_mut(result.request) else {
            return;
        };
        let tab_id = tab.id;
        tab.results.push(result);
        self.results
            .insert(result, ResultModel::new(result, tab_id, columns));
    }
}

mod connection_intents;

#[cfg(test)]
mod tests;
