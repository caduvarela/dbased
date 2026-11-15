#![allow(clippy::unwrap_used, clippy::expect_used)]

use dbased_commands::{
    Command, ConnectionId, ExecuteTarget, MetadataNode, ResultId, ResultStatus, TabId,
    fake::FakeCore,
};

use super::*;
use crate::state::{
    NodeKey, TabStatus,
    tabs::{Activity, LastRun},
};

/// Plays commands through the fake and feeds every event back into the state, the way the
/// app does, until nothing is left to send.
struct Harness {
    state: UiState,
    core: FakeCore,
}

impl Harness {
    fn new() -> Self {
        let core = FakeCore::new();
        let mut harness = Self {
            state: UiState::new(),
            core,
        };
        for event in harness.core.initial_events() {
            let commands = harness.state.apply(event);
            harness.run(commands);
        }
        harness
    }

    fn run(&mut self, commands: Vec<Command>) {
        let mut queue = commands;
        while let Some(command) = queue.pop() {
            for event in self.core.handle(command) {
                queue.extend(self.state.apply(event));
            }
        }
    }

    fn first_tab(&self) -> TabId {
        self.state
            .tabs
            .iter()
            .next()
            .map(|tab| tab.id)
            .expect("a tab is open")
    }

    fn execute(&mut self, script: &str) -> TabId {
        let tab = self.first_tab();
        let commands = self
            .state
            .execute(tab, script.to_string(), ExecuteTarget::All);
        self.run(commands);
        tab
    }
}

#[test]
fn the_first_connection_list_opens_a_tab() {
    let harness = Harness::new();
    assert_eq!(harness.state.connections.items().len(), 2);
    assert_eq!(harness.state.tabs.len(), 1);
    let tab = harness.state.tabs.active().expect("active tab");
    assert_eq!(tab.connection, ConnectionId(1));
}

#[test]
fn a_later_connection_list_does_not_open_more_tabs() {
    let mut harness = Harness::new();
    let commands = harness
        .state
        .apply(dbased_commands::Event::ConnectionsChanged {
            connections: harness.state.connections.items().to_vec(),
        });
    assert!(commands.is_empty());
    assert_eq!(harness.state.tabs.len(), 1);
}

#[test]
fn a_select_produces_a_finished_result_and_an_idle_tab() {
    let mut harness = Harness::new();
    let tab = harness.execute("select 3");
    let model = harness.state.tabs.get(tab).expect("tab");
    assert_eq!(model.activity, Activity::Idle);
    assert_eq!(model.last_run, Some(LastRun::Completed));
    assert_eq!(model.results.len(), 1);
    let result = harness.state.result(model.results[0]).expect("result");
    assert_eq!(result.row_count(), 3);
    assert!(result.has_grid());
    assert_eq!(result.status(), Some(ResultStatus::Completed));
    assert_eq!(harness.state.tab_status(tab), Some(TabStatus::Connected));
}

#[test]
fn several_statements_make_one_result_each() {
    let mut harness = Harness::new();
    let tab = harness.execute("select 2; select 5");
    let model = harness.state.tabs.get(tab).expect("tab");
    assert_eq!(model.results.len(), 2);
    assert_eq!(model.statements.len(), 2);
    let counts: Vec<u64> = model
        .results
        .iter()
        .filter_map(|id| harness.state.result(*id))
        .map(|result| result.row_count())
        .collect();
    assert_eq!(counts, vec![2, 5]);
}

#[test]
fn a_failing_statement_stops_the_run_and_keeps_the_error() {
    let mut harness = Harness::new();
    let tab = harness.execute("select 1; fail; select 2");
    let model = harness.state.tabs.get(tab).expect("tab");
    assert_eq!(
        model.last_run,
        Some(LastRun::StoppedOnError { statement: 1 })
    );
    assert_eq!(model.results.len(), 2);
    let failed = harness.state.result(model.results[1]).expect("result");
    assert_eq!(failed.status(), Some(ResultStatus::Failed));
    assert!(failed.status_line().starts_with("Failed: syntax error"));
}

#[test]
fn a_huge_select_is_truncated() {
    let mut harness = Harness::new();
    let tab = harness.execute("select 9000000");
    let model = harness.state.tabs.get(tab).expect("tab");
    let result = harness.state.result(model.results[0]).expect("result");
    assert_eq!(result.status(), Some(ResultStatus::Truncated));
    assert_eq!(result.row_count(), 5_000_000);
}

#[test]
fn a_sleeping_statement_runs_until_cancelled() {
    let mut harness = Harness::new();
    let tab = harness.execute("sleep");
    assert!(matches!(
        harness.state.tabs.get(tab).map(|model| model.activity),
        Some(Activity::Running(_))
    ));
    assert_eq!(harness.state.tab_status(tab), Some(TabStatus::Running));
    assert!(harness.state.can_cancel(tab));
    assert!(!harness.state.can_run(tab));

    let commands = harness.state.cancel(tab);
    assert!(matches!(commands.as_slice(), [Command::Cancel { .. }]));
    assert_eq!(harness.state.tab_status(tab), Some(TabStatus::Cancelling));
    assert!(!harness.state.can_cancel(tab));
    harness.run(commands);

    let model = harness.state.tabs.get(tab).expect("tab");
    assert_eq!(model.activity, Activity::Idle);
    assert_eq!(model.last_run, Some(LastRun::Cancelled { statement: 0 }));
    let result = harness.state.result(model.results[0]).expect("result");
    assert_eq!(result.status(), Some(ResultStatus::Cancelled));
}

#[test]
fn a_busy_tab_ignores_a_second_run() {
    let mut harness = Harness::new();
    let tab = harness.execute("sleep");
    let commands = harness
        .state
        .execute(tab, "select 1".to_string(), ExecuteTarget::All);
    assert!(commands.is_empty());
}

#[test]
fn running_again_releases_the_previous_results() {
    let mut harness = Harness::new();
    let tab = harness.execute("select 2");
    let old: ResultId = harness.state.tabs.get(tab).expect("tab").results[0];
    let commands = harness
        .state
        .execute(tab, "select 4".to_string(), ExecuteTarget::All);
    assert!(matches!(
        commands.first(),
        Some(Command::ReleaseResult { result }) if *result == old
    ));
    assert!(harness.state.result(old).is_none());
    harness.run(commands);
    let model = harness.state.tabs.get(tab).expect("tab");
    assert_eq!(model.results.len(), 1);
}

#[test]
fn a_rejected_request_frees_the_tab_and_says_why() {
    let mut harness = Harness::new();
    let tab = harness.execute("   ");
    let model = harness.state.tabs.get(tab).expect("tab");
    assert_eq!(model.activity, Activity::Idle);
    assert!(
        matches!(&model.last_run, Some(LastRun::Failed { message }) if message == "nothing to execute")
    );
}

#[test]
fn the_explorer_loads_schemas_then_tables_lazily() {
    let mut harness = Harness::new();
    let connection = NodeKey::Connection(ConnectionId(1));
    let commands = harness.state.toggle_node(&connection);
    harness.run(commands);

    let nodes = harness.state.explorer.nodes(&harness.state.connections);
    let schemas: Vec<&str> = nodes[0]
        .children
        .iter()
        .map(|node| node.label.as_str())
        .collect();
    assert_eq!(schemas, vec!["public", "sales"]);
    assert!(nodes[0].children[0].children[0].label == "Loading...");

    let schema = nodes[0].children[0].key.clone().expect("schema key");
    assert_eq!(
        schema.metadata_node(),
        MetadataNode::Schema {
            schema: "public".to_string()
        }
    );
    let commands = harness.state.toggle_node(&schema);
    harness.run(commands);
    let nodes = harness.state.explorer.nodes(&harness.state.connections);
    let tables: Vec<&str> = nodes[0].children[0]
        .children
        .iter()
        .map(|node| node.label.as_str())
        .collect();
    assert_eq!(tables, vec!["orders", "users", "active_users"]);
}

#[test]
fn expanding_a_connection_connects_it() {
    let mut harness = Harness::new();
    let commands = harness
        .state
        .toggle_node(&NodeKey::Connection(ConnectionId(2)));
    harness.run(commands);
    let state = harness
        .state
        .connections
        .get(ConnectionId(2))
        .map(|connection| connection.state);
    assert_eq!(state, Some(dbased_commands::ConnectionState::Connected));
}

#[test]
fn rebinding_a_tab_swaps_its_id_and_connection() {
    let mut harness = Harness::new();
    let tab = harness.execute("select 2");
    let (fresh, commands) = harness
        .state
        .rebind_tab(tab, ConnectionId(2))
        .expect("rebind");
    assert_ne!(fresh, tab);
    assert!(matches!(
        commands.as_slice(),
        [Command::CloseTab { .. }, Command::OpenTab { .. }]
    ));
    harness.run(commands);
    let model = harness.state.tabs.get(fresh).expect("tab");
    assert_eq!(model.connection, ConnectionId(2));
    assert!(model.results.is_empty());
    assert!(harness.state.tabs.get(tab).is_none());
    assert_eq!(harness.state.tabs.active_id(), Some(fresh));
}

#[test]
fn a_running_tab_cannot_be_rebound() {
    let mut harness = Harness::new();
    let tab = harness.execute("sleep");
    assert!(harness.state.rebind_tab(tab, ConnectionId(2)).is_none());
}

#[test]
fn closing_a_tab_drops_its_results() {
    let mut harness = Harness::new();
    let tab = harness.execute("select 2");
    let result = harness.state.tabs.get(tab).expect("tab").results[0];
    let commands = harness.state.close_tab(tab);
    assert!(matches!(commands.as_slice(), [Command::CloseTab { .. }]));
    assert!(harness.state.result(result).is_none());
    assert!(harness.state.tabs.is_empty());
}

#[test]
fn releasing_a_result_tells_the_core_once() {
    let mut harness = Harness::new();
    let tab = harness.execute("select 2");
    let result = harness.state.tabs.get(tab).expect("tab").results[0];
    assert_eq!(harness.state.release_result(result).len(), 1);
    assert!(harness.state.release_result(result).is_empty());
    assert!(harness.state.tabs.get(tab).expect("tab").results.is_empty());
}

#[test]
fn a_failed_send_becomes_a_notice() {
    let mut state = UiState::new();
    state.send_failed(SendFailure::Full);
    assert_eq!(state.notice.as_deref(), Some("The Core is not responding."));
    state.dismiss_notice();
    assert!(state.notice.is_none());
    state.core_stopped();
    assert!(state.core_stopped);
    assert!(state.notice.is_some());
}

#[test]
fn an_unprompted_core_error_becomes_a_notice() {
    let mut state = UiState::new();
    state.apply(dbased_commands::Event::Error {
        error: dbased_commands::ErrorInfo {
            kind: dbased_commands::ErrorKind::Storage,
            message: "config unreadable".to_string(),
            database: None,
        },
    });
    assert_eq!(state.notice.as_deref(), Some("config unreadable"));
}
