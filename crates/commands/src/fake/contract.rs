//! Contract-test scaffolding (docs/PROJECT.md 11.4, 11.5). It talks to any Core through
//! a command sender and an event receiver, so the fake and the real Core can run the
//! same scripts and be held to the same ordering guarantees and event shapes.

use async_channel::{Receiver, Sender};

use crate::{Command, Event, RequestId, ResultId};

/// True when `event` is the terminal event of `request` (docs/PROJECT.md 11.1).
pub fn is_terminal(event: &Event, request: RequestId) -> bool {
    match event {
        Event::RequestCompleted { request: other }
        | Event::RequestFailed { request: other, .. }
        | Event::ConnectionSaved { request: other, .. }
        | Event::ChildrenLoaded { request: other, .. }
        | Event::ExecutionFinished { request: other, .. } => *other == request,
        _ => false,
    }
}

/// Receives events up to and including the terminal event of `request`. The result is
/// short of its terminal event only when the channel closed first.
pub async fn collect_until_terminal(events: &Receiver<Event>, request: RequestId) -> Vec<Event> {
    let mut collected = Vec::new();
    while let Ok(event) = events.recv().await {
        let terminal = is_terminal(&event, request);
        collected.push(event);
        if terminal {
            break;
        }
    }
    collected
}

/// Sends `command` and collects the events of `request` up to its terminal event.
pub async fn send_and_collect(
    commands: &Sender<Command>,
    events: &Receiver<Event>,
    command: Command,
    request: RequestId,
) -> Vec<Event> {
    if commands.send(command).await.is_err() {
        return Vec::new();
    }
    collect_until_terminal(events, request).await
}

/// Checks that `request` has exactly one terminal event and that it comes last.
pub fn check_one_terminal(events: &[Event], request: RequestId) -> Result<(), String> {
    let terminals = events
        .iter()
        .filter(|event| is_terminal(event, request))
        .count();
    if terminals != 1 {
        return Err(format!("expected one terminal event, found {terminals}"));
    }
    match events.last() {
        Some(last) if is_terminal(last, request) => Ok(()),
        _ => Err("the terminal event is not last".to_string()),
    }
}

/// Checks the ordering guarantee of one execution (docs/PROJECT.md 11.1):
/// `ExecutionStarted`, then per statement `ResultStarted`, `ResultProgress`*,
/// `ResultFinished`, then `ExecutionFinished`. Events of other kinds are ignored.
pub fn check_execution_order(events: &[Event]) -> Result<(), String> {
    let mut started = false;
    let mut finished = false;
    let mut open_result: Option<ResultId> = None;
    let mut next_statement = 0_u32;
    for event in events {
        match event {
            Event::ExecutionStarted { .. } if started => {
                return Err("second ExecutionStarted".to_string());
            }
            Event::ExecutionStarted { .. } => started = true,
            Event::ResultStarted { result, .. } => {
                if !started || finished || open_result.is_some() {
                    return Err(format!("misplaced ResultStarted {result:?}"));
                }
                if result.statement != next_statement {
                    return Err(format!("result {result:?} out of order"));
                }
                open_result = Some(*result);
            }
            Event::ResultProgress { result, .. } => {
                if open_result != Some(*result) {
                    return Err(format!(
                        "progress for a result that is not open: {result:?}"
                    ));
                }
            }
            Event::ResultFinished { result, .. } => {
                if open_result != Some(*result) {
                    return Err(format!("finish for a result that is not open: {result:?}"));
                }
                open_result = None;
                next_statement += 1;
            }
            Event::ExecutionFinished { .. } => {
                if !started || finished || open_result.is_some() {
                    return Err("misplaced ExecutionFinished".to_string());
                }
                finished = true;
            }
            _ => {}
        }
    }
    if started && !finished {
        return Err("ExecutionFinished is missing".to_string());
    }
    Ok(())
}

/// The event's variant and, where it carries one, its status; ids and payloads are left
/// out so two Cores can be compared by shape.
pub fn event_shape(event: &Event) -> String {
    match event {
        Event::ConnectionsChanged { .. } => "ConnectionsChanged".to_string(),
        Event::ConnectionStateChanged { state, .. } => format!("ConnectionStateChanged({state:?})"),
        Event::ConnectionSaved { .. } => "ConnectionSaved".to_string(),
        Event::RequestCompleted { .. } => "RequestCompleted".to_string(),
        Event::RequestFailed { error, .. } => format!("RequestFailed({:?})", error.kind),
        Event::TabClosed { .. } => "TabClosed".to_string(),
        Event::SessionStateChanged { state, .. } => format!("SessionStateChanged({state:?})"),
        Event::ExecutionStarted { statements, .. } => {
            format!("ExecutionStarted({})", statements.len())
        }
        Event::ResultStarted { .. } => "ResultStarted".to_string(),
        Event::ResultProgress { .. } => "ResultProgress".to_string(),
        Event::ResultFinished { outcome, .. } => format!("ResultFinished({:?})", outcome.status),
        Event::ExecutionFinished { status, .. } => format!("ExecutionFinished({status:?})"),
        Event::RowsWindow { .. } => "RowsWindow".to_string(),
        Event::ChildrenLoaded { .. } => "ChildrenLoaded".to_string(),
        Event::Error { error } => format!("Error({:?})", error.kind),
    }
}
