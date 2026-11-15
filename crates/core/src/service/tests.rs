#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_channel::{Receiver, Sender};
use async_trait::async_trait;
use dbased_commands::{
    Command, ConnectionId, ConnectionSettings, ConnectionState, ConnectionSummary, DatabaseKind,
    ErrorKind, Event, ExecuteTarget, MetadataItem, MetadataItemKind, MetadataNode, PasswordSource,
    PasswordUpdate, RequestId, TabId, TlsMode, bridge,
};
use secrecy::{ExposeSecret, SecretString};
use tokio::task::JoinHandle;

use super::{
    ConnectionStore, Core, CoreDependencies, DriverConnector, InMemorySecretStore, SecretStore,
    error_info,
};
use crate::{
    Column, CoreError, DatabaseErrorInfo, Driver, PageRequest, Routine, Schema, Session, TableRef,
    ViewRef,
};

use super::tests_session::{ScriptedSession, SessionLog};

pub(super) const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

struct ScriptedDriver {
    closes: Arc<AtomicUsize>,
    sessions: Arc<SessionLog>,
}

#[async_trait]
impl Driver for ScriptedDriver {
    async fn open_session(&self) -> Result<Box<dyn Session>, CoreError> {
        self.sessions.opens.fetch_add(1, Ordering::SeqCst);
        self.sessions.order.lock().unwrap().push("session_open");
        Ok(Box::new(ScriptedSession {
            log: self.sessions.clone(),
        }))
    }

    async fn list_schemas(&self) -> Result<Vec<Schema>, CoreError> {
        Ok(vec![
            Schema {
                name: "public".into(),
            },
            Schema {
                name: "sales".into(),
            },
        ])
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableRef>, CoreError> {
        if schema == "broken" {
            return Err(CoreError::Database(DatabaseErrorInfo {
                sqlstate: "42501".into(),
                message: "permission denied".into(),
                detail: None,
                hint: None,
                position: None,
            }));
        }
        Ok(vec![TableRef {
            schema: schema.into(),
            name: "orders".into(),
        }])
    }

    async fn list_columns(&self, _table: &TableRef) -> Result<Vec<Column>, CoreError> {
        Ok(vec![Column {
            name: "id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }])
    }

    async fn list_views(&self, schema: &str) -> Result<Vec<ViewRef>, CoreError> {
        Ok(vec![ViewRef {
            schema: schema.into(),
            name: "recent_orders".into(),
        }])
    }

    async fn list_routines(&self, _schema: &str) -> Result<Vec<Routine>, CoreError> {
        Ok(Vec::new())
    }

    async fn close(&self) {
        self.sessions.order.lock().unwrap().push("driver_close");
        self.closes.fetch_add(1, Ordering::SeqCst);
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{identifier}\"")
    }

    fn paginate(&self, base_sql: &str, _page: PageRequest) -> String {
        base_sql.to_owned()
    }
}

#[derive(Default)]
pub(super) struct ScriptedConnector {
    pub(super) connects: AtomicUsize,
    pub(super) closes: Arc<AtomicUsize>,
    pub(super) sessions: Arc<SessionLog>,
    pub(super) required_password: Option<&'static str>,
}

#[async_trait]
impl DriverConnector for ScriptedConnector {
    async fn connect(
        &self,
        _settings: &ConnectionSettings,
        password: SecretString,
    ) -> Result<Arc<dyn Driver>, CoreError> {
        self.connects.fetch_add(1, Ordering::SeqCst);
        if let Some(required) = self.required_password
            && password.expose_secret() != required
        {
            return Err(CoreError::Authentication("wrong password".into()));
        }
        Ok(Arc::new(ScriptedDriver {
            closes: self.closes.clone(),
            sessions: self.sessions.clone(),
        }))
    }
}

pub(super) struct Harness {
    pub(super) commands: Sender<Command>,
    pub(super) events: Receiver<Event>,
    core: JoinHandle<()>,
    pub(super) config_path: PathBuf,
    pub(super) secrets: Arc<InMemorySecretStore>,
    pub(super) connector: Arc<ScriptedConnector>,
    initial: Vec<ConnectionSummary>,
    pub(super) _directory: Option<tempfile::TempDir>,
}

impl Harness {
    pub(super) async fn new() -> Self {
        Self::with_connector(ScriptedConnector::default()).await
    }

    pub(super) async fn with_connector(connector: ScriptedConnector) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dbased").join("config.toml");
        let mut harness = Self::start(
            path,
            Arc::new(InMemorySecretStore::default()),
            Arc::new(connector),
        )
        .await;
        harness._directory = Some(directory);
        harness
    }

    pub(super) async fn start(
        config_path: PathBuf,
        secrets: Arc<InMemorySecretStore>,
        connector: Arc<ScriptedConnector>,
    ) -> Self {
        Self::start_sized(config_path, secrets, connector, 1024).await
    }

    pub(super) async fn start_sized(
        config_path: PathBuf,
        secrets: Arc<InMemorySecretStore>,
        connector: Arc<ScriptedConnector>,
        event_capacity: usize,
    ) -> Self {
        let (ui, link) = bridge(256, event_capacity);
        let core = Core::new(CoreDependencies {
            connection_store: ConnectionStore::new(config_path.clone()),
            secret_store: secrets.clone(),
            connector: connector.clone(),
        });
        let core = tokio::spawn(core.run(link));
        let mut harness = Self {
            commands: ui.commands,
            events: ui.events,
            core,
            config_path,
            secrets,
            connector,
            initial: Vec::new(),
            _directory: None,
        };
        let Event::ConnectionsChanged { connections } = harness.next_event().await else {
            panic!("the first event is the loaded connection list");
        };
        harness.initial = connections;
        harness
    }

    pub(super) async fn next_event(&self) -> Event {
        tokio::time::timeout(EVENT_TIMEOUT, self.events.recv())
            .await
            .expect("an event arrives in time")
            .expect("the event channel is open")
    }

    pub(super) async fn send(&self, command: Command) {
        self.commands.send(command).await.unwrap();
    }

    /// Events up to and including the terminal event of `request`.
    pub(super) async fn until_terminal(&self, request: RequestId) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let event = self.next_event().await;
            let done = terminal_request(&event) == Some(request);
            seen.push(event);
            if done {
                return seen;
            }
        }
    }

    pub(super) async fn request(&self, command: Command, request: RequestId) -> Vec<Event> {
        self.send(command).await;
        self.until_terminal(request).await
    }

    pub(super) async fn save(
        &self,
        request: u64,
        id: Option<ConnectionId>,
        name: &str,
    ) -> ConnectionId {
        let events = self
            .request(
                Command::SaveConnection {
                    request: RequestId(request),
                    id,
                    settings: settings(name),
                    password: PasswordUpdate::Keep,
                },
                RequestId(request),
            )
            .await;
        match events.last() {
            Some(Event::ConnectionSaved { id, .. }) => *id,
            other => panic!("expected ConnectionSaved, got {other:?}"),
        }
    }

    pub(super) async fn connect(
        &self,
        request: u64,
        id: ConnectionId,
        password: PasswordSource,
    ) -> Event {
        let events = self
            .request(
                Command::Connect {
                    request: RequestId(request),
                    id,
                    password,
                },
                RequestId(request),
            )
            .await;
        events.into_iter().last().unwrap()
    }

    /// Drops the command sender and waits for `Core::run` to return.
    pub(super) async fn shut_down(self) -> Vec<Event> {
        drop(self.commands);
        tokio::time::timeout(EVENT_TIMEOUT, self.core)
            .await
            .expect("the core stops after the channel closes")
            .unwrap();
        let mut rest = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            rest.push(event);
        }
        rest
    }
}

pub(super) fn settings(name: &str) -> ConnectionSettings {
    ConnectionSettings {
        name: name.to_owned(),
        kind: DatabaseKind::Postgres,
        host: "localhost".to_owned(),
        port: 5432,
        user: "app".to_owned(),
        database: "app".to_owned(),
        tls: TlsMode::Disable,
    }
}

pub(super) fn terminal_request(event: &Event) -> Option<RequestId> {
    match event {
        Event::ConnectionSaved { request, .. }
        | Event::RequestCompleted { request }
        | Event::RequestFailed { request, .. }
        | Event::ChildrenLoaded { request, .. }
        | Event::ExecutionFinished { request, .. } => Some(*request),
        _ => None,
    }
}

pub(super) fn failed_kind(event: &Event) -> ErrorKind {
    match event {
        Event::RequestFailed { error, .. } => error.kind,
        other => panic!("expected RequestFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn connections_round_trip_through_the_config_file() {
    let harness = Harness::new().await;
    assert!(harness.initial.is_empty());

    let first = harness.save(1, None, "first").await;
    let second = harness.save(2, None, "second").await;
    assert_eq!((first, second), (ConnectionId(1), ConnectionId(2)));
    harness.save(3, Some(first), "renamed").await;

    let path = harness.config_path.clone();
    let secrets = harness.secrets.clone();
    let connector = harness.connector.clone();
    // @NOTE: the first core must be gone before a second one reads the same file.
    let mut keep_alive = harness;
    let tempdir = keep_alive._directory.take();
    keep_alive.shut_down().await;

    let restarted = Harness::start(path, secrets, connector).await;
    let names: Vec<_> = restarted
        .initial
        .iter()
        .map(|summary| (summary.id, summary.settings.name.as_str()))
        .collect();
    assert_eq!(names, vec![(first, "renamed"), (second, "second")]);
    assert!(
        restarted
            .initial
            .iter()
            .all(|summary| summary.state == ConnectionState::Disconnected)
    );

    let events = restarted
        .request(
            Command::DeleteConnection {
                request: RequestId(10),
                id: first,
            },
            RequestId(10),
        )
        .await;
    assert!(matches!(
        &events[0],
        Event::ConnectionsChanged { connections } if connections.len() == 1
    ));
    assert_eq!(
        events[1],
        Event::RequestCompleted {
            request: RequestId(10)
        }
    );
    let stored = ConnectionStore::new(restarted.config_path.clone())
        .load()
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id, second);
    drop(tempdir);
}

#[tokio::test]
async fn save_validates_and_rejects_unknown_ids() {
    let harness = Harness::new().await;
    let unknown = harness
        .request(
            Command::SaveConnection {
                request: RequestId(1),
                id: Some(ConnectionId(99)),
                settings: settings("x"),
                password: PasswordUpdate::Keep,
            },
            RequestId(1),
        )
        .await;
    assert_eq!(failed_kind(&unknown[0]), ErrorKind::UnknownConnection);

    let mut nameless = settings("");
    nameless.name = "  ".to_owned();
    let invalid = harness
        .request(
            Command::SaveConnection {
                request: RequestId(2),
                id: None,
                settings: nameless,
                password: PasswordUpdate::Keep,
            },
            RequestId(2),
        )
        .await;
    assert_eq!(failed_kind(&invalid[0]), ErrorKind::Configuration);
}

#[tokio::test]
async fn saved_passwords_drive_the_password_required_path() {
    let connector = ScriptedConnector {
        required_password: Some("hunter2"),
        ..ScriptedConnector::default()
    };
    let harness = Harness::with_connector(connector).await;
    let id = harness.save(1, None, "db").await;

    let missing = harness.connect(2, id, PasswordSource::Saved).await;
    assert_eq!(failed_kind(&missing), ErrorKind::PasswordRequired);
    assert_eq!(harness.connector.connects.load(Ordering::SeqCst), 0);

    let events = harness
        .request(
            Command::SaveConnection {
                request: RequestId(3),
                id: Some(id),
                settings: settings("db"),
                password: PasswordUpdate::Save(SecretString::from("hunter2")),
            },
            RequestId(3),
        )
        .await;
    assert!(matches!(
        &events[0],
        Event::ConnectionsChanged { connections } if connections[0].password_saved
    ));
    assert_eq!(
        harness.secrets.get(id).unwrap().unwrap().expose_secret(),
        "hunter2"
    );

    let connected = harness.connect(4, id, PasswordSource::Saved).await;
    assert_eq!(
        connected,
        Event::RequestCompleted {
            request: RequestId(4)
        }
    );

    harness
        .request(
            Command::Disconnect {
                request: RequestId(5),
                id,
            },
            RequestId(5),
        )
        .await;
    let wrong = harness
        .connect(6, id, PasswordSource::Typed(SecretString::from("nope")))
        .await;
    assert_eq!(failed_kind(&wrong), ErrorKind::Authentication);

    let forgotten = harness
        .request(
            Command::SaveConnection {
                request: RequestId(7),
                id: Some(id),
                settings: settings("db"),
                password: PasswordUpdate::Forget,
            },
            RequestId(7),
        )
        .await;
    assert!(matches!(
        &forgotten[0],
        Event::ConnectionsChanged { connections } if !connections[0].password_saved
    ));
    assert!(harness.secrets.get(id).unwrap().is_none());
}

#[tokio::test]
async fn connect_and_disconnect_report_state_and_close_the_driver() {
    let harness = Harness::new().await;
    let id = harness.save(1, None, "db").await;

    let events = harness
        .request(
            Command::Connect {
                request: RequestId(2),
                id,
                password: PasswordSource::None,
            },
            RequestId(2),
        )
        .await;
    assert_eq!(
        events,
        vec![
            Event::ConnectionStateChanged {
                id,
                state: ConnectionState::Connecting
            },
            Event::ConnectionStateChanged {
                id,
                state: ConnectionState::Connected
            },
            Event::RequestCompleted {
                request: RequestId(2)
            },
        ]
    );
    assert_eq!(harness.connector.closes.load(Ordering::SeqCst), 0);

    let events = harness
        .request(
            Command::Disconnect {
                request: RequestId(3),
                id,
            },
            RequestId(3),
        )
        .await;
    assert_eq!(
        events,
        vec![
            Event::ConnectionStateChanged {
                id,
                state: ConnectionState::Disconnected
            },
            Event::RequestCompleted {
                request: RequestId(3)
            },
        ]
    );
    assert_eq!(harness.connector.closes.load(Ordering::SeqCst), 1);

    // Disconnecting an already disconnected connection still ends the request.
    let again = harness
        .request(
            Command::Disconnect {
                request: RequestId(4),
                id,
            },
            RequestId(4),
        )
        .await;
    assert_eq!(
        again,
        vec![Event::RequestCompleted {
            request: RequestId(4)
        }]
    );
}

#[tokio::test]
async fn test_connection_uses_a_throwaway_driver_and_closes_it() {
    let harness = Harness::new().await;
    let events = harness
        .request(
            Command::TestConnection {
                request: RequestId(1),
                id: None,
                settings: settings("unsaved"),
                password: PasswordSource::None,
            },
            RequestId(1),
        )
        .await;
    assert_eq!(
        events,
        vec![Event::RequestCompleted {
            request: RequestId(1)
        }]
    );
    assert_eq!(harness.connector.connects.load(Ordering::SeqCst), 1);
    assert_eq!(harness.connector.closes.load(Ordering::SeqCst), 1);

    let events = harness
        .request(
            Command::TestConnection {
                request: RequestId(2),
                id: None,
                settings: settings("unsaved"),
                password: PasswordSource::Saved,
            },
            RequestId(2),
        )
        .await;
    assert_eq!(failed_kind(&events[0]), ErrorKind::PasswordRequired);
}

#[tokio::test]
async fn metadata_requests_walk_the_tree() {
    let harness = Harness::new().await;
    let id = harness.save(1, None, "db").await;

    let before = harness
        .request(
            Command::LoadChildren {
                request: RequestId(2),
                connection: id,
                node: MetadataNode::Connection,
            },
            RequestId(2),
        )
        .await;
    assert_eq!(failed_kind(&before[0]), ErrorKind::Connection);

    harness.connect(3, id, PasswordSource::None).await;
    let schema_items: Vec<MetadataItem> = ["public", "sales"]
        .into_iter()
        .map(|name| MetadataItem {
            kind: MetadataItemKind::Schema,
            name: name.to_owned(),
        })
        .collect();

    let schemas = harness
        .request(
            Command::LoadChildren {
                request: RequestId(4),
                connection: id,
                node: MetadataNode::Connection,
            },
            RequestId(4),
        )
        .await;
    assert_eq!(
        schemas[0],
        Event::ChildrenLoaded {
            request: RequestId(4),
            connection: id,
            node: MetadataNode::Connection,
            items: schema_items,
        }
    );

    let node = MetadataNode::Schema {
        schema: "sales".into(),
    };
    let tables = harness
        .request(
            Command::LoadChildren {
                request: RequestId(5),
                connection: id,
                node: node.clone(),
            },
            RequestId(5),
        )
        .await;
    let expected = vec![
        MetadataItem {
            kind: MetadataItemKind::Table,
            name: "orders".into(),
        },
        MetadataItem {
            kind: MetadataItemKind::View,
            name: "recent_orders".into(),
        },
    ];
    assert!(matches!(
        &tables[0],
        Event::ChildrenLoaded { items, node: reply, .. } if *items == expected && *reply == node
    ));

    let columns = harness
        .request(
            Command::LoadChildren {
                request: RequestId(6),
                connection: id,
                node: MetadataNode::Table {
                    schema: "sales".into(),
                    name: "orders".into(),
                },
            },
            RequestId(6),
        )
        .await;
    assert!(matches!(
        &columns[0],
        Event::ChildrenLoaded { items, .. } if items == &vec![MetadataItem {
            kind: MetadataItemKind::Column { type_name: "bigint".into(), nullable: false },
            name: "id".into(),
        }]
    ));

    let failed = harness
        .request(
            Command::LoadChildren {
                request: RequestId(7),
                connection: id,
                node: MetadataNode::Schema {
                    schema: "broken".into(),
                },
            },
            RequestId(7),
        )
        .await;
    match &failed[0] {
        Event::RequestFailed { error, .. } => {
            assert_eq!(error.kind, ErrorKind::Database);
            assert_eq!(error.database.as_ref().unwrap().sqlstate, "42501");
        }
        other => panic!("expected RequestFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn execute_on_an_unknown_tab_ends_the_request() {
    let harness = Harness::new().await;
    let events = harness
        .request(
            Command::Execute {
                request: RequestId(1),
                tab: TabId(1),
                script: "select 1".into(),
                target: ExecuteTarget::All,
            },
            RequestId(1),
        )
        .await;
    assert_eq!(failed_kind(&events[0]), ErrorKind::UnknownTab);
}

#[tokio::test]
async fn every_request_ends_with_exactly_one_terminal_event() {
    let harness = Harness::new().await;
    let id = harness.save(1, None, "db").await;
    let commands = vec![
        Command::Connect {
            request: RequestId(2),
            id,
            password: PasswordSource::None,
        },
        Command::Connect {
            request: RequestId(3),
            id: ConnectionId(77),
            password: PasswordSource::None,
        },
        Command::LoadChildren {
            request: RequestId(4),
            connection: id,
            node: MetadataNode::Connection,
        },
        Command::TestConnection {
            request: RequestId(5),
            id: Some(id),
            settings: settings("db"),
            password: PasswordSource::Saved,
        },
        Command::Execute {
            request: RequestId(6),
            tab: TabId(1),
            script: "select 1".into(),
            target: ExecuteTarget::All,
        },
        Command::Disconnect {
            request: RequestId(7),
            id,
        },
        Command::DeleteConnection {
            request: RequestId(8),
            id: ConnectionId(77),
        },
        Command::DeleteConnection {
            request: RequestId(9),
            id,
        },
    ];
    let expected = commands.len() as u64;
    for command in commands {
        harness.send(command).await;
    }
    let remaining = harness.shut_down().await;

    let mut terminals: HashMap<RequestId, usize> = HashMap::new();
    for event in &remaining {
        if let Some(request) = terminal_request(event) {
            *terminals.entry(request).or_default() += 1;
        }
    }
    for request in 2..2 + expected {
        assert_eq!(
            terminals.get(&RequestId(request)),
            Some(&1),
            "request {request} needs exactly one terminal event, got {remaining:?}"
        );
    }
}

#[tokio::test]
async fn closing_the_command_channel_closes_every_driver() {
    let harness = Harness::new().await;
    let first = harness.save(1, None, "one").await;
    let second = harness.save(2, None, "two").await;
    harness.connect(3, first, PasswordSource::None).await;
    harness.connect(4, second, PasswordSource::None).await;
    let connector = harness.connector.clone();
    assert_eq!(connector.closes.load(Ordering::SeqCst), 0);

    harness.shut_down().await;
    assert_eq!(connector.closes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_connect_still_running_at_shutdown_is_closed_not_leaked() {
    let harness = Harness::new().await;
    let id = harness.save(1, None, "db").await;
    harness
        .send(Command::Connect {
            request: RequestId(2),
            id,
            password: PasswordSource::None,
        })
        .await;
    let connector = harness.connector.clone();
    harness.shut_down().await;
    let connects = connector.connects.load(Ordering::SeqCst);
    assert_eq!(connector.closes.load(Ordering::SeqCst), connects);
}

#[tokio::test]
async fn an_unreadable_config_is_reported_and_never_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    std::fs::write(&path, "this is = not [valid toml").unwrap();

    let (ui, link) = bridge(256, 1024);
    let core = Core::new(CoreDependencies {
        connection_store: ConnectionStore::new(path.clone()),
        secret_store: Arc::new(InMemorySecretStore::default()),
        connector: Arc::new(ScriptedConnector::default()),
    });
    let core = tokio::spawn(core.run(link));
    let first = ui.events.recv().await.unwrap();
    assert!(matches!(first, Event::Error { error } if error.kind == ErrorKind::Storage));
    let second = ui.events.recv().await.unwrap();
    assert!(matches!(second, Event::ConnectionsChanged { connections } if connections.is_empty()));

    ui.commands
        .send(Command::SaveConnection {
            request: RequestId(1),
            id: None,
            settings: settings("x"),
            password: PasswordUpdate::Keep,
        })
        .await
        .unwrap();
    let reply = ui.events.recv().await.unwrap();
    assert_eq!(failed_kind(&reply), ErrorKind::Storage);
    drop(ui.commands);
    core.await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "this is = not [valid toml"
    );
}

#[test]
fn the_store_keeps_other_tables_and_leaves_no_temp_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nested").join("config.toml");
    let store = ConnectionStore::new(path.clone());
    assert!(store.load().unwrap().is_empty());

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "[editor]\nfont_size = 14\n").unwrap();
    let connection = super::StoredConnection {
        id: ConnectionId(3),
        settings: settings("kept"),
        password_saved: true,
    };
    store.save(std::slice::from_ref(&connection)).unwrap();

    assert_eq!(store.load().unwrap(), vec![connection]);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("font_size = 14"), "{text}");
    assert!(!text.contains("password ="), "{text}");
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, vec![std::ffi::OsString::from("config.toml")]);
}

#[test]
fn every_core_error_maps_to_its_error_kind() {
    use crate::CloseReason;
    let cases: Vec<(CoreError, ErrorKind)> = vec![
        (CoreError::Connection("x".into()), ErrorKind::Connection),
        (
            CoreError::Authentication("x".into()),
            ErrorKind::Authentication,
        ),
        (CoreError::Tls("x".into()), ErrorKind::Tls),
        (CoreError::Timeout("x"), ErrorKind::Timeout),
        (
            CoreError::Configuration("x".into()),
            ErrorKind::Configuration,
        ),
        (
            CoreError::Database(DatabaseErrorInfo {
                sqlstate: "42P01".into(),
                message: "no such table".into(),
                detail: Some("detail".into()),
                hint: Some("hint".into()),
                position: Some(8),
            }),
            ErrorKind::Database,
        ),
        (CoreError::Protocol("x".into()), ErrorKind::Protocol),
        (
            CoreError::SessionClosed(CloseReason::ConnectionError),
            ErrorKind::SessionClosed,
        ),
        (CoreError::CancelFailed("x".into()), ErrorKind::CancelFailed),
        (CoreError::NotImplemented("x"), ErrorKind::NotImplemented),
        (CoreError::Unsupported("x"), ErrorKind::Unsupported),
        (CoreError::PasswordRequired, ErrorKind::PasswordRequired),
        (CoreError::UnknownConnection, ErrorKind::UnknownConnection),
        (CoreError::UnknownTab, ErrorKind::UnknownTab),
        (CoreError::TabBusy, ErrorKind::TabBusy),
        (CoreError::EmptyScript, ErrorKind::EmptyScript),
        (CoreError::Storage("x".into()), ErrorKind::Storage),
        (CoreError::SecretStore("x".into()), ErrorKind::SecretStore),
        (CoreError::Internal("x".into()), ErrorKind::Internal),
    ];
    for (error, kind) in cases {
        let info = error_info(&error);
        assert_eq!(info.kind, kind, "{error:?}");
        assert_eq!(info.message, error.to_string());
        assert_eq!(info.database.is_some(), kind == ErrorKind::Database);
    }

    let info = error_info(&CoreError::Database(DatabaseErrorInfo {
        sqlstate: "42P01".into(),
        message: "no such table".into(),
        detail: Some("detail".into()),
        hint: Some("hint".into()),
        position: Some(8),
    }));
    let database = info.database.unwrap();
    assert_eq!(database.sqlstate, "42P01");
    assert_eq!(database.position, Some(8));
    assert_eq!(database.hint.as_deref(), Some("hint"));
}
