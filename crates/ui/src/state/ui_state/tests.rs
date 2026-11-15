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

// Connection management.

mod connections {
    use dbased_commands::{ErrorInfo, ErrorKind, Event, PasswordUpdate, RequestId};
    use secrecy::SecretString;

    use super::*;
    use crate::state::{ConnectionForm, SaveStatus, TestStatus};

    fn valid_form() -> ConnectionForm {
        let mut form = ConnectionForm::new();
        form.name = "Reports".to_string();
        form.user = "reader".to_string();
        form
    }

    fn password_required() -> ErrorInfo {
        ErrorInfo {
            kind: ErrorKind::PasswordRequired,
            message: "a password is required".to_string(),
            database: None,
        }
    }

    #[test]
    fn an_invalid_form_sends_nothing() {
        let mut harness = Harness::new();
        let mut form = valid_form();
        form.host.clear();
        assert!(harness.state.test_connection(&form).is_err());
        assert!(harness.state.save_connection(&form).is_err());
        assert_eq!(harness.state.flow.test, TestStatus::Idle);
        assert_eq!(harness.state.flow.save, SaveStatus::Idle);
    }

    #[test]
    fn testing_a_connection_ends_in_success() {
        let mut harness = Harness::new();
        let commands = harness.state.test_connection(&valid_form()).unwrap();
        assert_eq!(harness.state.flow.test, TestStatus::Testing);
        harness.run(commands);
        assert_eq!(harness.state.flow.test, TestStatus::Success);
    }

    #[test]
    fn a_second_test_is_ignored_while_one_runs() {
        let mut harness = Harness::new();
        harness.state.test_connection(&valid_form()).unwrap();
        let again = harness.state.test_connection(&valid_form()).unwrap();
        assert!(again.is_empty());
    }

    #[test]
    fn a_failed_test_shows_the_cores_message() {
        let mut harness = Harness::new();
        let request = RequestId(900);
        harness
            .state
            .flow
            .track(request, crate::state::connection_flow::Pending::Test);
        harness.state.flow.test = TestStatus::Testing;
        harness.state.apply(Event::RequestFailed {
            request,
            error: ErrorInfo {
                kind: ErrorKind::Connection,
                message: "connection refused".to_string(),
                database: None,
            },
        });
        assert_eq!(
            harness.state.flow.test,
            TestStatus::Failed("connection refused".to_string())
        );
    }

    #[test]
    fn saving_a_new_connection_adds_it_with_its_password() {
        let mut harness = Harness::new();
        let mut form = valid_form();
        form.password = SecretString::from("pw".to_string());
        let commands = harness.state.save_connection(&form).unwrap();
        assert!(matches!(
            &commands[0],
            Command::SaveConnection {
                id: None,
                password: PasswordUpdate::Save(_),
                ..
            }
        ));
        harness.run(commands);
        assert_eq!(harness.state.flow.save, SaveStatus::Saved(ConnectionId(3)));
        let saved = harness.state.connections.get(ConnectionId(3)).unwrap();
        assert_eq!(saved.settings.name, "Reports");
        assert!(saved.password_saved);
    }

    #[test]
    fn unchecking_save_password_forgets_it() {
        let mut harness = Harness::new();
        let mut form = valid_form();
        form.save_password = false;
        let commands = harness.state.save_connection(&form).unwrap();
        harness.run(commands);
        let saved = harness.state.connections.get(ConnectionId(3)).unwrap();
        assert!(!saved.password_saved);
    }

    #[test]
    fn editing_updates_the_same_connection() {
        let mut harness = Harness::new();
        let summary = harness
            .state
            .connections
            .get(ConnectionId(2))
            .unwrap()
            .clone();
        let mut form = ConnectionForm::from_summary(&summary);
        form.name = "Renamed".to_string();
        let commands = harness.state.save_connection(&form).unwrap();
        harness.run(commands);
        assert_eq!(harness.state.connections.name(ConnectionId(2)), "Renamed");
        assert_eq!(harness.state.connections.items().len(), 2);
    }

    #[test]
    fn connect_and_disconnect_follow_the_core() {
        let mut harness = Harness::new();
        let commands = harness.state.connect(ConnectionId(1));
        harness.run(commands);
        let state_of = |harness: &Harness| {
            harness
                .state
                .connections
                .get(ConnectionId(1))
                .unwrap()
                .state
        };
        assert_eq!(
            state_of(&harness),
            dbased_commands::ConnectionState::Connected
        );
        let commands = harness.state.disconnect(ConnectionId(1));
        harness.run(commands);
        assert_eq!(
            state_of(&harness),
            dbased_commands::ConnectionState::Disconnected
        );
    }

    fn forget_password(harness: &mut Harness, id: ConnectionId) {
        let summary = harness.state.connections.get(id).unwrap().clone();
        let mut form = ConnectionForm::from_summary(&summary);
        form.save_password = false;
        let commands = harness.state.save_connection(&form).unwrap();
        harness.run(commands);
    }

    #[test]
    fn connecting_without_a_saved_password_opens_the_prompt() {
        let mut harness = Harness::new();
        forget_password(&mut harness, ConnectionId(2));
        let commands = harness.state.connect(ConnectionId(2));
        harness.run(commands);
        let prompt = harness.state.flow.password_prompt.clone().unwrap();
        assert_eq!(prompt.connection, ConnectionId(2));
        assert!(prompt.resume.is_none());

        let commands = harness
            .state
            .submit_password(SecretString::from("typed".to_string()), false);
        assert!(matches!(
            &commands[0],
            Command::Connect {
                password: dbased_commands::PasswordSource::Typed(_),
                ..
            }
        ));
        assert_eq!(commands.len(), 1);
        harness.run(commands);
        assert!(harness.state.flow.password_prompt.is_none());
        let connected = harness
            .state
            .connections
            .get(ConnectionId(2))
            .unwrap()
            .state;
        assert_eq!(connected, dbased_commands::ConnectionState::Connected);
    }

    #[test]
    fn the_prompt_can_also_save_the_password() {
        let mut harness = Harness::new();
        forget_password(&mut harness, ConnectionId(2));
        let commands = harness.state.connect(ConnectionId(2));
        harness.run(commands);
        let commands = harness
            .state
            .submit_password(SecretString::from("typed".to_string()), true);
        assert!(matches!(
            &commands[1],
            Command::SaveConnection {
                id: Some(ConnectionId(2)),
                password: PasswordUpdate::Save(_),
                ..
            }
        ));
        harness.run(commands);
        assert!(
            harness
                .state
                .connections
                .get(ConnectionId(2))
                .unwrap()
                .password_saved
        );
    }

    #[test]
    fn cancelling_the_prompt_drops_it() {
        let mut harness = Harness::new();
        forget_password(&mut harness, ConnectionId(2));
        let commands = harness.state.connect(ConnectionId(2));
        harness.run(commands);
        harness.state.cancel_password();
        assert!(harness.state.flow.password_prompt.is_none());
    }

    #[test]
    fn an_execute_needing_a_password_prompts_then_replays() {
        let mut harness = Harness::new();
        let tab = harness.first_tab();
        let commands = harness
            .state
            .execute(tab, "select 3".to_string(), ExecuteTarget::All);
        let Command::Execute { request, .. } = commands.last().unwrap().clone() else {
            panic!("an execute");
        };
        // The fake never refuses an execute, so play the refusal by hand.
        let replies = harness.state.apply(Event::RequestFailed {
            request,
            error: password_required(),
        });
        assert!(replies.is_empty());
        assert_eq!(harness.state.tab_status(tab), Some(TabStatus::NotConnected));
        assert!(harness.state.can_run(tab));
        let prompt = harness.state.flow.password_prompt.clone().unwrap();
        assert_eq!(prompt.connection, ConnectionId(1));

        let commands = harness
            .state
            .submit_password(SecretString::from("pw".to_string()), false);
        harness.run(commands);
        let tab_model = harness.state.tabs.get(tab).unwrap();
        assert_eq!(
            tab_model.results.len(),
            1,
            "the script ran after connecting"
        );
    }

    #[test]
    fn the_delete_confirmation_counts_open_tabs() {
        let mut harness = Harness::new();
        let (_, commands) = harness.state.open_tab(ConnectionId(1)).unwrap();
        harness.run(commands);
        let confirmation = harness.state.delete_confirmation(ConnectionId(1)).unwrap();
        assert_eq!(confirmation.open_tabs, 2);
        assert!(confirmation.message().contains("2 open tabs"));
        let other = harness.state.delete_confirmation(ConnectionId(2)).unwrap();
        assert_eq!(other.open_tabs, 0);
        assert!(!other.message().contains("tab"));
    }

    #[test]
    fn deleting_a_connection_closes_its_tabs_first() {
        let mut harness = Harness::new();
        let (_, commands) = harness.state.open_tab(ConnectionId(2)).unwrap();
        harness.run(commands);
        let commands = harness.state.delete_connection(ConnectionId(1));
        assert!(matches!(commands[0], Command::CloseTab { .. }));
        assert!(matches!(
            commands.last().unwrap(),
            Command::DeleteConnection { .. }
        ));
        harness.run(commands);
        assert!(harness.state.connections.get(ConnectionId(1)).is_none());
        assert!(
            harness
                .state
                .tabs
                .iter()
                .all(|tab| tab.connection == ConnectionId(2))
        );
        assert_eq!(harness.state.tabs.len(), 1);
    }
}
