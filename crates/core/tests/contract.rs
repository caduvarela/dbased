#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The same scenario scripts against `FakeCore` and the real Core, compared by event shape
//! (docs/PROJECT.md 11.4, 11.5 step 5). The real Core runs with a scripted driver here and,
//! under the `integration` feature, with Postgres.

#[cfg(feature = "integration")]
mod support;

use std::sync::Arc;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use async_trait::async_trait;
use dbased_commands::fake::FakeCore;
use dbased_commands::fake::contract::{
    check_execution_order, check_one_terminal, collect_until_terminal, event_shape,
};
use dbased_commands::{
    ColumnMeta, Command, ConnectionId, ConnectionSettings, DatabaseKind, Event, ExecuteTarget,
    PasswordUpdate, RequestId, TabId, TlsMode, Value, bridge,
};
use dbased_core::{
    CancelHandle, Canceller, Column, ConnectionStore, Core, CoreDependencies, CoreError,
    DatabaseErrorInfo, Driver, DriverConnector, InMemorySecretStore, PageRequest, Routine, RowSink,
    RowSinkMessage, Schema, Session, SessionHealth, SessionId, StatementOutcome, StatementStatus,
    TableRef, UserSql, ViewRef,
};
use secrecy::SecretString;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "integration")]
use support::PASSWORD;
#[cfg(not(feature = "integration"))]
const PASSWORD: &str = "secret";

const TAB: TabId = TabId(1);
const EVENT_TIMEOUT: Duration = Duration::from_secs(60);

/// One statement of a scenario; each backend renders it in its own dialect.
#[derive(Clone, Copy)]
enum Step {
    Select(u64),
    Fail,
    Sleep,
}

#[derive(Clone, Copy, PartialEq)]
enum Flavor {
    Fake,
    Scripted,
    #[cfg(feature = "integration")]
    Postgres,
}

fn render(steps: &[Step], flavor: Flavor) -> String {
    let statements: Vec<String> = steps
        .iter()
        .map(|step| match (step, flavor) {
            (Step::Select(rows), Flavor::Fake | Flavor::Scripted) => format!("select {rows}"),
            (Step::Fail, Flavor::Fake | Flavor::Scripted) => "fail".to_owned(),
            (Step::Sleep, Flavor::Fake | Flavor::Scripted) => "sleep".to_owned(),
            #[cfg(feature = "integration")]
            (Step::Select(rows), Flavor::Postgres) => {
                format!("select value from generate_series(1, {rows}) as value")
            }
            #[cfg(feature = "integration")]
            (Step::Fail, Flavor::Postgres) => "select * from table_that_does_not_exist".to_owned(),
            #[cfg(feature = "integration")]
            (Step::Sleep, Flavor::Postgres) => "select pg_sleep(60)".to_owned(),
        })
        .collect();
    statements.join("; ")
}

// -- scripted driver: the statement text picks the behavior, like the fake's cues --

struct ScriptedConnector;

#[async_trait]
impl DriverConnector for ScriptedConnector {
    async fn connect(
        &self,
        _settings: &ConnectionSettings,
        _password: SecretString,
    ) -> Result<Arc<dyn Driver>, CoreError> {
        Ok(Arc::new(ScriptedDriver))
    }
}

struct ScriptedDriver;

#[async_trait]
impl Driver for ScriptedDriver {
    async fn open_session(&self) -> Result<Box<dyn Session>, CoreError> {
        Ok(Box::new(ScriptedSession))
    }
    async fn list_schemas(&self) -> Result<Vec<Schema>, CoreError> {
        Ok(Vec::new())
    }
    async fn list_tables(&self, _schema: &str) -> Result<Vec<TableRef>, CoreError> {
        Ok(Vec::new())
    }
    async fn list_columns(&self, _table: &TableRef) -> Result<Vec<Column>, CoreError> {
        Ok(Vec::new())
    }
    async fn list_views(&self, _schema: &str) -> Result<Vec<ViewRef>, CoreError> {
        Ok(Vec::new())
    }
    async fn list_routines(&self, _schema: &str) -> Result<Vec<Routine>, CoreError> {
        Ok(Vec::new())
    }
    async fn close(&self) {}
    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{identifier}\"")
    }
    fn paginate(&self, base_sql: &str, _page: PageRequest) -> String {
        base_sql.to_owned()
    }
}

struct NoCancel;

#[async_trait]
impl Canceller for NoCancel {
    async fn cancel_backend(&self, _backend: SessionId) -> Result<(), CoreError> {
        Ok(())
    }
}

struct ScriptedSession;

fn outcome(status: StatementStatus, rows: u64) -> StatementOutcome {
    StatementOutcome {
        status,
        rows_received: rows,
        rows_affected: None,
        partial: false,
        server_error: None,
        cancel_error: None,
        extra_statements_discarded: false,
        session: SessionHealth::Healthy,
    }
}

fn value_column() -> Vec<ColumnMeta> {
    vec![ColumnMeta {
        name: "n".into(),
        type_name: "int8".into(),
        nullable: Some(false),
    }]
}

#[async_trait]
impl Session for ScriptedSession {
    fn session_id(&self) -> SessionId {
        SessionId(1)
    }

    fn cancel_handle(&self) -> CancelHandle {
        CancelHandle::new(SessionId(1), Arc::new(NoCancel))
    }

    async fn execute(
        &mut self,
        sql: UserSql,
        sink: RowSink,
        token: CancellationToken,
    ) -> Result<StatementOutcome, CoreError> {
        let text = sql.as_str().to_owned();
        let mut words = text.split_whitespace();
        let result = match words.next().unwrap_or("") {
            "select" => {
                let rows: u64 = words.next().and_then(|word| word.parse().ok()).unwrap_or(1);
                sink.send(RowSinkMessage::Columns(value_column()))
                    .await
                    .unwrap();
                for number in 0..rows {
                    let row = vec![Value::Int(i64::try_from(number).unwrap())];
                    sink.send(RowSinkMessage::Row(row)).await.unwrap();
                }
                outcome(StatementStatus::Completed, rows)
            }
            "fail" => {
                sink.send(RowSinkMessage::Columns(Vec::new()))
                    .await
                    .unwrap();
                let mut failed = outcome(StatementStatus::Failed, 0);
                failed.server_error = Some(DatabaseErrorInfo {
                    sqlstate: "42601".into(),
                    message: "syntax error".into(),
                    detail: None,
                    hint: None,
                    position: Some(1),
                });
                failed
            }
            _ => {
                sink.send(RowSinkMessage::Columns(Vec::new()))
                    .await
                    .unwrap();
                token.cancelled().await;
                let mut cancelled = outcome(StatementStatus::Cancelled, 0);
                cancelled.partial = true;
                cancelled
            }
        };
        sink.send(RowSinkMessage::Outcome(result.clone()))
            .await
            .unwrap();
        Ok(result)
    }

    async fn health_check(&mut self) -> Result<SessionHealth, CoreError> {
        Ok(SessionHealth::Healthy)
    }

    async fn close(self: Box<Self>) {}
}

// -- the rig: one Core behind a bridge, whichever kind it is --

struct Rig {
    flavor: Flavor,
    commands: Sender<Command>,
    events: Receiver<Event>,
    connection: ConnectionId,
    next_request: u64,
    /// Whether `ResultStarted` of the cancelled statement arrived before the cancel was sent.
    started_before_cancel: bool,
    task: JoinHandle<()>,
    _directory: Option<tempfile::TempDir>,
}

impl Rig {
    async fn fake() -> Self {
        let (ui, link) = bridge(256, 1024);
        let task = tokio::spawn(FakeCore::new().serve(link));
        let mut rig = Self::assemble(Flavor::Fake, ui, task, None);
        rig.expect_connections_changed().await;
        rig.connection = ConnectionId(1);
        rig.open_tab().await;
        rig
    }

    async fn real(
        flavor: Flavor,
        connector: Arc<dyn DriverConnector>,
        settings: ConnectionSettings,
    ) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let (ui, link) = bridge(256, 1024);
        let core = Core::new(CoreDependencies {
            connection_store: ConnectionStore::new(directory.path().join("config.toml")),
            secret_store: Arc::new(InMemorySecretStore::default()),
            connector,
        });
        let task = tokio::spawn(core.run(link));
        let mut rig = Self::assemble(flavor, ui, task, Some(directory));
        rig.expect_connections_changed().await;
        let request = rig.request_id();
        rig.send(Command::SaveConnection {
            request,
            id: None,
            settings,
            // @NOTE: saved so that running connects on its own (11.2).
            password: PasswordUpdate::Save(SecretString::from(PASSWORD)),
        })
        .await;
        let saved = rig.until_terminal(request).await;
        let Some(Event::ConnectionSaved { id, .. }) = saved.last() else {
            panic!("the connection was not saved: {saved:?}");
        };
        rig.connection = *id;
        rig.open_tab().await;
        rig
    }

    async fn scripted() -> Self {
        let settings = ConnectionSettings {
            name: "scripted".into(),
            kind: DatabaseKind::Postgres,
            host: "127.0.0.1".into(),
            port: 5432,
            user: "u".into(),
            database: "d".into(),
            tls: TlsMode::Disable,
        };
        Self::real(Flavor::Scripted, Arc::new(ScriptedConnector), settings).await
    }

    fn assemble(
        flavor: Flavor,
        ui: dbased_commands::UiLink,
        task: JoinHandle<()>,
        directory: Option<tempfile::TempDir>,
    ) -> Self {
        Self {
            flavor,
            commands: ui.commands,
            events: ui.events,
            connection: ConnectionId(0),
            next_request: 100,
            started_before_cancel: false,
            task,
            _directory: directory,
        }
    }

    fn request_id(&mut self) -> RequestId {
        self.next_request += 1;
        RequestId(self.next_request)
    }

    async fn send(&self, command: Command) {
        self.commands.send(command).await.unwrap();
    }

    async fn next_event(&self) -> Event {
        tokio::time::timeout(EVENT_TIMEOUT, self.events.recv())
            .await
            .expect("an event arrives in time")
            .expect("the event channel is open")
    }

    async fn expect_connections_changed(&self) {
        let first = self.next_event().await;
        assert!(
            matches!(first, Event::ConnectionsChanged { .. }),
            "{first:?}"
        );
    }

    async fn until_terminal(&self, request: RequestId) -> Vec<Event> {
        tokio::time::timeout(EVENT_TIMEOUT, collect_until_terminal(&self.events, request))
            .await
            .expect("the terminal event arrives in time")
    }

    async fn open_tab(&self) {
        self.send(Command::OpenTab {
            tab: TAB,
            connection: self.connection,
        })
        .await;
    }

    async fn start(&mut self, steps: &[Step], target: ExecuteTarget) -> RequestId {
        let request = self.request_id();
        self.send(Command::Execute {
            request,
            tab: TAB,
            script: render(steps, self.flavor),
            target,
        })
        .await;
        request
    }

    /// Runs the script to its terminal event.
    async fn run(&mut self, steps: &[Step]) -> Vec<Event> {
        let request = self.start(steps, ExecuteTarget::All).await;
        let events = self.until_terminal(request).await;
        check_one_terminal(&events, request).unwrap();
        check_execution_order(&events).unwrap();
        events
    }

    /// Starts the script, lets it get going, cancels it and returns every event up to the end.
    async fn run_and_cancel(&mut self, steps: &[Step]) -> Vec<Event> {
        let request = self.start(steps, ExecuteTarget::All).await;
        let mut seen = Vec::new();
        loop {
            let event = self.next_event().await;
            let started = matches!(event, Event::ExecutionStarted { .. });
            seen.push(event);
            if started {
                break;
            }
        }
        // @NOTE: the real session needs a moment to be running before a cancel can reach it.
        tokio::time::sleep(Duration::from_millis(300)).await;
        while let Ok(event) = self.events.try_recv() {
            seen.push(event);
        }
        self.started_before_cancel = seen
            .iter()
            .any(|event| matches!(event, Event::ResultStarted { .. }));
        self.send(Command::Cancel { tab: TAB, request }).await;
        seen.extend(self.until_terminal(request).await);
        check_one_terminal(&seen, request).unwrap();
        check_execution_order(&seen).unwrap();
        seen
    }

    async fn shutdown(self) {
        drop(self.commands);
        let finished = tokio::time::timeout(Duration::from_secs(20), self.task).await;
        assert!(finished.is_ok(), "the Core stops once the channel closes");
    }
}

/// Shapes without the progress events: those are coalesced or dropped by design (11.1).
fn shapes(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter(|event| !matches!(event, Event::ResultProgress { .. }))
        .map(event_shape)
        .collect()
}

// -- scenarios --

#[derive(Clone, Copy, Debug)]
enum Scenario {
    SelectRows,
    SelectNothing,
    Fail,
    SleepAndCancel,
    MultiStatement,
    ConnectOnExecute,
}

const SCENARIOS: [Scenario; 6] = [
    Scenario::SelectRows,
    Scenario::SelectNothing,
    Scenario::Fail,
    Scenario::SleepAndCancel,
    Scenario::MultiStatement,
    Scenario::ConnectOnExecute,
];

async fn play(rig: &mut Rig, scenario: Scenario) -> Vec<String> {
    match scenario {
        Scenario::SelectRows => shapes(&rig.run(&[Step::Select(3)]).await),
        Scenario::SelectNothing => shapes(&rig.run(&[Step::Select(0)]).await),
        Scenario::Fail => {
            let steps = [Step::Select(1), Step::Fail, Step::Select(2)];
            shapes(&rig.run(&steps).await)
        }
        Scenario::SleepAndCancel => {
            let events = rig.run_and_cancel(&[Step::Sleep]).await;
            shapes(&events)
        }
        Scenario::MultiStatement => {
            let steps = [Step::Select(1), Step::Select(2), Step::Select(3)];
            shapes(&rig.run(&steps).await)
        }
        Scenario::ConnectOnExecute => {
            // @NOTE: the tab is on a disconnected connection, so the first run connects it
            // and the second must not repeat the connect events.
            let mut first = shapes(&rig.run(&[Step::Select(1)]).await);
            first.push("--".to_owned());
            first.extend(shapes(&rig.run(&[Step::Select(1)]).await));
            first
        }
    }
}

async fn fetch_window(rig: &mut Rig) -> (u64, usize) {
    let events = rig.run(&[Step::Select(5)]).await;
    let result = events
        .iter()
        .find_map(|event| match event {
            Event::ResultFinished { result, .. } => Some(*result),
            _ => None,
        })
        .expect("a finished result");
    rig.send(Command::FetchRows {
        result,
        start: 1,
        count: 3,
    })
    .await;
    loop {
        if let Event::RowsWindow { start, rows, .. } = rig.next_event().await {
            return (start, rows.len());
        }
    }
}

/// Differences the doc allows between the fake and a real driver, applied to the real side.
/// None for the scripted driver: it follows the `Session` contract exactly.
fn tolerate(flavor: Flavor, shapes: Vec<String>) -> Vec<String> {
    match flavor {
        Flavor::Fake | Flavor::Scripted => shapes,
        #[cfg(feature = "integration")]
        Flavor::Postgres => shapes,
    }
}

async fn compare(real_flavor: Flavor, mut real: Rig, mut fake: Rig) {
    let mut failures = Vec::new();
    for scenario in SCENARIOS {
        let fake_shapes = play(&mut fake, scenario).await;
        let real_shapes = tolerate(real_flavor, play(&mut real, scenario).await);
        if fake_shapes != real_shapes {
            failures.push(format!(
                "{scenario:?}\n  fake: {fake_shapes:#?}\n  real: {real_shapes:#?}"
            ));
        }
    }
    let fake_window = fetch_window(&mut fake).await;
    let real_window = fetch_window(&mut real).await;
    if fake_window != real_window {
        failures.push(format!(
            "FetchRows window\n  fake: {fake_window:?}\n  real: {real_window:?}"
        ));
    }
    // @NOTE: the Postgres driver sends Columns with the first row, so a slow statement has no
    // ResultStarted until it produces a row or ends; the fake announces it at once. Known drift.
    let allowed_late_start = real_flavor != Flavor::Scripted;
    if fake.started_before_cancel != real.started_before_cancel
        && !(allowed_late_start && fake.started_before_cancel && !real.started_before_cancel)
    {
        failures.push(format!(
            "ResultStarted before cancel\n  fake: {}\n  real: {}",
            fake.started_before_cancel, real.started_before_cancel
        ));
    }
    fake.shutdown().await;
    real.shutdown().await;
    assert!(failures.is_empty(), "drift:\n{}", failures.join("\n"));
}

#[tokio::test]
async fn the_scripted_core_matches_the_fake() {
    compare(Flavor::Scripted, Rig::scripted().await, Rig::fake().await).await;
}

#[cfg(feature = "integration")]
#[tokio::test]
async fn the_postgres_core_matches_the_fake() {
    let database = support::TestDatabase::create().await;
    let settings = ConnectionSettings {
        name: "contract".into(),
        kind: DatabaseKind::Postgres,
        host: "127.0.0.1".into(),
        port: database.server.port,
        user: "postgres".into(),
        database: database.name.clone(),
        tls: TlsMode::Disable,
    };
    let real = Rig::real(
        Flavor::Postgres,
        Arc::new(dbased_core::DefaultConnector),
        settings,
    )
    .await;
    compare(Flavor::Postgres, real, Rig::fake().await).await;
}
