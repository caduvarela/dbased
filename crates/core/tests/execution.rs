#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dbased_commands::{
    Command, ConnectionId, ConnectionSettings, DatabaseKind, ErrorKind, Event, ExecuteTarget,
    ExecutionStatus, PasswordUpdate, RequestId, ResultId, ResultOutcome, ResultStatus,
    SessionState, StatementSpan, TabId, TlsMode, UiLink, Value, bridge,
};
use dbased_core::drivers::postgres::{PostgresConfig, PostgresDriver, TlsMode as PostgresTls};
use dbased_core::{
    ConnectionStore, Core, CoreDependencies, CoreError, DefaultConnector, Driver, DriverConnector,
    DriverFactory, InMemorySecretStore, SessionLimits,
};
use secrecy::SecretString;
use support::{PASSWORD, TestDatabase};
use tokio::task::JoinHandle;

const TAB: TabId = TabId(1);

/// Builds the Postgres driver like the default connector, with small result caps.
struct CappedConnector {
    limits: SessionLimits,
}

#[async_trait]
impl DriverConnector for CappedConnector {
    async fn connect(
        &self,
        settings: &ConnectionSettings,
        password: SecretString,
    ) -> Result<Arc<dyn Driver>, CoreError> {
        let mut config = PostgresConfig::new(
            settings.host.clone(),
            settings.port,
            settings.user.clone(),
            password,
            settings.database.clone(),
        );
        config.tls = PostgresTls::Disable;
        config.limits = self.limits;
        let driver = PostgresDriver::connect(config).await?;
        Ok(Arc::from(driver))
    }
}

struct Service {
    link: UiLink,
    core: JoinHandle<()>,
    connection: ConnectionId,
    next_request: u64,
    _directory: tempfile::TempDir,
}

impl Service {
    async fn start(database: &TestDatabase) -> Self {
        Self::start_with(database, Arc::new(DefaultConnector)).await
    }

    async fn start_with(database: &TestDatabase, connector: Arc<dyn DriverConnector>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let (link, core_link) = bridge(256, 1024);
        let core = Core::new(CoreDependencies {
            connection_store: ConnectionStore::new(directory.path().join("config.toml")),
            secret_store: Arc::new(InMemorySecretStore::default()),
            connector,
        });
        let mut service = Self {
            link,
            core: tokio::spawn(core.run(core_link)),
            connection: ConnectionId(0),
            next_request: 100,
            _directory: directory,
        };
        assert!(matches!(
            service.next_event().await,
            Event::ConnectionsChanged { .. }
        ));
        let settings = ConnectionSettings {
            name: "execution".into(),
            kind: DatabaseKind::Postgres,
            host: "127.0.0.1".into(),
            port: database.server.port,
            user: "postgres".into(),
            database: database.name.clone(),
            tls: TlsMode::Disable,
        };
        let request = service.request_id();
        service
            .send(Command::SaveConnection {
                request,
                id: None,
                settings,
                // @NOTE: saved so that running connects on its own (11.2).
                password: PasswordUpdate::Save(SecretString::from(PASSWORD)),
            })
            .await;
        let Event::ConnectionSaved { id, .. } =
            service.until_terminal(request).await.pop().unwrap()
        else {
            panic!("the connection was not saved");
        };
        service.connection = id;
        service
            .send(Command::OpenTab {
                tab: TAB,
                connection: id,
            })
            .await;
        service
    }

    fn request_id(&mut self) -> RequestId {
        self.next_request += 1;
        RequestId(self.next_request)
    }

    async fn send(&self, command: Command) {
        self.link.commands.send(command).await.unwrap();
    }

    async fn next_event(&self) -> Event {
        tokio::time::timeout(Duration::from_secs(60), self.link.events.recv())
            .await
            .expect("an event arrives in time")
            .expect("the event channel is open")
    }

    async fn until_terminal(&self, request: RequestId) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let event = self.next_event().await;
            let done = match &event {
                Event::ConnectionSaved { request: found, .. }
                | Event::RequestCompleted { request: found }
                | Event::RequestFailed { request: found, .. }
                | Event::ExecutionFinished { request: found, .. } => *found == request,
                _ => false,
            };
            seen.push(event);
            if done {
                return seen;
            }
        }
    }

    async fn execute(&mut self, script: &str, target: ExecuteTarget) -> (RequestId, Vec<Event>) {
        let request = self.start_execute(script, target).await;
        (request, self.until_terminal(request).await)
    }

    async fn start_execute(&mut self, script: &str, target: ExecuteTarget) -> RequestId {
        let request = self.request_id();
        self.send(Command::Execute {
            request,
            tab: TAB,
            script: script.to_owned(),
            target,
        })
        .await;
        request
    }

    async fn fetch(&self, request: RequestId, statement: u32, count: u32) -> Vec<Vec<Value>> {
        let result = ResultId { request, statement };
        self.send(Command::FetchRows {
            result,
            start: 0,
            count,
        })
        .await;
        loop {
            if let Event::RowsWindow {
                result: found,
                rows,
                ..
            } = self.next_event().await
                && found == result
            {
                return rows;
            }
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

async fn create_table(database: &TestDatabase, ddl: &str) {
    let mut owner = connect_to(database).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl.to_owned()))
        .execute(&mut owner)
        .await
        .unwrap();
    sqlx::Connection::close(owner).await.unwrap();
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

async fn table_exists(database: &TestDatabase, name: &str) -> bool {
    let mut owner = connect_to(database).await;
    let exists = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(name)
        .fetch_one(&mut owner)
        .await
        .unwrap();
    sqlx::Connection::close(owner).await.unwrap();
    exists
}

async fn backends_in(database: &TestDatabase) -> i64 {
    let mut admin = database.admin().await;
    sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = $1")
        .bind(&database.name)
        .fetch_one(&mut admin)
        .await
        .unwrap()
}

fn spans(events: &[Event]) -> Vec<StatementSpan> {
    events
        .iter()
        .find_map(|event| match event {
            Event::ExecutionStarted { statements, .. } => Some(statements.clone()),
            _ => None,
        })
        .expect("ExecutionStarted")
}

fn outcome(events: &[Event], statement: u32) -> ResultOutcome {
    events
        .iter()
        .find_map(|event| match event {
            Event::ResultFinished { result, outcome } if result.statement == statement => {
                Some(outcome.clone())
            }
            _ => None,
        })
        .expect("ResultFinished")
}

fn status(events: &[Event]) -> ExecutionStatus {
    match events.last() {
        Some(Event::ExecutionFinished { status, .. }) => status.clone(),
        other => panic!("expected ExecutionFinished, got {other:?}"),
    }
}

#[tokio::test]
async fn run_current_on_the_first_statement_never_reaches_the_second() {
    let database = TestDatabase::create().await;
    create_table(&database, "CREATE TABLE t (id int)").await;
    let mut service = Service::start(&database).await;

    let script = "SELECT 1; DROP TABLE t";
    let (_, events) = service
        .execute(script, ExecuteTarget::Current { cursor: 3 })
        .await;
    assert_eq!(status(&events), ExecutionStatus::Completed);
    assert_eq!(spans(&events).len(), 1);
    assert!(table_exists(&database, "t").await, "t must still exist");

    // Control: the same script run in full does drop it, one statement at a time.
    let (_, events) = service.execute(script, ExecuteTarget::All).await;
    assert_eq!(status(&events), ExecutionStatus::Completed);
    assert!(!table_exists(&database, "t").await);
    service.stop().await;
}

#[tokio::test]
async fn several_statements_give_several_results_with_their_own_columns() {
    let database = TestDatabase::create().await;
    let mut service = Service::start(&database).await;

    let (request, events) = service
        .execute(
            "SELECT 1 AS a; SELECT 'x' AS b, 2 AS c; CREATE TEMP TABLE scratch (id int)",
            ExecuteTarget::All,
        )
        .await;
    assert_eq!(status(&events), ExecutionStatus::Completed);
    assert!(events.iter().any(|event| matches!(
        event,
        Event::SessionStateChanged {
            state: SessionState::Open,
            ..
        }
    )));
    let columns: Vec<Vec<String>> = events
        .iter()
        .filter_map(|event| match event {
            Event::ResultStarted { columns, .. } => {
                Some(columns.iter().map(|column| column.name.clone()).collect())
            }
            _ => None,
        })
        .collect();
    let expected: [&[&str]; 3] = [&["a"], &["b", "c"], &[]];
    assert_eq!(columns, expected);
    assert_eq!(service.fetch(request, 1, 10).await.len(), 1);
    assert_eq!(outcome(&events, 2).rows_affected, Some(0));

    // The tab keeps its session: the temp table is still there.
    let (_, events) = service
        .execute("SELECT count(*) FROM scratch", ExecuteTarget::All)
        .await;
    assert_eq!(status(&events), ExecutionStatus::Completed);
    service.stop().await;
}

#[tokio::test]
async fn a_failed_statement_stops_the_script_and_the_error_position_maps_into_the_script() {
    let database = TestDatabase::create().await;
    let mut service = Service::start(&database).await;

    let script = "SELECT 1;\nSELECT * FROM missing_table;\nSELECT 3";
    let (_, events) = service.execute(script, ExecuteTarget::All).await;
    assert_eq!(
        status(&events),
        ExecutionStatus::StoppedOnError { statement: 1 }
    );
    let failed = outcome(&events, 1);
    assert_eq!(failed.status, ResultStatus::Failed);
    let error = failed.error.expect("database error");
    assert_eq!(error.sqlstate, "42P01");

    // @NOTE: the position is 1-based characters into the statement; ASCII here, so bytes match.
    let span = spans(&events)[1];
    let position = error.position.expect("position") as usize;
    let offset = span.start + position - 1;
    assert!(
        script[offset..].starts_with("missing_table"),
        "{}",
        &script[offset..]
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::ResultStarted { .. }))
            .count(),
        2,
        "the third statement never started"
    );
    service.stop().await;
}

#[tokio::test]
async fn cancelling_pg_sleep_in_a_script_stops_it_and_keeps_the_session() {
    let database = TestDatabase::create().await;
    let mut service = Service::start(&database).await;

    let request = service
        .start_execute(
            "SET application_name = 'kept'; SELECT pg_sleep(60); SELECT 3",
            ExecuteTarget::All,
        )
        .await;
    // @NOTE: columns arrive with the first row, so a sleeping statement has no ResultStarted;
    // the finished first statement plus a short pause means the sleep is running.
    loop {
        if let Event::ResultFinished { result, .. } = service.next_event().await
            && result.statement == 0
        {
            break;
        }
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    service.send(Command::Cancel { tab: TAB, request }).await;
    let events = service.until_terminal(request).await;
    assert_eq!(status(&events), ExecutionStatus::Cancelled { statement: 1 });
    assert_eq!(outcome(&events, 1).status, ResultStatus::Cancelled);

    // Clean cancel: same session, state intact.
    let (request, events) = service
        .execute("SHOW application_name", ExecuteTarget::All)
        .await;
    assert_eq!(status(&events), ExecutionStatus::Completed);
    assert_eq!(
        service.fetch(request, 0, 1).await,
        vec![vec![Value::Text("kept".into())]]
    );
    service.stop().await;
}

#[tokio::test]
async fn the_row_cap_truncates_one_statement_and_the_script_continues() {
    let database = TestDatabase::create().await;
    let connector = CappedConnector {
        limits: SessionLimits {
            max_rows: 10,
            max_bytes: 512 * 1024 * 1024,
        },
    };
    let mut service = Service::start_with(&database, Arc::new(connector)).await;

    let (request, events) = service
        .execute(
            "SELECT generate_series(1, 1000) AS n; SELECT 7 AS seven",
            ExecuteTarget::All,
        )
        .await;
    assert_eq!(status(&events), ExecutionStatus::Completed);
    let capped = outcome(&events, 0);
    assert_eq!(capped.status, ResultStatus::Truncated);
    assert!(capped.partial);
    assert_eq!(capped.rows_received, 10);
    assert_eq!(outcome(&events, 1).status, ResultStatus::Completed);
    assert_eq!(service.fetch(request, 0, 50).await.len(), 10);
    assert_eq!(
        service.fetch(request, 1, 50).await,
        vec![vec![Value::Int(7)]]
    );
    service.stop().await;
}

#[tokio::test]
async fn running_a_script_with_nothing_in_it_fails_the_request() {
    let database = TestDatabase::create().await;
    let mut service = Service::start(&database).await;
    let (_, events) = service.execute("-- nothing\n;", ExecuteTarget::All).await;
    match events.last() {
        Some(Event::RequestFailed { error, .. }) => assert_eq!(error.kind, ErrorKind::EmptyScript),
        other => panic!("expected RequestFailed, got {other:?}"),
    }
    service.stop().await;
}

#[tokio::test]
async fn shutdown_while_a_statement_runs_leaves_no_backend_behind() {
    let database = TestDatabase::create().await;
    let mut service = Service::start(&database).await;
    service
        .start_execute("SELECT pg_sleep(60)", ExecuteTarget::All)
        .await;
    loop {
        if let Event::ExecutionStarted { .. } = service.next_event().await {
            break;
        }
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(backends_in(&database).await >= 1, "the tab session is open");

    let started = std::time::Instant::now();
    service.stop().await;
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(backends_in(&database).await, 0);
}
