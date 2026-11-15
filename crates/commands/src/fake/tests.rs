#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use secrecy::SecretString;

use super::contract::{
    check_execution_order, check_one_terminal, collect_until_terminal, event_shape,
    send_and_collect,
};
use super::*;
use crate::{
    COMMAND_CAPACITY, ConnectionSettings, DatabaseKind, EVENT_CAPACITY, ErrorKind, TlsMode, bridge,
};

const TAB: TabId = TabId(1);
const POSTGRES: ConnectionId = ConnectionId(1);

fn opened() -> FakeCore {
    let mut core = FakeCore::new();
    core.handle(Command::OpenTab {
        tab: TAB,
        connection: POSTGRES,
    });
    core
}

fn execute(core: &mut FakeCore, request: u64, script: &str) -> Vec<Event> {
    core.handle(Command::Execute {
        request: RequestId(request),
        tab: TAB,
        script: script.to_string(),
        target: ExecuteTarget::All,
    })
}

fn shapes(events: &[Event]) -> Vec<String> {
    events.iter().map(event_shape).collect()
}

/// Execution events only, without connect and session noise.
fn execution_shapes(events: &[Event]) -> Vec<String> {
    shapes(events)
        .into_iter()
        .filter(|shape| {
            shape.starts_with("Execution")
                || shape.starts_with("Result")
                || shape.starts_with("RequestFailed")
        })
        .collect()
}

#[test]
fn scripts_produce_the_expected_event_sequences() {
    let cases: [(&str, &[&str]); 6] = [
        (
            "select 3",
            &[
                "ExecutionStarted(1)",
                "ResultStarted",
                "ResultProgress",
                "ResultFinished(Completed)",
                "ExecutionFinished(Completed)",
            ],
        ),
        (
            "select 1; select 2",
            &[
                "ExecutionStarted(2)",
                "ResultStarted",
                "ResultProgress",
                "ResultFinished(Completed)",
                "ResultStarted",
                "ResultProgress",
                "ResultFinished(Completed)",
                "ExecutionFinished(Completed)",
            ],
        ),
        (
            "select 1; fail; select 2",
            &[
                "ExecutionStarted(3)",
                "ResultStarted",
                "ResultProgress",
                "ResultFinished(Completed)",
                "ResultStarted",
                "ResultFinished(Failed)",
                "ExecutionFinished(StoppedOnError { statement: 1 })",
            ],
        ),
        (
            "update t set a = 1",
            &[
                "ExecutionStarted(1)",
                "ResultStarted",
                "ResultFinished(Completed)",
                "ExecutionFinished(Completed)",
            ],
        ),
        (
            "select 6000000",
            &[
                "ExecutionStarted(1)",
                "ResultStarted",
                "ResultProgress",
                "ResultFinished(Truncated)",
                "ExecutionFinished(Completed)",
            ],
        ),
        (
            "select 1; sleep; select 2",
            &[
                "ExecutionStarted(3)",
                "ResultStarted",
                "ResultProgress",
                "ResultFinished(Completed)",
                "ResultStarted",
            ],
        ),
    ];
    for (script, expected) in cases {
        let mut core = opened();
        let events = execute(&mut core, 10, script);
        assert_eq!(execution_shapes(&events), expected, "script: {script}");
        check_execution_order(&events)
            .or_else(|message| {
                // @NOTE: a sleeping script is still running, so it has no ExecutionFinished yet.
                if script.contains("sleep") {
                    Ok(())
                } else {
                    Err(message)
                }
            })
            .unwrap();
    }
}

#[test]
fn every_request_ends_with_exactly_one_terminal_event() {
    let mut core = opened();
    let settings = ConnectionSettings {
        name: "new".into(),
        kind: DatabaseKind::MySql,
        host: "h".into(),
        port: 1,
        user: "u".into(),
        database: "d".into(),
        tls: TlsMode::Disable,
    };
    let requests: Vec<(u64, Command)> = vec![
        (
            1,
            Command::SaveConnection {
                request: RequestId(1),
                id: None,
                settings: settings.clone(),
                password: PasswordUpdate::Save(SecretString::from("pw")),
            },
        ),
        (
            2,
            Command::TestConnection {
                request: RequestId(2),
                id: None,
                settings,
                password: PasswordSource::Typed(SecretString::from("pw")),
            },
        ),
        (
            3,
            Command::Connect {
                request: RequestId(3),
                id: POSTGRES,
                password: PasswordSource::Saved,
            },
        ),
        (
            4,
            Command::LoadChildren {
                request: RequestId(4),
                connection: POSTGRES,
                node: MetadataNode::Connection,
            },
        ),
        (
            5,
            Command::Execute {
                request: RequestId(5),
                tab: TAB,
                script: "select 2".into(),
                target: ExecuteTarget::All,
            },
        ),
        (
            6,
            Command::Execute {
                request: RequestId(6),
                tab: TabId(99),
                script: "select 2".into(),
                target: ExecuteTarget::All,
            },
        ),
        (
            7,
            Command::Execute {
                request: RequestId(7),
                tab: TAB,
                script: "  ".into(),
                target: ExecuteTarget::All,
            },
        ),
        (
            8,
            Command::Disconnect {
                request: RequestId(8),
                id: POSTGRES,
            },
        ),
        (
            9,
            Command::DeleteConnection {
                request: RequestId(9),
                id: ConnectionId(77),
            },
        ),
    ];
    for (request, command) in requests {
        let events = core.handle(command);
        check_one_terminal(&events, RequestId(request)).unwrap();
    }
}

fn fetch(core: &mut FakeCore, start: u64, count: u32) -> (u64, Vec<Vec<Value>>) {
    let events = core.handle(Command::FetchRows {
        result: ResultId {
            request: RequestId(1),
            statement: 0,
        },
        start,
        count,
    });
    match events.into_iter().next() {
        Some(Event::RowsWindow { start, rows, .. }) => (start, rows),
        other => panic!("expected RowsWindow, got {other:?}"),
    }
}

#[test]
fn rows_window_respects_bounds_at_start_middle_and_end() {
    let mut core = opened();
    execute(&mut core, 1, "select 1000");

    let (start, rows) = fetch(&mut core, 0, 200);
    assert_eq!((start, rows.len()), (0, 200));
    assert_eq!(rows[0][0], Value::Int(0));

    let (start, rows) = fetch(&mut core, 500, 200);
    assert_eq!((start, rows.len()), (500, 200));
    assert_eq!(rows[0][0], Value::Int(500));
    assert_eq!(rows[199][0], Value::Int(699));

    let (_, rows) = fetch(&mut core, 900, 200);
    assert_eq!(rows.len(), 100);
    assert_eq!(rows[99][0], Value::Int(999));

    let (_, rows) = fetch(&mut core, 1000, 200);
    assert!(rows.is_empty());
    let (_, rows) = fetch(&mut core, 5000, 200);
    assert!(rows.is_empty());
}

#[test]
fn rows_are_deterministic_and_cover_every_value_variant() {
    let mut core = opened();
    execute(&mut core, 1, "select 10");
    let (_, first) = fetch(&mut core, 0, 10);
    let (_, second) = fetch(&mut core, 0, 10);
    assert_eq!(first, second);

    let discriminants: std::collections::HashSet<_> =
        first.iter().flatten().map(std::mem::discriminant).collect();
    assert_eq!(discriminants.len(), 13);
    assert_eq!(first[0].len(), rows::select_columns().len());
}

#[test]
fn five_million_rows_cost_nothing_to_serve() {
    let mut core = opened();
    let events = execute(&mut core, 1, "select 5000000");
    assert!(shapes(&events).contains(&"ResultFinished(Completed)".to_string()));
    let (_, rows) = fetch(&mut core, 4_999_900, 200);
    assert_eq!(rows.len(), 100);
    assert_eq!(rows[99][0], Value::Int(4_999_999));
}

#[test]
fn sleep_runs_until_cancelled() {
    let mut core = opened();
    let events = execute(&mut core, 5, "sleep; select 1");
    assert!(
        !shapes(&events)
            .iter()
            .any(|shape| shape.starts_with("ExecutionFinished"))
    );

    let busy = execute(&mut core, 6, "select 1");
    assert_eq!(shapes(&busy), ["RequestFailed(TabBusy)"]);

    let cancelled = core.handle(Command::Cancel {
        tab: TAB,
        request: RequestId(5),
    });
    assert_eq!(
        shapes(&cancelled),
        [
            "ResultFinished(Cancelled)",
            "ExecutionFinished(Cancelled { statement: 0 })"
        ]
    );
    let mut all = events;
    all.extend(cancelled);
    check_execution_order(&all).unwrap();
    check_one_terminal(&all, RequestId(5)).unwrap();

    let after = execute(&mut core, 7, "select 1");
    assert!(shapes(&after).contains(&"ExecutionFinished(Completed)".to_string()));
}

#[test]
fn cancel_of_another_request_is_ignored() {
    let mut core = opened();
    execute(&mut core, 5, "sleep");
    let events = core.handle(Command::Cancel {
        tab: TAB,
        request: RequestId(99),
    });
    assert!(events.is_empty());
}

#[test]
fn closing_a_sleeping_tab_cancels_then_closes() {
    let mut core = opened();
    execute(&mut core, 5, "sleep");
    let events = core.handle(Command::CloseTab { tab: TAB });
    assert_eq!(
        shapes(&events),
        [
            "ResultFinished(Cancelled)",
            "ExecutionFinished(Cancelled { statement: 0 })",
            "TabClosed"
        ]
    );
}

#[test]
fn fail_carries_a_position() {
    let mut core = opened();
    let events = execute(&mut core, 1, "fail");
    let position = |events: &[Event]| {
        events.iter().find_map(|event| match event {
            Event::ResultFinished { outcome, .. } => {
                outcome.error.as_ref().and_then(|error| error.position)
            }
            _ => None,
        })
    };
    assert_eq!(position(&events), Some(1));

    let events = execute(&mut core, 2, "fail here");
    assert_eq!(position(&events), Some(6));
}

#[test]
fn failed_statement_starts_with_empty_columns() {
    let mut core = opened();
    let events = execute(&mut core, 1, "fail");
    assert!(events.iter().any(|event| matches!(
        event,
        Event::ResultStarted { columns, .. } if columns.is_empty()
    )));
}

#[test]
fn execute_failures_use_the_documented_kinds() {
    let mut core = opened();
    let kind = |events: Vec<Event>| match events.into_iter().next() {
        Some(Event::RequestFailed { error, .. }) => error.kind,
        other => panic!("expected RequestFailed, got {other:?}"),
    };
    let unknown_tab = core.handle(Command::Execute {
        request: RequestId(1),
        tab: TabId(42),
        script: "select 1".into(),
        target: ExecuteTarget::All,
    });
    assert_eq!(kind(unknown_tab), ErrorKind::UnknownTab);
    assert_eq!(kind(execute(&mut core, 2, " ; ")), ErrorKind::EmptyScript);
    execute(&mut core, 3, "sleep");
    assert_eq!(kind(execute(&mut core, 4, "select 1")), ErrorKind::TabBusy);
}

#[test]
fn execute_targets_pick_statements_with_script_offsets() {
    let script = "select 1; select 22; select 333";
    let mut core = opened();
    let spans = |events: &[Event]| {
        events.iter().find_map(|event| match event {
            Event::ExecutionStarted { statements, .. } => Some(statements.clone()),
            _ => None,
        })
    };

    let current = core.handle(Command::Execute {
        request: RequestId(1),
        tab: TAB,
        script: script.into(),
        target: ExecuteTarget::Current { cursor: 12 },
    });
    let statements = spans(&current).unwrap();
    assert_eq!(statements.len(), 1);
    assert_eq!(&script[statements[0].start..statements[0].end], "select 22");

    let selection = core.handle(Command::Execute {
        request: RequestId(2),
        tab: TAB,
        script: script.into(),
        target: ExecuteTarget::Selection { start: 10, end: 31 },
    });
    let statements = spans(&selection).unwrap();
    assert_eq!(statements.len(), 2);
    assert_eq!(
        &script[statements[1].start..statements[1].end],
        "select 333"
    );
}

#[test]
fn execute_connects_the_connection_first() {
    let mut core = opened();
    let events = execute(&mut core, 1, "select 1");
    let shapes = shapes(&events);
    assert_eq!(shapes[0], "ConnectionStateChanged(Connecting)");
    assert_eq!(shapes[1], "ConnectionStateChanged(Connected)");
    assert_eq!(shapes[2], "SessionStateChanged(Open)");
    let again = execute(&mut core, 2, "select 1");
    assert_eq!(shapes_first(&again), "ExecutionStarted(1)");
}

fn shapes_first(events: &[Event]) -> String {
    event_shape(&events[0])
}

#[test]
fn connection_crud_and_metadata() {
    let mut core = FakeCore::new();
    let initial = core.initial_events();
    assert!(matches!(
        &initial[0],
        Event::ConnectionsChanged { connections } if connections.len() == 2
    ));

    let settings = match &initial[0] {
        Event::ConnectionsChanged { connections } => connections[0].settings.clone(),
        _ => unreachable!(),
    };
    let saved = core.handle(Command::SaveConnection {
        request: RequestId(1),
        id: None,
        settings,
        password: PasswordUpdate::Forget,
    });
    assert!(matches!(
        saved[1],
        Event::ConnectionSaved {
            id: ConnectionId(3),
            ..
        }
    ));

    let password_required = core.handle(Command::Connect {
        request: RequestId(2),
        id: ConnectionId(3),
        password: PasswordSource::Saved,
    });
    assert_eq!(
        shapes(&password_required),
        ["RequestFailed(PasswordRequired)"]
    );

    let children = core.handle(Command::LoadChildren {
        request: RequestId(3),
        connection: POSTGRES,
        node: MetadataNode::Schema {
            schema: "public".into(),
        },
    });
    assert!(matches!(
        &children[0],
        Event::ChildrenLoaded { items, .. } if items.len() == 3
    ));

    let deleted = core.handle(Command::DeleteConnection {
        request: RequestId(4),
        id: ConnectionId(3),
    });
    assert_eq!(shapes(&deleted), ["ConnectionsChanged", "RequestCompleted"]);
}

fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWaker(Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        thread::park();
    }
}

#[test]
fn serve_answers_over_the_bridge_and_stops_when_the_ui_leaves() {
    let (ui_link, core_link) = bridge(COMMAND_CAPACITY, EVENT_CAPACITY);
    let server = thread::spawn(move || block_on(FakeCore::new().serve(core_link)));

    block_on(async {
        let first = ui_link.events.recv().await.unwrap();
        assert_eq!(event_shape(&first), "ConnectionsChanged");

        ui_link
            .commands
            .send(Command::OpenTab {
                tab: TAB,
                connection: POSTGRES,
            })
            .await
            .unwrap();
        let events = send_and_collect(
            &ui_link.commands,
            &ui_link.events,
            Command::Execute {
                request: RequestId(1),
                tab: TAB,
                script: "select 5; select 6".into(),
                target: ExecuteTarget::All,
            },
            RequestId(1),
        )
        .await;
        check_one_terminal(&events, RequestId(1)).unwrap();
        check_execution_order(&events).unwrap();

        ui_link
            .commands
            .send(Command::Disconnect {
                request: RequestId(2),
                id: POSTGRES,
            })
            .await
            .unwrap();
        let events = collect_until_terminal(&ui_link.events, RequestId(2)).await;
        check_one_terminal(&events, RequestId(2)).unwrap();
    });

    drop(ui_link);
    server.join().unwrap();
}
