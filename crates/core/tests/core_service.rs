#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use dbased_commands::{
    Command, ConnectionId, ConnectionSettings, DatabaseKind, ErrorKind, Event, MetadataItem,
    MetadataItemKind, MetadataNode, PasswordSource, PasswordUpdate, RequestId, TlsMode, UiLink,
    bridge,
};
use dbased_core::{ConnectionStore, Core, CoreDependencies, DefaultConnector, InMemorySecretStore};
use secrecy::SecretString;
use support::{PASSWORD, TestDatabase};
use tokio::task::JoinHandle;

struct Service {
    link: UiLink,
    core: JoinHandle<()>,
    _directory: tempfile::TempDir,
}

impl Service {
    async fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let (link, core_link) = bridge(256, 1024);
        let core = Core::new(CoreDependencies {
            connection_store: ConnectionStore::new(directory.path().join("config.toml")),
            secret_store: Arc::new(InMemorySecretStore::default()),
            connector: Arc::new(DefaultConnector),
        });
        let core = tokio::spawn(core.run(core_link));
        let service = Self {
            link,
            core,
            _directory: directory,
        };
        assert!(matches!(
            service.next_event().await,
            Event::ConnectionsChanged { .. }
        ));
        service
    }

    async fn next_event(&self) -> Event {
        tokio::time::timeout(Duration::from_secs(30), self.link.events.recv())
            .await
            .expect("an event arrives in time")
            .expect("the event channel is open")
    }

    /// Sends a command and returns the terminal event of its request.
    async fn request(&self, command: Command, request: u64) -> Event {
        self.link.commands.send(command).await.unwrap();
        loop {
            let event = self.next_event().await;
            let done = match &event {
                Event::ConnectionSaved { request: found, .. }
                | Event::RequestCompleted { request: found }
                | Event::RequestFailed { request: found, .. }
                | Event::ChildrenLoaded { request: found, .. } => found.0 == request,
                _ => false,
            };
            if done {
                return event;
            }
        }
    }

    async fn save(&self, database: &TestDatabase, user: &str) -> ConnectionId {
        let settings = ConnectionSettings {
            name: "integration".into(),
            kind: DatabaseKind::Postgres,
            host: "127.0.0.1".into(),
            port: database.server.port,
            user: user.into(),
            database: database.name.clone(),
            tls: TlsMode::Disable,
        };
        let event = self
            .request(
                Command::SaveConnection {
                    request: RequestId(1),
                    id: None,
                    settings,
                    password: PasswordUpdate::Keep,
                },
                1,
            )
            .await;
        match event {
            Event::ConnectionSaved { id, .. } => id,
            other => panic!("expected ConnectionSaved, got {other:?}"),
        }
    }

    async fn children(
        &self,
        request: u64,
        id: ConnectionId,
        node: MetadataNode,
    ) -> Vec<MetadataItem> {
        let event = self
            .request(
                Command::LoadChildren {
                    request: RequestId(request),
                    connection: id,
                    node,
                },
                request,
            )
            .await;
        match event {
            Event::ChildrenLoaded { items, .. } => items,
            other => panic!("expected ChildrenLoaded, got {other:?}"),
        }
    }

    async fn stop(self) {
        drop(self.link.commands);
        tokio::time::timeout(Duration::from_secs(30), self.core)
            .await
            .expect("the core stops")
            .unwrap();
    }
}

fn typed_password() -> PasswordSource {
    PasswordSource::Typed(SecretString::from(PASSWORD))
}

async fn backends_in(database: &TestDatabase) -> i64 {
    let mut admin = database.admin().await;
    sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = $1")
        .bind(&database.name)
        .fetch_one(&mut admin)
        .await
        .unwrap()
}

#[tokio::test]
async fn connects_lists_schemas_and_tables_and_disconnect_closes_the_pool() {
    let database = TestDatabase::create().await;
    let mut owner = connect_to(&database).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(
        "CREATE TABLE people (id bigint NOT NULL, nickname text); \
         CREATE VIEW everyone AS SELECT id FROM people",
    ))
    .execute(&mut owner)
    .await
    .unwrap();
    sqlx::Connection::close(owner).await.unwrap();

    let service = Service::start().await;
    let id = service.save(&database, "postgres").await;

    let connected = service
        .request(
            Command::Connect {
                request: RequestId(2),
                id,
                password: typed_password(),
            },
            2,
        )
        .await;
    assert_eq!(
        connected,
        Event::RequestCompleted {
            request: RequestId(2)
        }
    );

    let schemas = service.children(3, id, MetadataNode::Connection).await;
    assert!(
        schemas
            .iter()
            .any(|item| item.kind == MetadataItemKind::Schema && item.name == "public")
    );

    let tables = service
        .children(
            4,
            id,
            MetadataNode::Schema {
                schema: "public".into(),
            },
        )
        .await;
    assert_eq!(
        tables,
        vec![
            MetadataItem {
                kind: MetadataItemKind::Table,
                name: "people".into()
            },
            MetadataItem {
                kind: MetadataItemKind::View,
                name: "everyone".into()
            },
        ]
    );

    let columns = service
        .children(
            5,
            id,
            MetadataNode::Table {
                schema: "public".into(),
                name: "people".into(),
            },
        )
        .await;
    assert_eq!(
        columns,
        vec![
            MetadataItem {
                kind: MetadataItemKind::Column {
                    type_name: "bigint".into(),
                    nullable: false
                },
                name: "id".into()
            },
            MetadataItem {
                kind: MetadataItemKind::Column {
                    type_name: "text".into(),
                    nullable: true
                },
                name: "nickname".into()
            },
        ]
    );
    assert!(
        backends_in(&database).await > 0,
        "the pool holds a connection"
    );

    let disconnected = service
        .request(
            Command::Disconnect {
                request: RequestId(6),
                id,
            },
            6,
        )
        .await;
    assert_eq!(
        disconnected,
        Event::RequestCompleted {
            request: RequestId(6)
        }
    );
    assert_eq!(backends_in(&database).await, 0, "the pool is closed");
    service.stop().await;
}

#[tokio::test]
async fn a_wrong_password_ends_the_request_with_authentication() {
    let database = TestDatabase::create().await;
    let service = Service::start().await;
    let id = service.save(&database, "postgres").await;

    let event = service
        .request(
            Command::Connect {
                request: RequestId(2),
                id,
                password: PasswordSource::Typed(SecretString::from("definitely-wrong")),
            },
            2,
        )
        .await;
    match event {
        Event::RequestFailed { error, .. } => assert_eq!(error.kind, ErrorKind::Authentication),
        other => panic!("expected RequestFailed, got {other:?}"),
    }
    service.stop().await;
}

#[tokio::test]
async fn shutdown_closes_the_pool_of_a_connected_connection() {
    let database = TestDatabase::create().await;
    let service = Service::start().await;
    let id = service.save(&database, "postgres").await;
    service
        .request(
            Command::Connect {
                request: RequestId(2),
                id,
                password: typed_password(),
            },
            2,
        )
        .await;
    service.children(3, id, MetadataNode::Connection).await;
    assert!(backends_in(&database).await > 0);

    service.stop().await;
    assert_eq!(backends_in(&database).await, 0);
}

#[tokio::test]
async fn test_connection_succeeds_and_leaves_nothing_open() {
    let database = TestDatabase::create().await;
    let service = Service::start().await;
    let settings = ConnectionSettings {
        name: "probe".into(),
        kind: DatabaseKind::Postgres,
        host: "127.0.0.1".into(),
        port: database.server.port,
        user: "postgres".into(),
        database: database.name.clone(),
        tls: TlsMode::Disable,
    };
    let event = service
        .request(
            Command::TestConnection {
                request: RequestId(1),
                id: None,
                settings,
                password: typed_password(),
            },
            1,
        )
        .await;
    assert_eq!(
        event,
        Event::RequestCompleted {
            request: RequestId(1)
        }
    );
    assert_eq!(backends_in(&database).await, 0);
    service.stop().await;
}

async fn connect_to(database: &TestDatabase) -> sqlx::PgConnection {
    use sqlx::ConnectOptions;
    sqlx::postgres::PgConnectOptions::new_without_pgpass()
        .host("127.0.0.1")
        .port(database.server.port)
        .username("postgres")
        .password(PASSWORD)
        .database(&database.name)
        .ssl_mode(sqlx::postgres::PgSslMode::Disable)
        .connect()
        .await
        .unwrap()
}
