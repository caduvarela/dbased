//! Interaction tests for connection management (docs/PROJECT.md 7.1, layer 2): the
//! dialogs and the explorer toolbar over `FakeCore`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use dbased_commands::{ConnectionId, ConnectionState, DatabaseKind, TlsMode};
use gpui::{Entity, TestAppContext, VisualTestContext};
use gpui_component::WindowExt as _;

use crate::{
    connection_dialog::{ConnectionDialog, Field},
    explorer_panel::ExplorerEvent,
    state::{SaveStatus, TestStatus},
    tests::launch,
    workspace::Workspace,
};

fn open_dialog(
    workspace: &Entity<Workspace>,
    editing: Option<ConnectionId>,
    cx: &mut VisualTestContext,
) -> Entity<ConnectionDialog> {
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_connection_dialog(editing, window, cx);
    });
    cx.run_until_parked();
    workspace
        .read_with(cx, |workspace, _| workspace.connection_dialog())
        .expect("the dialog is open")
}

fn set_text(
    dialog: &Entity<ConnectionDialog>,
    field: Field,
    text: &str,
    cx: &mut VisualTestContext,
) {
    let input = dialog.read_with(cx, |dialog, _| dialog.input(field).clone());
    input.update_in(cx, |input, window, cx| input.set_value(text, window, cx));
}

fn fill_required(dialog: &Entity<ConnectionDialog>, cx: &mut VisualTestContext) {
    set_text(dialog, Field::Name, "Reports", cx);
    set_text(dialog, Field::User, "reader", cx);
    set_text(dialog, Field::Password, "s3cret", cx);
}

fn dialog_open(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| window.has_active_dialog(cx))
}

fn connection_count(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> usize {
    workspace.read_with(cx, |workspace, cx| {
        workspace.state().read(cx).connections.items().len()
    })
}

#[gpui::test]
fn the_new_connection_button_opens_the_dialog_with_safe_defaults(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let explorer = workspace.read_with(cx, |workspace, _| workspace.explorer().clone());
    explorer.update(cx, |_, cx| cx.emit(ExplorerEvent::NewConnection));
    cx.run_until_parked();
    assert!(dialog_open(cx));
    let dialog = workspace
        .read_with(cx, |workspace, _| workspace.connection_dialog())
        .expect("a connection dialog");
    let form = dialog.read_with(cx, |dialog, cx| dialog.collect(cx));
    assert_eq!(form.kind, DatabaseKind::Postgres);
    assert_eq!(form.port, "5432");
    assert_eq!(form.tls, TlsMode::Require);
    assert!(form.save_password, "Save password starts checked");
    assert!(form.editing().is_none());
}

#[gpui::test]
fn the_toolbar_follows_the_selected_connection(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let explorer = workspace.read_with(cx, |workspace, _| workspace.explorer().clone());
    assert_eq!(
        explorer.read_with(cx, |explorer, cx| explorer.selected_connection(cx)),
        None
    );
    let tree = explorer.read_with(cx, |explorer, _| explorer.tree().clone());
    tree.update(cx, |tree, cx| tree.set_selected_index(Some(1), cx));
    assert_eq!(
        explorer.read_with(cx, |explorer, cx| explorer.selected_connection(cx)),
        Some(ConnectionId(2))
    );
}

#[gpui::test]
fn changing_the_driver_moves_the_default_port(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let dialog = open_dialog(&workspace, None, cx);
    dialog.update_in(cx, |dialog, window, cx| {
        dialog.set_kind(DatabaseKind::MySql, window, cx);
    });
    let form = dialog.read_with(cx, |dialog, cx| dialog.collect(cx));
    assert_eq!(form.kind, DatabaseKind::MySql);
    assert_eq!(form.port, "3306");
}

#[gpui::test]
fn a_port_the_user_typed_survives_a_driver_change(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let dialog = open_dialog(&workspace, None, cx);
    set_text(&dialog, Field::Port, "6000", cx);
    set_text(&dialog, Field::Database, "shop", cx);
    dialog.update_in(cx, |dialog, window, cx| {
        dialog.set_kind(DatabaseKind::MySql, window, cx);
    });
    let form = dialog.read_with(cx, |dialog, cx| dialog.collect(cx));
    assert_eq!(form.port, "6000");
    assert_eq!(form.database, "shop");
}

#[gpui::test]
fn an_invalid_form_shows_errors_and_sends_nothing(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let dialog = open_dialog(&workspace, None, cx);
    dialog.update(cx, |dialog, cx| dialog.test(cx));
    cx.run_until_parked();
    let errors = dialog.read_with(cx, |dialog, _| dialog.errors().cloned());
    assert!(errors.expect("errors shown").name.is_some());
    let test = workspace.read_with(cx, |workspace, cx| {
        workspace.state().read(cx).flow.test.clone()
    });
    assert_eq!(test, TestStatus::Idle);
}

#[gpui::test]
fn testing_the_connection_reports_success_from_the_core(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let dialog = open_dialog(&workspace, None, cx);
    fill_required(&dialog, cx);
    dialog.update(cx, |dialog, cx| dialog.test(cx));
    cx.run_until_parked();
    let test = workspace.read_with(cx, |workspace, cx| {
        workspace.state().read(cx).flow.test.clone()
    });
    assert_eq!(test, TestStatus::Success);
    assert!(dialog_open(cx), "testing does not close the dialog");

    let host = dialog.read_with(cx, |dialog, _| dialog.input(Field::Host).clone());
    host.update_in(cx, |input, window, cx| input.focus(window, cx));
    cx.simulate_input("x");
    cx.run_until_parked();
    let test = workspace.read_with(cx, |workspace, cx| {
        workspace.state().read(cx).flow.test.clone()
    });
    assert_eq!(test, TestStatus::Idle, "an edit makes the result stale");
}

#[gpui::test]
fn enter_saves_the_connection_and_closes_the_dialog(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let dialog = open_dialog(&workspace, None, cx);
    fill_required(&dialog, cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(connection_count(&workspace, cx), 3);
    let saved = workspace.read_with(cx, |workspace, cx| {
        workspace
            .state()
            .read(cx)
            .connections
            .get(ConnectionId(3))
            .cloned()
    });
    let saved = saved.expect("the new connection");
    assert_eq!(saved.settings.name, "Reports");
    assert_eq!(saved.settings.tls, TlsMode::Require);
    assert!(saved.password_saved);
    assert!(!dialog_open(cx), "the dialog closes after the save");
}

#[gpui::test]
fn unchecking_save_password_saves_without_it(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let dialog = open_dialog(&workspace, None, cx);
    fill_required(&dialog, cx);
    dialog.update(cx, |dialog, cx| dialog.set_save_password(false, cx));
    dialog.update_in(cx, |dialog, window, cx| dialog.submit(window, cx));
    cx.run_until_parked();
    let saved = workspace.read_with(cx, |workspace, cx| {
        workspace
            .state()
            .read(cx)
            .connections
            .get(ConnectionId(3))
            .cloned()
    });
    assert!(!saved.expect("saved").password_saved);
}

#[gpui::test]
fn escape_cancels_without_saving(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let dialog = open_dialog(&workspace, None, cx);
    fill_required(&dialog, cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!dialog_open(cx));
    assert_eq!(connection_count(&workspace, cx), 2);
    let save = workspace.read_with(cx, |workspace, cx| {
        workspace.state().read(cx).flow.save.clone()
    });
    assert_eq!(save, SaveStatus::Idle);
}

#[gpui::test]
fn editing_starts_from_the_saved_settings_and_updates_in_place(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let dialog = open_dialog(&workspace, Some(ConnectionId(2)), cx);
    let form = dialog.read_with(cx, |dialog, cx| dialog.collect(cx));
    assert_eq!(form.name, "Local MySQL");
    assert_eq!(form.kind, DatabaseKind::MySql);
    assert_eq!(form.port, "3306");
    assert_eq!(form.editing(), Some(ConnectionId(2)));
    set_text(&dialog, Field::Name, "Renamed", cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(connection_count(&workspace, cx), 2);
    let name = workspace.read_with(cx, |workspace, cx| {
        workspace.state().read(cx).connections.name(ConnectionId(2))
    });
    assert_eq!(name, "Renamed");
}

#[gpui::test]
fn the_password_never_shows_in_debug_output(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let dialog = open_dialog(&workspace, None, cx);
    fill_required(&dialog, cx);
    let form = dialog.read_with(cx, |dialog, cx| dialog.collect(cx));
    let printed = format!("{form:?} {:?}", form.password_update());
    assert!(!printed.contains("s3cret"));
}

#[gpui::test]
fn connect_and_disconnect_from_the_toolbar_events(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let explorer = workspace.read_with(cx, |workspace, _| workspace.explorer().clone());
    let state_of = |workspace: &Entity<Workspace>, cx: &mut VisualTestContext| {
        workspace.read_with(cx, |workspace, cx| {
            workspace
                .state()
                .read(cx)
                .connections
                .get(ConnectionId(2))
                .map(|connection| connection.state)
        })
    };
    explorer.update(cx, |_, cx| cx.emit(ExplorerEvent::Connect(ConnectionId(2))));
    cx.run_until_parked();
    assert_eq!(state_of(&workspace, cx), Some(ConnectionState::Connected));
    explorer.update(cx, |_, cx| {
        cx.emit(ExplorerEvent::Disconnect(ConnectionId(2)));
    });
    cx.run_until_parked();
    assert_eq!(
        state_of(&workspace, cx),
        Some(ConnectionState::Disconnected)
    );
}

#[gpui::test]
fn deleting_asks_first_then_closes_the_open_tabs(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    // The first tab is bound to connection 1.
    let explorer = workspace.read_with(cx, |workspace, _| workspace.explorer().clone());
    explorer.update(cx, |_, cx| cx.emit(ExplorerEvent::Delete(ConnectionId(1))));
    cx.run_until_parked();
    assert!(dialog_open(cx), "a confirmation appears");
    assert_eq!(
        connection_count(&workspace, cx),
        2,
        "nothing is deleted yet"
    );
    let tabs = workspace.read_with(cx, |workspace, cx| workspace.state().read(cx).tabs.len());
    assert_eq!(tabs, 1);

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(connection_count(&workspace, cx), 1);
    let tabs = workspace.read_with(cx, |workspace, cx| workspace.state().read(cx).tabs.len());
    assert_eq!(tabs, 0, "its tabs closed with it");
    let editor = workspace.read_with(cx, |workspace, cx| workspace.active_editor(cx));
    assert!(editor.is_none());
}

#[gpui::test]
fn cancelling_the_delete_keeps_everything(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.confirm_delete(ConnectionId(1), window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!dialog_open(cx));
    assert_eq!(connection_count(&workspace, cx), 2);
    let tabs = workspace.read_with(cx, |workspace, cx| workspace.state().read(cx).tabs.len());
    assert_eq!(tabs, 1);
}

/// Saves connection 2 without its password, so the next connect needs a prompt.
fn forget_password_of_second(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    let dialog = open_dialog(workspace, Some(ConnectionId(2)), cx);
    dialog.update(cx, |dialog, cx| dialog.set_save_password(false, cx));
    dialog.update_in(cx, |dialog, window, cx| dialog.submit(window, cx));
    cx.run_until_parked();
    assert!(!dialog_open(cx));
}

#[gpui::test]
fn connecting_without_a_saved_password_prompts_for_it(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    forget_password_of_second(&workspace, cx);
    workspace.update(cx, |workspace, cx| workspace.connect(ConnectionId(2), cx));
    cx.run_until_parked();
    assert!(dialog_open(cx), "the password prompt is open");
    let prompt = workspace
        .read_with(cx, |workspace, _| workspace.password_dialog())
        .expect("a password dialog");

    let input = prompt.read_with(cx, |prompt, _| prompt.input().clone());
    input.update_in(cx, |input, window, cx| input.set_value("typed", window, cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!dialog_open(cx));
    let state = workspace.read_with(cx, |workspace, cx| {
        workspace
            .state()
            .read(cx)
            .connections
            .get(ConnectionId(2))
            .map(|connection| (connection.state, connection.password_saved))
    });
    assert_eq!(state, Some((ConnectionState::Connected, false)));
}

#[gpui::test]
fn the_prompt_can_save_the_password(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    forget_password_of_second(&workspace, cx);
    workspace.update(cx, |workspace, cx| workspace.connect(ConnectionId(2), cx));
    cx.run_until_parked();
    let prompt = workspace
        .read_with(cx, |workspace, _| workspace.password_dialog())
        .expect("a password dialog");
    prompt.update(cx, |prompt, cx| prompt.set_save(true, cx));
    prompt.update(cx, |prompt, cx| prompt.submit(cx));
    cx.run_until_parked();
    let saved = workspace.read_with(cx, |workspace, cx| {
        workspace
            .state()
            .read(cx)
            .connections
            .get(ConnectionId(2))
            .map(|connection| connection.password_saved)
    });
    assert_eq!(saved, Some(true));
}

#[gpui::test]
fn escape_dismisses_the_prompt_without_connecting(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    forget_password_of_second(&workspace, cx);
    workspace.update(cx, |workspace, cx| workspace.connect(ConnectionId(2), cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!dialog_open(cx));
    let (prompting, state) = workspace.read_with(cx, |workspace, cx| {
        let state = workspace.state().read(cx);
        (
            state.flow.password_prompt.is_some(),
            state.connections.get(ConnectionId(2)).map(|c| c.state),
        )
    });
    assert!(!prompting);
    assert_eq!(state, Some(ConnectionState::Disconnected));
}
