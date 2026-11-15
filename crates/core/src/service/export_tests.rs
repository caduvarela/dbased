#![allow(clippy::unwrap_used, clippy::expect_used)]

//! CSV export through the Core service with a scripted session (docs/PROJECT.md 3.3).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use dbased_commands::{
    Command, ConnectionId, CsvOptions, ErrorKind, Event, ExportStatus, PasswordSource, RequestId,
};
use tempfile::TempDir;

use super::tests::{Harness, failed_kind};
use super::tests_session::SessionLog;

struct Fixture {
    harness: Harness,
    log: Arc<SessionLog>,
    connection: ConnectionId,
    directory: TempDir,
}

impl Fixture {
    async fn connected() -> Self {
        let fixture = Self::disconnected().await;
        fixture
            .harness
            .connect(2, fixture.connection, PasswordSource::None)
            .await;
        fixture
    }

    async fn disconnected() -> Self {
        let harness = Harness::new().await;
        let log = harness.connector.sessions.clone();
        let connection = harness.save(1, None, "db").await;
        Self {
            harness,
            log,
            connection,
            directory: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    async fn start(&self, request: u64, sql: &str, path: &Path, options: CsvOptions) {
        self.harness
            .send(Command::ExportCsv {
                request: RequestId(request),
                connection: self.connection,
                sql: sql.to_owned(),
                path: path.to_owned(),
                options,
            })
            .await;
    }

    async fn export(&self, request: u64, sql: &str, path: &Path) -> Vec<Event> {
        self.start(request, sql, path, CsvOptions::default()).await;
        self.harness.until_terminal(RequestId(request)).await
    }

    /// Names of everything in the export directory, partial files included.
    fn directory_listing(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

fn read(path: &Path) -> String {
    String::from_utf8(std::fs::read(path).unwrap()).unwrap()
}

fn terminal_count(events: &[Event], request: u64) -> usize {
    events
        .iter()
        .filter(|event| match event {
            Event::ExportFinished { request: found, .. }
            | Event::RequestFailed { request: found, .. } => *found == RequestId(request),
            _ => false,
        })
        .count()
}

#[tokio::test]
async fn a_completed_export_writes_the_file_atomically() {
    let fixture = Fixture::connected().await;
    let path = fixture.path("out.csv");

    let events = fixture.export(10, "rows 3;", &path).await;

    assert_eq!(
        events.last(),
        Some(&Event::ExportFinished {
            request: RequestId(10),
            status: ExportStatus::Completed,
            rows_written: 3,
            bytes_written: 8,
        })
    );
    assert_eq!(read(&path), "n\n0\n1\n2\n");
    assert_eq!(fixture.directory_listing(), ["out.csv"]);
    // @NOTE: a dedicated, uncapped session that is closed afterwards (3.2.4).
    assert_eq!(fixture.log.exports.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.log.closes.load(Ordering::SeqCst), 1);
    assert_eq!(terminal_count(&events, 10), 1);
}

#[tokio::test]
async fn the_statement_text_reaches_the_driver_without_its_terminator() {
    let fixture = Fixture::connected().await;

    fixture
        .export(10, "  rows 1 ;  ", &fixture.path("a.csv"))
        .await;

    let executed = fixture.log.executed.lock().unwrap().clone();
    assert_eq!(executed, ["rows 1"]);
}

#[tokio::test]
async fn options_control_delimiter_header_and_bom() {
    let fixture = Fixture::connected().await;
    let path = fixture.path("mixed.csv");
    let options = CsvOptions {
        delimiter: ';',
        header: true,
        bom: true,
    };

    fixture.start(10, "mixed", &path, options).await;
    fixture.harness.until_terminal(RequestId(10)).await;

    let expected = "\u{feff}id;note\n1;a,b\n2;\n3;\"line\nbreak \"\"q\"\"\"\n4;\"\"\n";
    assert_eq!(read(&path), expected);

    let plain = fixture.path("plain.csv");
    let options = CsvOptions {
        header: false,
        ..CsvOptions::default()
    };
    fixture.start(11, "mixed", &plain, options).await;
    fixture.harness.until_terminal(RequestId(11)).await;
    assert_eq!(
        read(&plain),
        "1,\"a,b\"\n2,\n3,\"line\nbreak \"\"q\"\"\"\n4,\"\"\n"
    );
}

#[tokio::test]
async fn progress_events_arrive_before_the_single_terminal_event() {
    let fixture = Fixture::connected().await;

    let events = fixture.export(10, "paced 12", &fixture.path("p.csv")).await;

    let progress: Vec<u64> = events
        .iter()
        .filter_map(|event| match event {
            Event::ExportProgress { rows_written, .. } => Some(*rows_written),
            _ => None,
        })
        .collect();
    assert!(progress.len() >= 2, "progress events: {progress:?}");
    assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(matches!(
        events.last(),
        Some(Event::ExportFinished {
            rows_written: 12,
            ..
        })
    ));
    assert_eq!(terminal_count(&events, 10), 1);
}

#[tokio::test]
async fn cancel_ends_the_export_and_removes_the_partial_file() {
    let fixture = Fixture::connected().await;
    let path = fixture.path("cancelled.csv");
    fixture
        .start(10, "paced 1000", &path, CsvOptions::default())
        .await;

    // @NOTE: wait for the first progress, so the partial file exists while we cancel.
    loop {
        if matches!(
            fixture.harness.next_event().await,
            Event::ExportProgress { .. }
        ) {
            break;
        }
    }
    assert!(!fixture.directory_listing().is_empty());
    fixture
        .harness
        .send(Command::CancelExport {
            request: RequestId(10),
        })
        .await;
    let events = fixture.harness.until_terminal(RequestId(10)).await;

    assert!(matches!(
        events.last(),
        Some(Event::ExportFinished {
            status: ExportStatus::Cancelled,
            ..
        })
    ));
    assert!(fixture.directory_listing().is_empty());
    assert_eq!(fixture.log.closes.load(Ordering::SeqCst), 1);
    assert_eq!(terminal_count(&events, 10), 1);
}

#[tokio::test]
async fn cancelling_an_unknown_export_is_a_no_op() {
    let fixture = Fixture::connected().await;
    fixture
        .harness
        .send(Command::CancelExport {
            request: RequestId(404),
        })
        .await;

    let events = fixture.export(10, "rows 1", &fixture.path("ok.csv")).await;

    assert!(matches!(events.last(), Some(Event::ExportFinished { .. })));
}

#[tokio::test]
async fn a_failing_statement_removes_the_partial_file_and_keeps_the_old_one() {
    let fixture = Fixture::connected().await;
    let path = fixture.path("keep.csv");
    std::fs::write(&path, "previous").unwrap();

    let events = fixture.export(10, "failafter 5", &path).await;

    let Some(Event::RequestFailed { error, .. }) = events.last() else {
        panic!("expected RequestFailed, got {events:?}");
    };
    assert_eq!(error.kind, ErrorKind::Database);
    assert_eq!(error.database.as_ref().unwrap().sqlstate, "22012");
    assert_eq!(read(&path), "previous");
    assert_eq!(fixture.directory_listing(), ["keep.csv"]);
    assert_eq!(terminal_count(&events, 10), 1);
}

#[tokio::test]
async fn a_session_failure_removes_the_partial_file() {
    let fixture = Fixture::connected().await;

    let events = fixture.export(10, "deadrows", &fixture.path("d.csv")).await;

    assert_eq!(
        failed_kind(events.last().unwrap()),
        ErrorKind::SessionClosed
    );
    assert!(fixture.directory_listing().is_empty());
    assert_eq!(fixture.log.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_statement_without_rows_is_rejected() {
    let fixture = Fixture::connected().await;

    let events = fixture.export(10, "affect 4", &fixture.path("n.csv")).await;

    assert_eq!(failed_kind(events.last().unwrap()), ErrorKind::Unsupported);
    assert!(fixture.directory_listing().is_empty());
}

#[tokio::test]
async fn only_one_statement_is_exported() {
    let fixture = Fixture::connected().await;

    let events = fixture
        .export(10, "rows 1; rows 2", &fixture.path("m.csv"))
        .await;
    assert_eq!(failed_kind(events.last().unwrap()), ErrorKind::Unsupported);

    let events = fixture.export(11, "  ;  ", &fixture.path("m.csv")).await;
    assert_eq!(failed_kind(events.last().unwrap()), ErrorKind::EmptyScript);

    assert_eq!(fixture.log.opens.load(Ordering::SeqCst), 0);
    assert!(fixture.directory_listing().is_empty());
}

#[tokio::test]
async fn bad_paths_and_delimiters_fail_before_any_query() {
    let fixture = Fixture::connected().await;

    let missing_directory = fixture.path("missing").join("out.csv");
    let events = fixture.export(10, "rows 1", &missing_directory).await;
    assert_eq!(failed_kind(events.last().unwrap()), ErrorKind::Storage);

    fixture
        .start(
            11,
            "rows 1",
            &fixture.path("q.csv"),
            CsvOptions {
                delimiter: '"',
                ..CsvOptions::default()
            },
        )
        .await;
    let events = fixture.harness.until_terminal(RequestId(11)).await;
    assert_eq!(
        failed_kind(events.last().unwrap()),
        ErrorKind::Configuration
    );

    assert_eq!(fixture.log.opens.load(Ordering::SeqCst), 0);
    assert!(fixture.directory_listing().is_empty());
}

#[tokio::test]
async fn export_needs_a_known_open_connection() {
    let fixture = Fixture::disconnected().await;

    let events = fixture.export(10, "rows 1", &fixture.path("c.csv")).await;
    assert_eq!(failed_kind(events.last().unwrap()), ErrorKind::Connection);

    fixture
        .harness
        .send(Command::ExportCsv {
            request: RequestId(11),
            connection: ConnectionId(99),
            sql: "rows 1".into(),
            path: fixture.path("c.csv"),
            options: CsvOptions::default(),
        })
        .await;
    let events = fixture.harness.until_terminal(RequestId(11)).await;
    assert_eq!(
        failed_kind(events.last().unwrap()),
        ErrorKind::UnknownConnection
    );
}

#[tokio::test]
async fn disconnect_cancels_a_running_export_and_waits_for_it() {
    let fixture = Fixture::connected().await;
    fixture
        .start(
            10,
            "paced 1000",
            &fixture.path("x.csv"),
            CsvOptions::default(),
        )
        .await;
    loop {
        if matches!(
            fixture.harness.next_event().await,
            Event::ExportProgress { .. }
        ) {
            break;
        }
    }

    fixture
        .harness
        .send(Command::Disconnect {
            request: RequestId(11),
            id: fixture.connection,
        })
        .await;
    let mut seen = Vec::new();
    while terminal_count(&seen, 10) == 0
        || !seen.contains(&Event::RequestCompleted {
            request: RequestId(11),
        })
    {
        seen.push(fixture.harness.next_event().await);
    }

    assert_eq!(terminal_count(&seen, 10), 1);
    assert!(seen.iter().any(|event| matches!(
        event,
        Event::ExportFinished {
            status: ExportStatus::Cancelled,
            ..
        }
    )));
    assert!(fixture.directory_listing().is_empty());
    // @NOTE: the session closed before the pool did.
    let order = fixture.log.order.lock().unwrap().clone();
    let session_close = order.iter().position(|step| *step == "session_close");
    let driver_close = order.iter().position(|step| *step == "driver_close");
    assert!(session_close < driver_close, "order: {order:?}");
}

#[tokio::test]
async fn shutdown_cancels_a_running_export() {
    let fixture = Fixture::connected().await;
    let directory = fixture.directory.path().to_owned();
    fixture
        .start(
            10,
            "paced 1000",
            &fixture.path("s.csv"),
            CsvOptions::default(),
        )
        .await;
    loop {
        if matches!(
            fixture.harness.next_event().await,
            Event::ExportProgress { .. }
        ) {
            break;
        }
    }

    let log = fixture.log.clone();
    let rest = tokio::time::timeout(Duration::from_secs(10), fixture.harness.shut_down())
        .await
        .unwrap();

    assert_eq!(terminal_count(&rest, 10), 1);
    assert_eq!(std::fs::read_dir(directory).unwrap().count(), 0);
    assert_eq!(log.closes.load(Ordering::SeqCst), 1);
}
