#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Execution through the Core service with a scripted session (docs/PROJECT.md 11.5 step 4).

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use dbased_commands::{
    Command, ConnectionId, ErrorKind, Event, ExecuteTarget, ExecutionStatus, PasswordSource,
    PasswordUpdate, RequestId, ResultId, ResultStatus, TabId, Value,
};
use secrecy::SecretString;

use super::InMemorySecretStore;
use super::tests::{Harness, ScriptedConnector, failed_kind, settings};
use super::tests_session::SessionLog;

const TAB: TabId = TabId(1);

struct Fixture {
    harness: Harness,
    log: Arc<SessionLog>,
    connection: ConnectionId,
}

impl Fixture {
    /// A saved connection with a saved password, not connected yet.
    async fn disconnected() -> Self {
        Self::with_connector(ScriptedConnector::default()).await
    }

    async fn with_connector(connector: ScriptedConnector) -> Self {
        let log = connector.sessions.clone();
        let harness = Harness::with_connector(connector).await;
        Self::finish(harness, log).await
    }

    async fn finish(harness: Harness, log: Arc<SessionLog>) -> Self {
        let events = harness
            .request(
                Command::SaveConnection {
                    request: RequestId(900),
                    id: None,
                    settings: settings("db"),
                    password: PasswordUpdate::Save(SecretString::from("secret")),
                },
                RequestId(900),
            )
            .await;
        let Some(Event::ConnectionSaved { id, .. }) = events.last() else {
            panic!("expected ConnectionSaved, got {events:?}");
        };
        Self {
            connection: *id,
            harness,
            log,
        }
    }

    async fn connected() -> Self {
        let fixture = Self::disconnected().await;
        fixture
            .harness
            .connect(901, fixture.connection, PasswordSource::None)
            .await;
        fixture.open_tab(TAB).await;
        fixture
    }

    async fn open_tab(&self, tab: TabId) {
        self.harness
            .send(Command::OpenTab {
                tab,
                connection: self.connection,
            })
            .await;
    }

    async fn send_execute(&self, request: u64, tab: TabId, script: &str, target: ExecuteTarget) {
        self.harness
            .send(Command::Execute {
                request: RequestId(request),
                tab,
                script: script.to_owned(),
                target,
            })
            .await;
    }

    async fn run(&self, request: u64, script: &str, target: ExecuteTarget) -> Vec<Event> {
        self.send_execute(request, TAB, script, target).await;
        self.harness.until_terminal(RequestId(request)).await
    }

    async fn run_all(&self, request: u64, script: &str) -> Vec<Event> {
        self.run(request, script, ExecuteTarget::All).await
    }

    async fn fetch(&self, request: u64, statement: u32, start: u64, count: u32) -> Vec<Vec<Value>> {
        let result = ResultId {
            request: RequestId(request),
            statement,
        };
        self.harness
            .send(Command::FetchRows {
                result,
                start,
                count,
            })
            .await;
        loop {
            match self.harness.next_event().await {
                Event::RowsWindow {
                    result: found,
                    start: found_start,
                    rows,
                } if found == result && found_start == start => return rows,
                _ => {}
            }
        }
    }

    fn executed(&self) -> Vec<String> {
        self.log.executed.lock().unwrap().clone()
    }

    fn order(&self) -> Vec<&'static str> {
        self.log.order.lock().unwrap().clone()
    }
}

async fn wait_until(mut condition: impl FnMut() -> bool) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the condition never became true");
}

/// A short, ordered description of an event stream. Progress is skipped: its timing is not
/// part of the ordering guarantee.
fn shape(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::ConnectionStateChanged { state, .. } => Some(format!("connection {state:?}")),
            Event::SessionStateChanged { state, .. } => Some(format!("session {state:?}")),
            Event::ExecutionStarted { statements, .. } => {
                Some(format!("started {}", statements.len()))
            }
            Event::ResultStarted { result, .. } => {
                Some(format!("result_started {}", result.statement))
            }
            Event::ResultFinished { result, outcome } => Some(format!(
                "result_finished {} {:?}",
                result.statement, outcome.status
            )),
            Event::ExecutionFinished { status, .. } => Some(format!("finished {status:?}")),
            Event::TabClosed { .. } => Some("tab_closed".to_owned()),
            Event::RequestCompleted { .. } => Some("completed".to_owned()),
            Event::ResultProgress { .. } | Event::RowsWindow { .. } => None,
            other => Some(format!("{other:?}")),
        })
        .collect()
}

fn finished_status(events: &[Event]) -> ExecutionStatus {
    match events.last() {
        Some(Event::ExecutionFinished { status, .. }) => status.clone(),
        other => panic!("expected ExecutionFinished, got {other:?}"),
    }
}

fn result_outcome(events: &[Event], statement: u32) -> dbased_commands::ResultOutcome {
    events
        .iter()
        .find_map(|event| match event {
            Event::ResultFinished { result, outcome } if result.statement == statement => {
                Some(outcome.clone())
            }
            _ => None,
        })
        .expect("the statement has a ResultFinished")
}

#[tokio::test]
async fn two_statements_give_two_results_in_order() {
    let fixture = Fixture::connected().await;
    let events = fixture.run_all(1, "rows 2; rows 3").await;
    assert_eq!(
        shape(&events),
        [
            "session Open",
            "started 2",
            "result_started 0",
            "result_finished 0 Completed",
            "result_started 1",
            "result_finished 1 Completed",
            "finished Completed",
        ]
    );
    assert_eq!(fixture.executed(), ["rows 2", "rows 3"]);
    assert_eq!(fixture.fetch(1, 0, 0, 10).await.len(), 2);
    assert_eq!(fixture.fetch(1, 1, 0, 10).await.len(), 3);
}

#[tokio::test]
async fn execution_started_reports_spans_into_the_script() {
    let fixture = Fixture::connected().await;
    let events = fixture.run_all(1, "rows 1;\n  rows 2").await;
    let Some(Event::ExecutionStarted { statements, .. }) = events
        .iter()
        .find(|e| matches!(e, Event::ExecutionStarted { .. }))
    else {
        panic!("no ExecutionStarted");
    };
    let spans: Vec<_> = statements
        .iter()
        .map(|span| (span.index, span.start, span.end))
        .collect();
    assert_eq!(spans, [(0, 0, 6), (1, 10, 16)]);
}

#[tokio::test]
async fn current_runs_only_the_statement_under_the_cursor() {
    let fixture = Fixture::connected().await;
    let events = fixture
        .run(
            1,
            "rows 1; rows 2; rows 3",
            ExecuteTarget::Current { cursor: 10 },
        )
        .await;
    assert_eq!(finished_status(&events), ExecutionStatus::Completed);
    assert_eq!(fixture.executed(), ["rows 2"]);
    assert_eq!(fixture.fetch(1, 0, 0, 10).await.len(), 2);
}

#[tokio::test]
async fn selection_runs_the_selected_statements_with_spans_in_script_offsets() {
    let fixture = Fixture::connected().await;
    let script = "rows 1; rows 2; rows 3";
    let events = fixture
        .run(1, script, ExecuteTarget::Selection { start: 8, end: 22 })
        .await;
    assert_eq!(fixture.executed(), ["rows 2", "rows 3"]);
    let Some(Event::ExecutionStarted { statements, .. }) = events
        .iter()
        .find(|e| matches!(e, Event::ExecutionStarted { .. }))
    else {
        panic!("no ExecutionStarted");
    };
    assert_eq!(&script[statements[0].start..statements[0].end], "rows 2");
    assert_eq!(&script[statements[1].start..statements[1].end], "rows 3");
}

#[tokio::test]
async fn an_empty_script_or_bad_selection_ends_the_request_without_running() {
    let fixture = Fixture::connected().await;
    let events = fixture.run_all(1, "  -- nothing here\n ;").await;
    assert_eq!(failed_kind(events.last().unwrap()), ErrorKind::EmptyScript);
    let events = fixture
        .run(2, "rows 1", ExecuteTarget::Selection { start: 5, end: 99 })
        .await;
    assert_eq!(failed_kind(events.last().unwrap()), ErrorKind::Internal);
    assert!(fixture.executed().is_empty());
    assert_eq!(fixture.log.opens.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_failed_statement_stops_the_script_and_keeps_its_error_position() {
    let fixture = Fixture::connected().await;
    let events = fixture.run_all(1, "rows 1; fail 3; rows 2").await;
    assert_eq!(
        finished_status(&events),
        ExecutionStatus::StoppedOnError { statement: 1 }
    );
    assert_eq!(fixture.executed(), ["rows 1", "fail 3"]);
    let failed = result_outcome(&events, 1);
    assert_eq!(failed.status, ResultStatus::Failed);
    assert_eq!(failed.error.unwrap().position, Some(3));
    // A failed statement still has a ResultStarted (empty columns) before its ResultFinished.
    let started: Vec<_> = shape(&events)
        .into_iter()
        .filter(|line| line.starts_with("result_started"))
        .collect();
    assert_eq!(started, ["result_started 0", "result_started 1"]);
}

#[tokio::test]
async fn a_truncated_statement_does_not_stop_the_script() {
    let fixture = Fixture::connected().await;
    let events = fixture.run_all(1, "truncate 3; rows 1").await;
    assert_eq!(finished_status(&events), ExecutionStatus::Completed);
    let truncated = result_outcome(&events, 0);
    assert_eq!(truncated.status, ResultStatus::Truncated);
    assert!(truncated.partial);
    assert_eq!(fixture.executed().len(), 2);
}

#[tokio::test]
async fn cancel_stops_the_script_and_a_stray_cancel_is_ignored() {
    let fixture = Fixture::connected().await;
    fixture
        .send_execute(1, TAB, "rows 1; sleep; rows 1", ExecuteTarget::All)
        .await;
    wait_until(|| fixture.log.started.load(Ordering::SeqCst) >= 2).await;

    for request in [99, 1] {
        fixture
            .harness
            .send(Command::Cancel {
                tab: TAB,
                request: RequestId(request),
            })
            .await;
    }
    let events = fixture.harness.until_terminal(RequestId(1)).await;
    assert_eq!(
        finished_status(&events),
        ExecutionStatus::Cancelled { statement: 1 }
    );
    assert_eq!(result_outcome(&events, 1).status, ResultStatus::Cancelled);
    assert_eq!(fixture.executed(), ["rows 1", "sleep"]);

    // The tab is free again.
    let events = fixture.run_all(2, "rows 1").await;
    assert_eq!(finished_status(&events), ExecutionStatus::Completed);
}

#[tokio::test]
async fn a_busy_tab_rejects_a_second_execute() {
    let fixture = Fixture::connected().await;
    fixture
        .send_execute(1, TAB, "sleep", ExecuteTarget::All)
        .await;
    wait_until(|| fixture.log.started.load(Ordering::SeqCst) >= 1).await;

    fixture
        .send_execute(2, TAB, "rows 1", ExecuteTarget::All)
        .await;
    let events = fixture.harness.until_terminal(RequestId(2)).await;
    assert_eq!(failed_kind(events.last().unwrap()), ErrorKind::TabBusy);

    fixture
        .harness
        .send(Command::Cancel {
            tab: TAB,
            request: RequestId(1),
        })
        .await;
    fixture.harness.until_terminal(RequestId(1)).await;
}

#[tokio::test]
async fn closing_a_tab_mid_run_drains_closes_the_session_and_releases_results() {
    let fixture = Fixture::connected().await;
    fixture
        .send_execute(1, TAB, "rows 4; sleep", ExecuteTarget::All)
        .await;
    wait_until(|| fixture.log.started.load(Ordering::SeqCst) >= 2).await;

    fixture.harness.send(Command::CloseTab { tab: TAB }).await;
    let mut seen = Vec::new();
    loop {
        let event = fixture.harness.next_event().await;
        let closed = matches!(event, Event::TabClosed { .. });
        seen.push(event);
        if closed {
            break;
        }
    }
    let lines = shape(&seen);
    assert!(
        lines.contains(&"finished Cancelled { statement: 1 }".to_owned()),
        "{lines:?}"
    );
    assert_eq!(lines.last().unwrap(), "tab_closed");
    assert_eq!(fixture.log.closes.load(Ordering::SeqCst), 1);

    // The released result is gone: a fetch answers nothing.
    fixture
        .harness
        .send(Command::FetchRows {
            result: ResultId {
                request: RequestId(1),
                statement: 0,
            },
            start: 0,
            count: 10,
        })
        .await;
    let events = fixture
        .harness
        .request(
            Command::LoadChildren {
                request: RequestId(5),
                connection: fixture.connection,
                node: dbased_commands::MetadataNode::Connection,
            },
            RequestId(5),
        )
        .await;
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::RowsWindow { .. }))
    );
}

#[tokio::test]
async fn dropping_the_command_sender_closes_every_session_before_the_pool() {
    let fixture = Fixture::connected().await;
    fixture.open_tab(TabId(2)).await;
    fixture.run_all(1, "rows 1").await;
    fixture
        .send_execute(2, TabId(2), "sleep", ExecuteTarget::All)
        .await;
    wait_until(|| fixture.log.started.load(Ordering::SeqCst) >= 2).await;

    let log = fixture.log.clone();
    fixture.harness.shut_down().await;
    assert_eq!(log.closes.load(Ordering::SeqCst), 2);
    let order = log.order.lock().unwrap().clone();
    let driver_close = order
        .iter()
        .position(|step| *step == "driver_close")
        .unwrap();
    let last_session_close = order
        .iter()
        .rposition(|step| *step == "session_close")
        .unwrap();
    assert!(last_session_close < driver_close, "{order:?}");
}

#[tokio::test]
async fn disconnect_releases_tab_sessions_first_and_the_tab_reopens_on_demand() {
    let fixture = Fixture::connected().await;
    fixture.run_all(1, "rows 1").await;

    let events = fixture
        .harness
        .request(
            Command::Disconnect {
                request: RequestId(2),
                id: fixture.connection,
            },
            RequestId(2),
        )
        .await;
    assert!(
        shape(&events).contains(&"session NotOpen".to_owned()),
        "{events:?}"
    );
    let order = fixture.order();
    let session_close = order
        .iter()
        .position(|step| *step == "session_close")
        .unwrap();
    let driver_close = order
        .iter()
        .position(|step| *step == "driver_close")
        .unwrap();
    assert!(session_close < driver_close, "{order:?}");

    // The next run connects again (Connecting is visible) and opens a fresh session.
    let events = fixture.run_all(3, "rows 1").await;
    let lines = shape(&events);
    assert_eq!(lines[0], "connection Connecting");
    assert_eq!(lines[1], "connection Connected");
    assert_eq!(finished_status(&events), ExecutionStatus::Completed);
    assert_eq!(fixture.log.opens.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn deleting_a_connection_closes_its_tabs_before_it_completes() {
    let fixture = Fixture::connected().await;
    fixture.run_all(1, "rows 1").await;
    let events = fixture
        .harness
        .request(
            Command::DeleteConnection {
                request: RequestId(2),
                id: fixture.connection,
            },
            RequestId(2),
        )
        .await;
    let lines = shape(&events);
    let closed = lines.iter().position(|line| line == "tab_closed").unwrap();
    assert_eq!(
        lines[closed + 1..]
            .iter()
            .filter(|l| *l == "completed")
            .count(),
        1
    );
    assert_eq!(fixture.log.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn running_on_a_disconnected_connection_connects_first_with_the_saved_password() {
    let fixture = Fixture::disconnected().await;
    fixture.open_tab(TAB).await;
    let events = fixture.run_all(1, "rows 1").await;
    assert_eq!(
        shape(&events),
        [
            "connection Connecting",
            "connection Connected",
            "session Open",
            "started 1",
            "result_started 0",
            "result_finished 0 Completed",
            "finished Completed",
        ]
    );
}

#[tokio::test]
async fn a_failed_auto_connect_ends_the_execute_with_that_error_and_frees_the_tab() {
    let connector = ScriptedConnector {
        required_password: Some("another"),
        ..ScriptedConnector::default()
    };
    let fixture = Fixture::with_connector(connector).await;
    fixture.open_tab(TAB).await;
    let events = fixture.run_all(1, "rows 1").await;
    assert_eq!(
        failed_kind(events.last().unwrap()),
        ErrorKind::Authentication
    );
    assert!(shape(&events).contains(&"connection Disconnected".to_owned()));

    let events = fixture.run_all(2, "rows 1").await;
    assert_eq!(
        failed_kind(events.last().unwrap()),
        ErrorKind::Authentication
    );
}

#[tokio::test]
async fn auto_connect_without_a_saved_password_asks_for_one() {
    let harness = Harness::new().await;
    let id = harness.save(1, None, "db").await;
    harness
        .send(Command::OpenTab {
            tab: TAB,
            connection: id,
        })
        .await;
    harness
        .send(Command::Execute {
            request: RequestId(2),
            tab: TAB,
            script: "rows 1".into(),
            target: ExecuteTarget::All,
        })
        .await;
    let events = harness.until_terminal(RequestId(2)).await;
    assert_eq!(
        failed_kind(events.last().unwrap()),
        ErrorKind::PasswordRequired
    );
}

#[tokio::test]
async fn fetch_rows_serves_windows_at_start_middle_end_and_caps_the_count() {
    let fixture = Fixture::connected().await;
    fixture.run_all(1, "rows 300").await;
    assert_eq!(fixture.fetch(1, 0, 0, 4).await.len(), 4);
    assert_eq!(fixture.fetch(1, 0, 150, 4).await[0], vec![Value::Int(150)]);
    assert_eq!(fixture.fetch(1, 0, 298, 10).await.len(), 2);
    assert!(fixture.fetch(1, 0, 5_000, 10).await.is_empty());
    assert_eq!(fixture.fetch(1, 0, 0, 1_000).await.len(), 200);
}

#[tokio::test]
async fn a_rerun_gets_new_results_and_the_old_ones_stay_until_released() {
    let fixture = Fixture::connected().await;
    fixture.run_all(1, "rows 2").await;
    fixture.run_all(2, "rows 3").await;
    assert_eq!(fixture.fetch(1, 0, 0, 10).await.len(), 2);
    assert_eq!(fixture.fetch(2, 0, 0, 10).await.len(), 3);

    fixture
        .harness
        .send(Command::ReleaseResult {
            result: ResultId {
                request: RequestId(1),
                statement: 0,
            },
        })
        .await;
    fixture
        .harness
        .send(Command::FetchRows {
            result: ResultId {
                request: RequestId(1),
                statement: 0,
            },
            start: 0,
            count: 10,
        })
        .await;
    assert_eq!(fixture.fetch(2, 0, 0, 10).await.len(), 3);
}

#[tokio::test]
async fn progress_never_blocks_the_read_when_the_event_channel_is_full() {
    let connector = Arc::new(ScriptedConnector::default());
    let log = connector.sessions.clone();
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::start_sized(
        directory.path().join("config.toml"),
        Arc::new(InMemorySecretStore::default()),
        connector,
        3,
    )
    .await;
    let fixture = Fixture::finish(harness, log).await;
    fixture
        .harness
        .connect(901, fixture.connection, PasswordSource::None)
        .await;
    fixture.open_tab(TAB).await;

    // Three events fill the channel; nothing reads until the whole result is buffered.
    fixture
        .send_execute(1, TAB, "slow 5", ExecuteTarget::All)
        .await;
    wait_until(|| fixture.log.slow_rows_sent.load(Ordering::SeqCst)).await;

    let events = fixture.harness.until_terminal(RequestId(1)).await;
    let progress = events
        .iter()
        .filter(|event| matches!(event, Event::ResultProgress { .. }))
        .count();
    assert_eq!(progress, 0, "progress was dropped, not queued: {events:?}");
    assert_eq!(result_outcome(&events, 0).rows_received, 5);
}

#[tokio::test]
async fn progress_is_coalesced_and_reports_the_buffered_count() {
    let fixture = Fixture::connected().await;
    let events = fixture.run_all(1, "slow 4").await;
    let counts: Vec<u64> = events
        .iter()
        .filter_map(|event| match event {
            Event::ResultProgress { rows_buffered, .. } => Some(*rows_buffered),
            _ => None,
        })
        .collect();
    assert!(!counts.is_empty());
    assert!(counts.windows(2).all(|pair| pair[0] < pair[1]));
}

#[tokio::test]
async fn a_session_closed_by_the_driver_is_closed_here_too_and_reopened_next_run() {
    let fixture = Fixture::connected().await;
    let events = fixture.run_all(1, "closed").await;
    assert!(shape(&events).contains(&"session Reset { reason: DrainTimeout }".to_owned()));
    assert_eq!(fixture.log.closes.load(Ordering::SeqCst), 1);

    fixture.run_all(2, "rows 1").await;
    assert_eq!(fixture.log.opens.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_session_level_failure_aborts_closes_and_resets() {
    let fixture = Fixture::connected().await;
    let events = fixture.run_all(1, "rows 1; dead; rows 1").await;
    let ExecutionStatus::Aborted { statement, error } = finished_status(&events) else {
        panic!("expected Aborted");
    };
    assert_eq!((statement, error.kind), (1, ErrorKind::SessionClosed));
    assert!(shape(&events).contains(&"session Reset { reason: ConnectionError }".to_owned()));
    assert_eq!(fixture.log.closes.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.executed(), ["rows 1", "dead"]);
}

#[tokio::test]
async fn a_session_failure_mid_result_still_finishes_the_started_result() {
    let fixture = Fixture::connected().await;
    let events = fixture.run_all(1, "deadrows").await;
    let lines = shape(&events);
    let started = lines.iter().position(|l| l == "result_started 0").unwrap();
    assert_eq!(lines[started + 1], "result_finished 0 Failed");
    let outcome = result_outcome(&events, 0);
    assert!(outcome.partial);
    assert_eq!(outcome.rows_received, 2);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::ResultStarted { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn an_aborted_transaction_keeps_the_session_and_says_so() {
    let fixture = Fixture::connected().await;
    let events = fixture.run_all(1, "aborted").await;
    assert!(shape(&events).contains(&"session TransactionAborted".to_owned()));
    assert_eq!(fixture.log.closes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_statement_that_carried_extra_statements_is_reported_as_an_internal_error() {
    let fixture = Fixture::connected().await;
    let events = fixture.run_all(1, "extra").await;
    let reported = events
        .iter()
        .any(|event| matches!(event, Event::Error { error } if error.kind == ErrorKind::Internal));
    assert!(reported, "{events:?}");
}
