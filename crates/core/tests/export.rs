#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! CSV export against a real Postgres (docs/PROJECT.md 3.3).

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dbased_commands::{
    Command, ConnectionId, ConnectionSettings, CsvOptions, DatabaseKind, ErrorKind, Event,
    ExportStatus, PasswordSource, PasswordUpdate, RequestId, TlsMode, UiLink, bridge,
};
use dbased_core::drivers::postgres::{PostgresConfig, PostgresDriver, TlsMode as PostgresTls};
use dbased_core::{
    ConnectionStore, Core, CoreDependencies, CoreError, Driver, DriverConnector, DriverFactory,
    InMemorySecretStore, SessionLimits,
};
use secrecy::SecretString;
use support::{PASSWORD, TestDatabase};
use tokio::task::JoinHandle;

const ROW_COUNT: u64 = 200_000;

/// Builds the Postgres driver with small result caps, to prove exports ignore them.
struct CappedConnector;

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
        config.limits = SessionLimits {
            max_rows: 1_000,
            max_bytes: 64 * 1024,
        };
        let driver = PostgresDriver::connect(config).await?;
        Ok(Arc::from(driver))
    }
}

struct Service {
    link: UiLink,
    core: JoinHandle<()>,
    connection: ConnectionId,
    next_request: u64,
    directory: tempfile::TempDir,
    _config: tempfile::TempDir,
}

impl Service {
    async fn start(database: &TestDatabase) -> Self {
        let config = tempfile::tempdir().unwrap();
        let (link, core_link) = bridge(256, 1024);
        let core = Core::new(CoreDependencies {
            connection_store: ConnectionStore::new(config.path().join("config.toml")),
            secret_store: Arc::new(InMemorySecretStore::default()),
            connector: Arc::new(CappedConnector),
        });
        let mut service = Self {
            link,
            core: tokio::spawn(core.run(core_link)),
            connection: ConnectionId(0),
            next_request: 100,
            directory: tempfile::tempdir().unwrap(),
            _config: config,
        };
        assert!(matches!(
            service.next_event().await,
            Event::ConnectionsChanged { .. }
        ));
        let settings = ConnectionSettings {
            name: "export".into(),
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
                password: PasswordUpdate::Save(SecretString::from(PASSWORD)),
            })
            .await;
        let Event::ConnectionSaved { id, .. } =
            service.until_terminal(request).await.pop().unwrap()
        else {
            panic!("the connection was not saved");
        };
        service.connection = id;
        let request = service.request_id();
        service
            .send(Command::Connect {
                request,
                id,
                password: PasswordSource::Saved,
            })
            .await;
        let terminal = service.until_terminal(request).await.pop().unwrap();
        assert_eq!(terminal, Event::RequestCompleted { request });
        service
    }

    fn request_id(&mut self) -> RequestId {
        self.next_request += 1;
        RequestId(self.next_request)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    fn directory_listing(&self) -> Vec<String> {
        std::fs::read_dir(self.directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    async fn send(&self, command: Command) {
        self.link.commands.send(command).await.unwrap();
    }

    async fn next_event(&self) -> Event {
        tokio::time::timeout(Duration::from_secs(120), self.link.events.recv())
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
                | Event::ExportFinished { request: found, .. } => *found == request,
                _ => false,
            };
            seen.push(event);
            if done {
                return seen;
            }
        }
    }

    async fn start_export(&mut self, sql: &str, path: &Path, options: CsvOptions) -> RequestId {
        let request = self.request_id();
        self.send(Command::ExportCsv {
            request,
            connection: self.connection,
            sql: sql.to_owned(),
            path: path.to_owned(),
            options,
        })
        .await;
        request
    }

    async fn stop(self) {
        drop(self.link.commands);
        tokio::time::timeout(Duration::from_secs(30), self.core)
            .await
            .expect("the core stops")
            .unwrap();
    }
}

/// One CSV field: `None` is an unquoted empty field (NULL), `Some` a present value.
type Field = Option<String>;

/// A strict RFC 4180 reader, written independently from the encoder it checks.
fn parse_csv(text: &str, delimiter: char) -> Vec<Vec<Field>> {
    let mut rows = Vec::new();
    let mut row: Vec<Field> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if in_quotes {
            if character == '"' {
                if characters.peek() == Some(&'"') {
                    characters.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(character);
            }
            continue;
        }
        match character {
            '"' => {
                assert!(
                    field.is_empty() && !quoted,
                    "a quote inside an unquoted field"
                );
                in_quotes = true;
                quoted = true;
            }
            '\n' => {
                row.push(finish_field(&mut field, &mut quoted));
                rows.push(std::mem::take(&mut row));
            }
            other if other == delimiter => row.push(finish_field(&mut field, &mut quoted)),
            other => field.push(other),
        }
    }
    assert!(
        !in_quotes && field.is_empty() && row.is_empty(),
        "file ends mid-record"
    );
    rows
}

fn finish_field(field: &mut String, quoted: &mut bool) -> Field {
    let text = std::mem::take(field);
    let was_quoted = std::mem::take(quoted);
    (was_quoted || !text.is_empty()).then_some(text)
}

const WIDE_QUERY: &str = "\
SELECT g AS id,
       'name ' || g AS label,
       CASE WHEN g % 10 = 0 THEN NULL ELSE 'v' || g END AS maybe_null,
       (g::numeric / 7)::numeric(30,10) AS ratio,
       DATE '2026-02-01' + (g % 28)::int AS day,
       TIMESTAMP '2026-01-01 00:00:00' + g * interval '1 second' AS moment,
       jsonb_build_object('n', g, 'tag', 'a,b') AS document,
       decode(lpad(to_hex(g % 256), 2, '0'), 'hex') AS blob,
       CASE WHEN g % 5 = 0 THEN E'multi\\nline, \"q\"' WHEN g % 5 = 1 THEN '' ELSE 'plain' END AS note
FROM generate_series(1, 200000) AS g";

fn expected_row(g: u64) -> Vec<Field> {
    let rounded = (2 * g * 10_000_000_000 + 7) / 14;
    let ratio = format!(
        "{}.{:010}",
        rounded / 10_000_000_000,
        rounded % 10_000_000_000
    );
    let seconds = g;
    let moment = format!(
        "2026-01-{:02} {:02}:{:02}:{:02}",
        1 + seconds / 86_400,
        seconds % 86_400 / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    );
    let note = match g % 5 {
        0 => "multi\nline, \"q\"",
        1 => "",
        _ => "plain",
    };
    let text = |value: String| Some(value);
    vec![
        text(g.to_string()),
        text(format!("name {g}")),
        (!g.is_multiple_of(10)).then(|| format!("v{g}")),
        text(ratio),
        text(format!("2026-02-{:02}", 1 + g % 28)),
        text(moment),
        text(format!("{{\"n\": {g}, \"tag\": \"a,b\"}}")),
        text(format!("\\x{:02x}", g % 256)),
        text(note.to_owned()),
    ]
}

#[tokio::test]
async fn exports_two_hundred_thousand_rows_exactly() {
    let database = TestDatabase::create().await;
    let mut service = Service::start(&database).await;
    let path = service.path("wide.csv");

    let request = service
        .start_export(WIDE_QUERY, &path, CsvOptions::default())
        .await;
    let events = service.until_terminal(request).await;

    let Some(Event::ExportFinished {
        status: ExportStatus::Completed,
        rows_written,
        bytes_written,
        ..
    }) = events.last()
    else {
        panic!("expected a completed export, got {:?}", events.last());
    };
    assert_eq!(*rows_written, ROW_COUNT);
    let content = std::fs::read(&path).unwrap();
    assert_eq!(*bytes_written, content.len() as u64);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::ExportProgress { .. }))
    );
    assert_eq!(service.directory_listing(), ["wide.csv"]);

    let text = String::from_utf8(content).unwrap();
    let rows = parse_csv(&text, ',');
    assert_eq!(rows.len() as u64, ROW_COUNT + 1);
    let header: Vec<Field> = [
        "id",
        "label",
        "maybe_null",
        "ratio",
        "day",
        "moment",
        "document",
        "blob",
        "note",
    ]
    .map(|name| Some(name.to_owned()))
    .to_vec();
    assert_eq!(rows[0], header);
    for (index, row) in rows[1..].iter().enumerate() {
        let g = index as u64 + 1;
        assert_eq!(row, &expected_row(g), "row {g}");
    }
    service.stop().await;
}

#[tokio::test]
async fn a_semicolon_export_without_header_keeps_values_intact() {
    let database = TestDatabase::create().await;
    let mut service = Service::start(&database).await;
    let path = service.path("semi.csv");
    let options = CsvOptions {
        delimiter: ';',
        header: false,
        bom: true,
    };

    let request = service
        .start_export(
            "SELECT 1 AS a, 'x;y' AS b, NULL::text AS c, '' AS d, 'caf\u{e9} \u{1F600}' AS e",
            &path,
            options,
        )
        .await;
    service.until_terminal(request).await;

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        "\u{feff}1;\"x;y\";;\"\";caf\u{e9} \u{1F600}\n"
    );
    service.stop().await;
}

#[tokio::test]
async fn cancel_mid_export_removes_the_partial_file_and_frees_the_server() {
    let database = TestDatabase::create().await;
    let mut service = Service::start(&database).await;
    let path = service.path("big.csv");

    let request = service
        .start_export(
            "SELECT g, repeat('x', 200) AS padding FROM generate_series(1, 100000000) AS g",
            &path,
            CsvOptions::default(),
        )
        .await;
    loop {
        if matches!(service.next_event().await, Event::ExportProgress { .. }) {
            break;
        }
    }
    service.send(Command::CancelExport { request }).await;
    let events = service.until_terminal(request).await;

    assert!(matches!(
        events.last(),
        Some(Event::ExportFinished {
            status: ExportStatus::Cancelled,
            ..
        })
    ));
    assert!(service.directory_listing().is_empty());

    // @NOTE: the cancelled backend is gone, and the next export works.
    let mut admin = database.admin().await;
    let running: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE datname = $1 AND query LIKE '%100000000%' \
         AND pid <> pg_backend_pid() AND state = 'active'",
    )
    .bind(&database.name)
    .fetch_one(&mut admin)
    .await
    .unwrap();
    assert_eq!(running, 0);
    let again = service.path("after.csv");
    let request = service
        .start_export("SELECT 1 AS one", &again, CsvOptions::default())
        .await;
    let events = service.until_terminal(request).await;
    assert!(matches!(
        events.last(),
        Some(Event::ExportFinished {
            status: ExportStatus::Completed,
            ..
        })
    ));
    assert_eq!(std::fs::read_to_string(&again).unwrap(), "one\n1\n");
    service.stop().await;
}

#[tokio::test]
async fn a_server_error_mid_export_removes_the_partial_file() {
    let database = TestDatabase::create().await;
    let mut service = Service::start(&database).await;
    let path = service.path("broken.csv");

    let request = service
        .start_export(
            "SELECT 1 / (g - 5000) AS ratio FROM generate_series(1, 20000) AS g",
            &path,
            CsvOptions::default(),
        )
        .await;
    let events = service.until_terminal(request).await;

    let Some(Event::RequestFailed { error, .. }) = events.last() else {
        panic!("expected RequestFailed, got {:?}", events.last());
    };
    assert_eq!(error.kind, ErrorKind::Database);
    assert_eq!(error.database.as_ref().unwrap().sqlstate, "22012");
    assert!(service.directory_listing().is_empty());
    service.stop().await;
}
