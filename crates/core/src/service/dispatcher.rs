use std::collections::BTreeMap;
use std::sync::Arc;

use async_channel::{Receiver, Sender};
use dbased_commands::{
    Command, ConnectionId, ConnectionSettings, ConnectionState, ConnectionSummary, Event,
    MetadataItem, MetadataItemKind, MetadataNode, PasswordSource, PasswordUpdate, RequestId,
};
use secrecy::SecretString;
use tokio::task::{JoinError, JoinSet};

use super::error_info::error_info;
use super::store::StoredConnection;
use super::{ConnectionStore, CoreDependencies, DriverConnector, SecretStore};
use crate::{CoreError, Driver, TableRef};

struct ConnectionEntry {
    settings: ConnectionSettings,
    password_saved: bool,
    state: ConnectionState,
    driver: Option<Arc<dyn Driver>>,
}

/// What a spawned task hands back to the dispatcher. Tasks that only emit their own terminal
/// event return `Done`.
enum Completion {
    Connected {
        request: RequestId,
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
    // @NOTE: set when the config file could not be read, so a save never overwrites it blindly.
    storage_blocked: Option<String>,
    tasks: JoinSet<Completion>,
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
            storage_blocked: None,
            tasks: JoinSet::new(),
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
                Some(joined) = self.tasks.join_next() => self.complete(joined).await,
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
            // @TODO: execution lands with tab actors (docs/PROJECT.md 11.5 step 4).
            Command::Execute { request, .. } => {
                let error = CoreError::NotImplemented("statement execution");
                self.fail(request, &error).await;
            }
            // @TODO: tab and result commands carry no request id; they are ignored until the
            // tab actors exist.
            Command::OpenTab { .. }
            | Command::CloseTab { .. }
            | Command::Cancel { .. }
            | Command::FetchRows { .. }
            | Command::ReleaseResult { .. } => {}
        }
    }

    async fn emit(&self, event: Event) {
        // @NOTE: a closed event channel means the UI is gone; the shutdown path handles the rest.
        self.events.send(event).await.ok();
    }

    async fn fail(&self, request: RequestId, error: &CoreError) {
        self.emit(Event::RequestFailed {
            request,
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
        let driver = self.connections.remove(&id).and_then(|entry| entry.driver);
        let secret_result = self.delete_secret(id).await;
        self.emit_connections_changed().await;

        // @TODO: close the connection's tabs (TabClosed each) once tab actors exist.
        let close_reply = secret_result.is_ok().then_some(request);
        match (driver, &secret_result) {
            (Some(driver), _) => self.spawn_close(driver, close_reply),
            (None, Ok(())) => self.emit(Event::RequestCompleted { request }).await,
            (None, Err(_)) => {}
        }
        if let Err(error) = secret_result {
            self.fail(request, &error).await;
        }
    }

    fn spawn_close(&mut self, driver: Arc<dyn Driver>, request: Option<RequestId>) {
        self.tasks.spawn(async move {
            driver.close().await;
            Completion::Closed { request }
        });
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
        self.tasks.spawn(async move {
            let outcome = async {
                let password = resolve_password(secrets, id, password).await?;
                let driver = connector.connect(&settings, password).await?;
                driver.close().await;
                Ok::<(), CoreError>(())
            }
            .await;
            events.send(terminal_event(request, outcome)).await.ok();
            Completion::Done
        });
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
                let error = CoreError::Internal("a connection attempt is already running".into());
                self.fail(request, &error).await;
                return;
            }
            ConnectionState::Disconnected => {}
        }
        entry.state = ConnectionState::Connecting;
        let settings = entry.settings.clone();
        self.emit(Event::ConnectionStateChanged {
            id,
            state: ConnectionState::Connecting,
        })
        .await;

        let secrets = self.secrets.clone();
        let connector = self.connector.clone();
        self.tasks.spawn(async move {
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
        });
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
        // @TODO: release every tab session of this connection before closing the pool.
        self.emit(Event::ConnectionStateChanged {
            id,
            state: ConnectionState::Disconnected,
        })
        .await;
        self.spawn_close(driver, Some(request));
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
        self.tasks.spawn(async move {
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
        });
    }

    async fn complete(&mut self, joined: Result<Completion, JoinError>) {
        // @NOTE: a panicked task cannot say which request it served; it is dropped here.
        let Ok(completion) = joined else { return };
        match completion {
            Completion::Done => {}
            Completion::Closed { request } => {
                if let Some(request) = request {
                    self.emit(Event::RequestCompleted { request }).await;
                }
            }
            Completion::Connected {
                request,
                id,
                result,
            } => self.finish_connect(request, id, result).await,
        }
    }

    async fn finish_connect(
        &mut self,
        request: RequestId,
        id: ConnectionId,
        result: Result<Arc<dyn Driver>, CoreError>,
    ) {
        let Some(entry) = self.connections.get_mut(&id) else {
            // @NOTE: the connection was deleted while connecting.
            if let Ok(driver) = result {
                driver.close().await;
            }
            self.fail(request, &CoreError::UnknownConnection).await;
            return;
        };
        match result {
            Ok(driver) if self.shutting_down => {
                entry.state = ConnectionState::Disconnected;
                driver.close().await;
            }
            Ok(driver) => {
                entry.driver = Some(driver);
                entry.state = ConnectionState::Connected;
                self.emit(Event::ConnectionStateChanged {
                    id,
                    state: ConnectionState::Connected,
                })
                .await;
                self.emit(Event::RequestCompleted { request }).await;
            }
            Err(error) => {
                entry.state = ConnectionState::Disconnected;
                self.emit(Event::ConnectionStateChanged {
                    id,
                    state: ConnectionState::Disconnected,
                })
                .await;
                self.fail(request, &error).await;
            }
        }
    }

    /// Lets in-flight tasks finish, then closes every pool.
    async fn shut_down(&mut self) {
        self.shutting_down = true;
        while let Some(joined) = self.tasks.join_next().await {
            self.complete(joined).await;
        }
        // @TODO: close every tab session before the pools (sessions first, docs/PROJECT.md 11.2).
        let drivers: Vec<Arc<dyn Driver>> = self
            .connections
            .values_mut()
            .filter_map(|entry| entry.driver.take())
            .collect();
        let closing = drivers.iter().map(|driver| driver.close());
        futures_util::future::join_all(closing).await;
    }
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
