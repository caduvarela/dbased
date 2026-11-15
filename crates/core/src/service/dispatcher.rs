use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_channel::{Receiver, Sender};
use dbased_commands::{
    Command, ConnectionId, ConnectionSettings, ConnectionState, ConnectionSummary, ErrorInfo,
    Event, ExecuteTarget, MetadataItem, MetadataItemKind, MetadataNode, PasswordSource,
    PasswordUpdate, RequestId, ResultId, TabId,
};
use secrecy::SecretString;
use tokio::sync::oneshot;
use tokio::task::{Id as TaskId, JoinError, JoinSet};
use tokio_util::sync::CancellationToken;

use super::error_info::error_info;
use super::results::ResultStore;
use super::store::StoredConnection;
use super::tab::{
    PendingExecute, RunningSlot, TabHandle, TabMessage, dialect_of, new_tab, plan_statements,
};
use super::{ConnectionStore, CoreDependencies, DriverConnector, SecretStore};
use crate::{CoreError, Driver, TableRef};

struct ConnectionEntry {
    settings: ConnectionSettings,
    password_saved: bool,
    state: ConnectionState,
    driver: Option<Arc<dyn Driver>>,
    // @NOTE: executes that arrived while the connection was down or connecting; each one holds
    // its tab's running slot, so the tab already reports busy.
    waiting_executes: Vec<PendingExecute>,
    // @NOTE: explicit `Connect` requests that arrived while an attempt was running; they join it
    // and end with its result.
    joined_connects: Vec<RequestId>,
}

/// What a spawned task serves, so a panic can still end its request (11.1: one terminal event).
enum TaskRole {
    Connect {
        id: ConnectionId,
        request: Option<RequestId>,
    },
    Close {
        request: Option<RequestId>,
    },
    /// A task that emits its own terminal event.
    Request(RequestId),
    Tab {
        tab: TabId,
        generation: u64,
        running: RunningSlot,
    },
}

/// What a spawned task hands back to the dispatcher. Tasks that only emit their own terminal
/// event return `Done`.
enum Completion {
    Connected {
        // @NOTE: `None` for the connect an execute starts on its own.
        request: Option<RequestId>,
        id: ConnectionId,
        result: Result<Arc<dyn Driver>, CoreError>,
    },
    Closed {
        request: Option<RequestId>,
    },
    Done,
}

/// One task owns all service state. It never awaits database I/O: anything slower than a map
/// lookup runs in `tasks` and reports back through a `Completion`.
pub(super) struct Dispatcher {
    store: Arc<ConnectionStore>,
    secrets: Arc<dyn SecretStore>,
    connector: Arc<dyn DriverConnector>,
    events: Sender<Event>,
    connections: BTreeMap<ConnectionId, ConnectionEntry>,
    tabs: HashMap<TabId, TabHandle>,
    results: ResultStore,
    // @NOTE: set when the config file could not be read, so a save never overwrites it blindly.
    storage_blocked: Option<String>,
    tasks: JoinSet<Completion>,
    roles: HashMap<TaskId, TaskRole>,
    next_generation: u64,
    shutting_down: bool,
}

impl Dispatcher {
    pub(super) fn new(dependencies: CoreDependencies, events: Sender<Event>) -> Self {
        Self {
            store: Arc::new(dependencies.connection_store),
            secrets: dependencies.secret_store,
            connector: dependencies.connector,
            events,
            connections: BTreeMap::new(),
            tabs: HashMap::new(),
            results: ResultStore::default(),
            storage_blocked: None,
            tasks: JoinSet::new(),
            roles: HashMap::new(),
            next_generation: 0,
            shutting_down: false,
        }
    }

    pub(super) async fn run(mut self, commands: Receiver<Command>) {
        self.load_connections().await;
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Ok(command) => self.handle(command).await,
                    Err(_) => break,
                },
                Some(joined) = self.tasks.join_next_with_id() => self.complete(joined).await,
            }
        }
        self.shut_down().await;
    }

    async fn load_connections(&mut self) {
        let store = self.store.clone();
        let loaded = blocking(move || store.load()).await;
        match loaded {
            Ok(stored) => {
                for connection in stored {
                    self.connections.insert(
                        connection.id,
                        ConnectionEntry {
                            settings: connection.settings,
                            password_saved: connection.password_saved,
                            state: ConnectionState::Disconnected,
                            driver: None,
                            waiting_executes: Vec::new(),
                            joined_connects: Vec::new(),
                        },
                    );
                }
            }
            Err(error) => {
                self.storage_blocked = Some(error.to_string());
                self.emit(Event::Error {
                    error: error_info(&error),
                })
                .await;
            }
        }
        self.emit_connections_changed().await;
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::SaveConnection {
                request,
                id,
                settings,
                password,
            } => self.save_connection(request, id, settings, password).await,
            Command::DeleteConnection { request, id } => self.delete_connection(request, id).await,
            Command::TestConnection {
                request,
                id,
                settings,
                password,
            } => self.test_connection(request, id, settings, password),
            Command::Connect {
                request,
                id,
                password,
            } => self.connect(request, id, password).await,
            Command::Disconnect { request, id } => self.disconnect(request, id).await,
            Command::LoadChildren {
                request,
                connection,
                node,
            } => self.load_children(request, connection, node).await,
            Command::OpenTab { tab, connection } => self.open_tab(tab, connection).await,
            Command::CloseTab { tab } => self.close_tab(tab).await,
            Command::Execute {
                request,
                tab,
                script,
                target,
            } => self.execute(request, tab, script, target).await,
            Command::Cancel { tab, request } => self.cancel(tab, request).await,
            Command::FetchRows {
                result,
                start,
                count,
            } => self.fetch_rows(result, start, count).await,
            Command::ReleaseResult { result } => self.results.release(result),
        }
    }

    fn spawn_task(
        &mut self,
        role: TaskRole,
        task: impl Future<Output = Completion> + Send + 'static,
    ) {
        let handle = self.tasks.spawn(task);
        self.roles.insert(handle.id(), role);
    }

    async fn emit(&self, event: Event) {
        // @NOTE: a closed event channel means the UI is gone; the shutdown path handles the rest.
        self.events.send(event).await.ok();
    }

    async fn fail(&self, request: RequestId, error: &CoreError) {
        self.fail_with(request, error_info(error)).await;
    }

    async fn fail_with(&self, request: RequestId, error: ErrorInfo) {
        self.emit(Event::RequestFailed { request, error }).await;
    }

    /// Commands without a request id (`OpenTab`, `FetchRows`) report problems here.
    async fn report(&self, error: &CoreError) {
        self.emit(Event::Error {
            error: error_info(error),
        })
        .await;
    }

    async fn emit_connections_changed(&self) {
        let connections = self
            .connections
            .iter()
            .map(|(id, entry)| ConnectionSummary {
                id: *id,
                settings: entry.settings.clone(),
                password_saved: entry.password_saved,
                state: entry.state,
            })
            .collect();
        self.emit(Event::ConnectionsChanged { connections }).await;
    }

    fn stored_connections(&self) -> Vec<StoredConnection> {
        self.connections
            .iter()
            .map(|(id, entry)| StoredConnection {
                id: *id,
                settings: entry.settings.clone(),
                password_saved: entry.password_saved,
            })
            .collect()
    }

    async fn persist(&self, connections: Vec<StoredConnection>) -> Result<(), CoreError> {
        if let Some(reason) = &self.storage_blocked {
            return Err(CoreError::Storage(format!(
                "the config file could not be read at startup: {reason}"
            )));
        }
        let store = self.store.clone();
        blocking(move || store.save(&connections)).await
    }

    async fn save_connection(
        &mut self,
        request: RequestId,
        id: Option<ConnectionId>,
        settings: ConnectionSettings,
        password: PasswordUpdate,
    ) {
        match self.try_save(id, settings, password).await {
            Ok(id) => self.emit(Event::ConnectionSaved { request, id }).await,
            Err(error) => self.fail(request, &error).await,
        }
    }

    async fn try_save(
        &mut self,
        id: Option<ConnectionId>,
        settings: ConnectionSettings,
        password: PasswordUpdate,
    ) -> Result<ConnectionId, CoreError> {
        validate_settings(&settings)?;
        let (id, is_new) = match id {
            Some(id) if self.connections.contains_key(&id) => (id, false),
            Some(_) => return Err(CoreError::UnknownConnection),
            None => (self.next_id(), true),
        };
        let previously_saved = self
            .connections
            .get(&id)
            .is_some_and(|entry| entry.password_saved);
        let password_saved = match &password {
            PasswordUpdate::Keep => previously_saved,
            PasswordUpdate::Save(_) => true,
            PasswordUpdate::Forget => false,
        };

        // @NOTE: the secret is written first so a failure here leaves the file untouched.
        self.apply_password_update(id, &password).await?;

        let mut stored = self.stored_connections();
        let record = StoredConnection {
            id,
            settings: settings.clone(),
            password_saved,
        };
        match stored.iter_mut().find(|existing| existing.id == id) {
            Some(existing) => *existing = record,
            None => stored.push(record),
        }
        if let Err(error) = self.persist(stored).await {
            if is_new && matches!(password, PasswordUpdate::Save(_)) {
                self.delete_secret(id).await.ok();
            }
            return Err(error);
        }

        match self.connections.get_mut(&id) {
            Some(entry) => {
                entry.settings = settings;
                entry.password_saved = password_saved;
            }
            None => {
                self.connections.insert(
                    id,
                    ConnectionEntry {
                        settings,
                        password_saved,
                        state: ConnectionState::Disconnected,
                        driver: None,
                        waiting_executes: Vec::new(),
                        joined_connects: Vec::new(),
                    },
                );
            }
        }
        self.emit_connections_changed().await;
        Ok(id)
    }

    fn next_id(&self) -> ConnectionId {
        let highest = self.connections.keys().next_back().map_or(0, |id| id.0);
        ConnectionId(highest + 1)
    }

    async fn apply_password_update(
        &self,
        id: ConnectionId,
        update: &PasswordUpdate,
    ) -> Result<(), CoreError> {
        match update {
            PasswordUpdate::Keep => Ok(()),
            PasswordUpdate::Save(password) => {
                let secrets = self.secrets.clone();
                let password = password.clone();
                blocking(move || secrets.set(id, &password)).await
            }
            PasswordUpdate::Forget => self.delete_secret(id).await,
        }
    }

    async fn delete_secret(&self, id: ConnectionId) -> Result<(), CoreError> {
        let secrets = self.secrets.clone();
        blocking(move || secrets.delete(id)).await
    }

    async fn delete_connection(&mut self, request: RequestId, id: ConnectionId) {
        if !self.connections.contains_key(&id) {
            self.fail(request, &CoreError::UnknownConnection).await;
            return;
        }
        let remaining = self
            .stored_connections()
            .into_iter()
            .filter(|connection| connection.id != id)
            .collect();
        if let Err(error) = self.persist(remaining).await {
            self.fail(request, &error).await;
            return;
        }
        let removed = self.connections.remove(&id);
        let (driver, waiting, joined) = match removed {
            Some(entry) => (entry.driver, entry.waiting_executes, entry.joined_connects),
            None => (None, Vec::new(), Vec::new()),
        };
        for pending in waiting {
            self.fail(pending.request, &CoreError::UnknownConnection)
                .await;
        }
        for request in joined {
            self.fail(request, &CoreError::UnknownConnection).await;
        }
        let closing_tabs = self.close_tabs_of(id);
        let secret_result = self.delete_secret(id).await;
        self.emit_connections_changed().await;

        let close_reply = secret_result.is_ok().then_some(request);
        if driver.is_some() || !closing_tabs.is_empty() {
            self.spawn_close(driver, close_reply, closing_tabs);
        } else if secret_result.is_ok() {
            self.emit(Event::RequestCompleted { request }).await;
        }
        if let Err(error) = secret_result {
            self.fail(request, &error).await;
        }
    }

    /// Waits for the tabs first: cancels use the pool, so it must outlive every session (11.2).
    fn spawn_close(
        &mut self,
        driver: Option<Arc<dyn Driver>>,
        request: Option<RequestId>,
        tabs_released: Vec<oneshot::Receiver<()>>,
    ) {
        let task = async move {
            futures_util::future::join_all(tabs_released).await;
            if let Some(driver) = driver {
                driver.close().await;
            }
            Completion::Closed { request }
        };
        self.spawn_task(TaskRole::Close { request }, task);
    }

    /// Cancels what is running and asks every tab of the connection to close its session.
    fn release_sessions(&self, connection: ConnectionId) -> Vec<oneshot::Receiver<()>> {
        let mut released = Vec::new();
        for handle in self
            .tabs
            .values()
            .filter(|tab| tab.connection == connection)
        {
            handle.running.cancel_current();
            let (done, acknowledged) = oneshot::channel();
            if handle
                .messages
                .send(TabMessage::ReleaseSession { done })
                .is_ok()
            {
                released.push(acknowledged);
            }
        }
        released
    }

    fn close_tabs_of(&mut self, connection: ConnectionId) -> Vec<oneshot::Receiver<()>> {
        let tab_ids: Vec<TabId> = self
            .tabs
            .iter()
            .filter(|(_, handle)| handle.connection == connection)
            .map(|(tab, _)| *tab)
            .collect();
        let handles: Vec<TabHandle> = tab_ids
            .iter()
            .filter_map(|tab| self.tabs.remove(tab))
            .collect();
        handles
            .into_iter()
            .map(|handle| self.close_handle(handle))
            .collect()
    }

    /// Cancels and asks the actor to close (it emits `TabClosed`). The actor's task lives in
    /// `tasks`, so shutdown waits for it.
    fn close_handle(&self, handle: TabHandle) -> oneshot::Receiver<()> {
        handle.running.cancel_current();
        let (done, acknowledged) = oneshot::channel();
        handle.messages.send(TabMessage::Close { done }).ok();
        acknowledged
    }

    async fn open_tab(&mut self, tab: TabId, connection: ConnectionId) {
        if !self.connections.contains_key(&connection) {
            self.report(&CoreError::UnknownConnection).await;
            return;
        }
        if self.tabs.contains_key(&tab) {
            self.report(&CoreError::Internal("the tab is already open".into()))
                .await;
            return;
        }
        self.next_generation += 1;
        let generation = self.next_generation;
        let (handle, actor) = new_tab(
            tab,
            generation,
            connection,
            self.events.clone(),
            self.results.clone(),
        );
        let role = TaskRole::Tab {
            tab,
            generation,
            running: handle.running.clone(),
        };
        self.spawn_task(role, async move {
            actor.await;
            Completion::Done
        });
        self.tabs.insert(tab, handle);
    }

    async fn close_tab(&mut self, tab: TabId) {
        let Some(handle) = self.tabs.remove(&tab) else {
            self.report(&CoreError::UnknownTab).await;
            return;
        };
        let waiting = self
            .connections
            .get_mut(&handle.connection)
            .map(|entry| take_waiting_of(entry, tab))
            .unwrap_or_default();
        for pending in waiting {
            self.fail(pending.request, &CoreError::UnknownTab).await;
        }
        self.close_handle(handle);
    }

    async fn execute(
        &mut self,
        request: RequestId,
        tab: TabId,
        script: String,
        target: ExecuteTarget,
    ) {
        let Some(handle) = self.tabs.get(&tab) else {
            self.fail(request, &CoreError::UnknownTab).await;
            return;
        };
        let connection = handle.connection;
        let Some(kind) = self
            .connections
            .get(&connection)
            .map(|entry| entry.settings.kind)
        else {
            self.fail(request, &CoreError::UnknownConnection).await;
            return;
        };
        // @NOTE: planned before the slot is claimed or any connect starts, so an empty script
        // or a bad selection costs nothing.
        let plan = match plan_statements(&script, target, dialect_of(kind)) {
            Ok(plan) => plan,
            Err(error) => {
                self.fail(request, &error).await;
                return;
            }
        };
        let token = CancellationToken::new();
        if let Err(error) = handle.running.begin(request, token.clone()) {
            self.fail(request, &error).await;
            return;
        }
        let running = handle.running.clone();
        let pending = PendingExecute {
            request,
            tab,
            script,
            plan,
            token,
        };
        let Some(entry) = self.connections.get_mut(&connection) else {
            running.finish();
            self.fail(request, &CoreError::UnknownConnection).await;
            return;
        };
        let connected = match (&entry.driver, entry.state) {
            (Some(driver), ConnectionState::Connected) => Some(driver.clone()),
            _ => None,
        };
        let Some(driver) = connected else {
            let must_connect = entry.state == ConnectionState::Disconnected;
            entry.waiting_executes.push(pending);
            if must_connect {
                // @NOTE: the Connecting state is visible to the UI, so this is not hidden magic.
                self.begin_connect(connection, None, PasswordSource::Saved)
                    .await;
            }
            return;
        };
        self.forward(pending, driver).await;
    }

    /// A request still waiting for its connection never reached the tab actor, so the
    /// dispatcher ends it. Cancelling before the start ends like cancelling at statement 0.
    async fn cancel(&mut self, tab: TabId, request: RequestId) {
        let Some(handle) = self.tabs.get(&tab) else {
            return;
        };
        let waiting = self
            .connections
            .get_mut(&handle.connection)
            .and_then(|entry| take_waiting_request(entry, tab, request));
        let Some(pending) = waiting else {
            handle.running.cancel(request);
            return;
        };
        handle.running.finish();
        for event in pending.cancelled_events() {
            self.emit(event).await;
        }
    }

    async fn forward(&self, pending: PendingExecute, driver: Arc<dyn Driver>) {
        let request = pending.request;
        let Some(handle) = self.tabs.get(&pending.tab) else {
            self.fail(request, &CoreError::UnknownTab).await;
            return;
        };
        let message = TabMessage::Execute {
            job: pending,
            driver,
        };
        if handle.messages.send(message).is_err() {
            handle.running.finish();
            let error = CoreError::Internal("the tab stopped".into());
            self.fail(request, &error).await;
        }
    }

    async fn fail_waiting(&self, waiting: Vec<PendingExecute>, error: &ErrorInfo) {
        for pending in waiting {
            if let Some(handle) = self.tabs.get(&pending.tab) {
                handle.running.finish();
            }
            self.fail_with(pending.request, error.clone()).await;
        }
    }

    async fn fetch_rows(&self, result: ResultId, start: u64, count: u32) {
        match self.results.window(result, start, count) {
            Ok(Some(rows)) => {
                self.emit(Event::RowsWindow {
                    result,
                    start,
                    rows,
                })
                .await;
            }
            // @NOTE: released while the fetch was in flight; the UI no longer wants it.
            Ok(None) => {}
            Err(error) => self.report(&error).await,
        }
    }

    fn test_connection(
        &mut self,
        request: RequestId,
        id: Option<ConnectionId>,
        settings: ConnectionSettings,
        password: PasswordSource,
    ) {
        let secrets = self.secrets.clone();
        let connector = self.connector.clone();
        let events = self.events.clone();
        let task = async move {
            let outcome = async {
                let password = resolve_password(secrets, id, password).await?;
                let driver = connector.connect(&settings, password).await?;
                driver.close().await;
                Ok::<(), CoreError>(())
            }
            .await;
            events.send(terminal_event(request, outcome)).await.ok();
            Completion::Done
        };
        self.spawn_task(TaskRole::Request(request), task);
    }

    async fn connect(&mut self, request: RequestId, id: ConnectionId, password: PasswordSource) {
        let Some(entry) = self.connections.get_mut(&id) else {
            self.fail(request, &CoreError::UnknownConnection).await;
            return;
        };
        match entry.state {
            ConnectionState::Connected => {
                self.emit(Event::RequestCompleted { request }).await;
                return;
            }
            ConnectionState::Connecting => {
                entry.joined_connects.push(request);
                return;
            }
            ConnectionState::Disconnected => {}
        }
        self.begin_connect(id, Some(request), password).await;
    }

    async fn begin_connect(
        &mut self,
        id: ConnectionId,
        request: Option<RequestId>,
        password: PasswordSource,
    ) {
        let Some(entry) = self.connections.get_mut(&id) else {
            return;
        };
        entry.state = ConnectionState::Connecting;
        let settings = entry.settings.clone();
        self.emit(Event::ConnectionStateChanged {
            id,
            state: ConnectionState::Connecting,
        })
        .await;

        let secrets = self.secrets.clone();
        let connector = self.connector.clone();
        let task = async move {
            let result = async {
                let password = resolve_password(secrets, Some(id), password).await?;
                connector.connect(&settings, password).await
            }
            .await;
            Completion::Connected {
                request,
                id,
                result,
            }
        };
        self.spawn_task(TaskRole::Connect { id, request }, task);
    }

    async fn disconnect(&mut self, request: RequestId, id: ConnectionId) {
        let Some(entry) = self.connections.get_mut(&id) else {
            self.fail(request, &CoreError::UnknownConnection).await;
            return;
        };
        if entry.state == ConnectionState::Connecting {
            let error = CoreError::Internal("a connection attempt is still running".into());
            self.fail(request, &error).await;
            return;
        }
        let Some(driver) = entry.driver.take() else {
            self.emit(Event::RequestCompleted { request }).await;
            return;
        };
        entry.state = ConnectionState::Disconnected;
        let released = self.release_sessions(id);
        self.emit(Event::ConnectionStateChanged {
            id,
            state: ConnectionState::Disconnected,
        })
        .await;
        self.spawn_close(Some(driver), Some(request), released);
    }

    async fn load_children(
        &mut self,
        request: RequestId,
        connection: ConnectionId,
        node: MetadataNode,
    ) {
        let Some(entry) = self.connections.get(&connection) else {
            self.fail(request, &CoreError::UnknownConnection).await;
            return;
        };
        let Some(driver) = entry.driver.clone() else {
            let error = CoreError::Connection("the connection is not open".to_owned());
            self.fail(request, &error).await;
            return;
        };
        let events = self.events.clone();
        let task = async move {
            let event = match children_of(driver.as_ref(), &node).await {
                Ok(items) => Event::ChildrenLoaded {
                    request,
                    connection,
                    node,
                    items,
                },
                Err(error) => Event::RequestFailed {
                    request,
                    error: error_info(&error),
                },
            };
            // @NOTE: sent even after a disconnect; the UI drops it by request id (3.2.4).
            events.send(event).await.ok();
            Completion::Done
        };
        self.spawn_task(TaskRole::Request(request), task);
    }

    async fn complete(&mut self, joined: Result<(TaskId, Completion), JoinError>) {
        let completion = match joined {
            Ok((task, completion)) => {
                self.roles.remove(&task);
                completion
            }
            Err(error) => {
                let role = self.roles.remove(&error.id());
                self.task_failed(role).await;
                return;
            }
        };
        match completion {
            Completion::Done => {}
            Completion::Closed { request } => self.complete_closed(request).await,
            Completion::Connected {
                request,
                id,
                result,
            } => self.finish_connect(request, id, result).await,
        }
    }

    async fn complete_closed(&self, request: Option<RequestId>) {
        if let Some(request) = request {
            self.emit(Event::RequestCompleted { request }).await;
        }
    }

    /// A panicked task still owes its request a terminal event and its state a clean-up.
    async fn task_failed(&mut self, role: Option<TaskRole>) {
        let error = CoreError::Internal("a background task panicked".into());
        match role {
            None => {}
            Some(TaskRole::Connect { id, request }) => {
                self.finish_connect(request, id, Err(error)).await;
            }
            // @NOTE: the pool may be left open; completing the request keeps the UI unstuck.
            Some(TaskRole::Close { request }) => self.complete_closed(request).await,
            Some(TaskRole::Request(request)) => self.fail(request, &error).await,
            Some(TaskRole::Tab {
                tab,
                generation,
                running,
            }) => self.tab_died(tab, generation, &running, &error).await,
        }
    }

    async fn tab_died(
        &mut self,
        tab: TabId,
        generation: u64,
        running: &RunningSlot,
        error: &CoreError,
    ) {
        let still_listed = self
            .tabs
            .get(&tab)
            .is_some_and(|handle| handle.generation == generation);
        let removed = if still_listed {
            self.tabs.remove(&tab)
        } else {
            None
        };
        let waiting = removed
            .and_then(|handle| self.connections.get_mut(&handle.connection))
            .map(|entry| take_waiting_of(entry, tab))
            .unwrap_or_default();
        let mut failed: Vec<RequestId> = Vec::new();
        for pending in waiting {
            failed.push(pending.request);
            self.fail(pending.request, error).await;
        }
        if let Some(request) = running.take()
            && !failed.contains(&request)
        {
            self.fail(request, error).await;
        }
        self.results.release_tab(tab, generation);
        self.emit(Event::TabClosed { tab }).await;
    }

    async fn finish_connect(
        &mut self,
        request: Option<RequestId>,
        id: ConnectionId,
        result: Result<Arc<dyn Driver>, CoreError>,
    ) {
        let Some(entry) = self.connections.get_mut(&id) else {
            // @NOTE: the connection was deleted while connecting; its waiting executes were
            // already failed by the delete.
            if let Ok(driver) = result {
                driver.close().await;
            }
            // @NOTE: joined connects were already failed by the delete.
            if let Some(request) = request {
                self.fail(request, &CoreError::UnknownConnection).await;
            }
            return;
        };
        let waiting = std::mem::take(&mut entry.waiting_executes);
        let requests: Vec<RequestId> = request
            .into_iter()
            .chain(std::mem::take(&mut entry.joined_connects))
            .collect();
        match result {
            Ok(driver) if self.shutting_down => {
                entry.state = ConnectionState::Disconnected;
                driver.close().await;
                let error = CoreError::Connection("the core is shutting down".into());
                for request in requests {
                    self.fail(request, &error).await;
                }
                self.fail_waiting(waiting, &error_info(&error)).await;
            }
            Ok(driver) => {
                entry.driver = Some(driver.clone());
                entry.state = ConnectionState::Connected;
                self.emit(Event::ConnectionStateChanged {
                    id,
                    state: ConnectionState::Connected,
                })
                .await;
                for request in requests {
                    self.emit(Event::RequestCompleted { request }).await;
                }
                for pending in waiting {
                    self.forward(pending, driver.clone()).await;
                }
            }
            Err(error) => {
                entry.state = ConnectionState::Disconnected;
                self.emit(Event::ConnectionStateChanged {
                    id,
                    state: ConnectionState::Disconnected,
                })
                .await;
                for request in requests {
                    self.fail(request, &error).await;
                }
                self.fail_waiting(waiting, &error_info(&error)).await;
            }
        }
    }

    /// Closes every tab session first, then every pool (11.2).
    async fn shut_down(&mut self) {
        self.shutting_down = true;
        let handles: Vec<TabHandle> = self.tabs.drain().map(|(_, handle)| handle).collect();
        for handle in handles {
            self.close_handle(handle);
        }
        while let Some(joined) = self.tasks.join_next_with_id().await {
            self.complete(joined).await;
        }
        let drivers: Vec<Arc<dyn Driver>> = self
            .connections
            .values_mut()
            .filter_map(|entry| entry.driver.take())
            .collect();
        let closing = drivers.iter().map(|driver| driver.close());
        futures_util::future::join_all(closing).await;
    }
}

fn take_waiting_of(entry: &mut ConnectionEntry, tab: TabId) -> Vec<PendingExecute> {
    let (mine, others) = std::mem::take(&mut entry.waiting_executes)
        .into_iter()
        .partition(|pending| pending.tab == tab);
    entry.waiting_executes = others;
    mine
}

fn take_waiting_request(
    entry: &mut ConnectionEntry,
    tab: TabId,
    request: RequestId,
) -> Option<PendingExecute> {
    let position = entry
        .waiting_executes
        .iter()
        .position(|pending| pending.tab == tab && pending.request == request)?;
    Some(entry.waiting_executes.remove(position))
}

fn validate_settings(settings: &ConnectionSettings) -> Result<(), CoreError> {
    if settings.name.trim().is_empty() {
        return Err(CoreError::Configuration(
            "the connection needs a name".into(),
        ));
    }
    if settings.host.trim().is_empty() {
        return Err(CoreError::Configuration(
            "the connection needs a host".into(),
        ));
    }
    Ok(())
}

fn terminal_event(request: RequestId, outcome: Result<(), CoreError>) -> Event {
    match outcome {
        Ok(()) => Event::RequestCompleted { request },
        Err(error) => Event::RequestFailed {
            request,
            error: error_info(&error),
        },
    }
}

/// Secret store calls may block (a keychain can prompt), so they never run on the dispatcher.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, CoreError> + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| CoreError::Internal(format!("blocking task failed: {error}")))?
}

async fn resolve_password(
    secrets: Arc<dyn SecretStore>,
    id: Option<ConnectionId>,
    source: PasswordSource,
) -> Result<SecretString, CoreError> {
    match source {
        PasswordSource::Typed(password) => Ok(password),
        PasswordSource::None => Ok(SecretString::from(String::new())),
        PasswordSource::Saved => {
            let id = id.ok_or(CoreError::PasswordRequired)?;
            let saved = blocking(move || secrets.get(id)).await?;
            saved.ok_or(CoreError::PasswordRequired)
        }
    }
}

async fn children_of(
    driver: &dyn Driver,
    node: &MetadataNode,
) -> Result<Vec<MetadataItem>, CoreError> {
    match node {
        MetadataNode::Connection => {
            let schemas = driver.list_schemas().await?;
            Ok(schemas
                .into_iter()
                .map(|schema| item(MetadataItemKind::Schema, schema.name))
                .collect())
        }
        MetadataNode::Schema { schema } => {
            let tables = driver.list_tables(schema).await?;
            let views = driver.list_views(schema).await?;
            let tables = tables
                .into_iter()
                .map(|table| item(MetadataItemKind::Table, table.name));
            let views = views
                .into_iter()
                .map(|view| item(MetadataItemKind::View, view.name));
            Ok(tables.chain(views).collect())
        }
        MetadataNode::Table { schema, name } => {
            let table = TableRef {
                schema: schema.clone(),
                name: name.clone(),
            };
            let columns = driver.list_columns(&table).await?;
            Ok(columns
                .into_iter()
                .map(|column| {
                    let kind = MetadataItemKind::Column {
                        type_name: column.data_type,
                        nullable: column.nullable,
                    };
                    item(kind, column.name)
                })
                .collect())
        }
    }
}

fn item(kind: MetadataItemKind, name: String) -> MetadataItem {
    MetadataItem { kind, name }
}
