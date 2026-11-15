//! Interaction tests (docs/PROJECT.md 7.1, layer 2): the real UI over `FakeCore`, driven
//! with simulated keystrokes, no display and no GPU.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use dbased_commands::{COMMAND_CAPACITY, EVENT_CAPACITY, ResultId, TabId, bridge, fake::FakeCore};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use gpui_component::{Root, table::TableDelegate as _};

use crate::{editor_panel::RunMode, init, state::TabStatus, workspace::Workspace};

/// Opens a window on a fresh `FakeCore` and lets the first events through.
pub(crate) fn launch(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(init);
    let (ui_link, core_link) = bridge(COMMAND_CAPACITY, EVENT_CAPACITY);
    cx.executor()
        .spawn(FakeCore::new().serve(core_link))
        .detach();
    let (root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(ui_link, window, cx));
        Root::new(workspace, window, cx)
    });
    cx.run_until_parked();
    let workspace = root.read_with(cx, |root, _| root.view().clone());
    let workspace = workspace
        .downcast::<Workspace>()
        .unwrap_or_else(|_| panic!("the root view is the workspace"));
    (workspace, cx)
}

fn first_tab(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> TabId {
    workspace.read_with(cx, |workspace, cx| {
        workspace
            .state()
            .read(cx)
            .tabs
            .iter()
            .next()
            .map(|tab| tab.id)
            .expect("a tab is open")
    })
}

fn type_script(workspace: &Entity<Workspace>, script: &str, cx: &mut VisualTestContext) {
    let editor = workspace
        .read_with(cx, |workspace, cx| workspace.active_editor(cx))
        .expect("an editor");
    editor.update_in(cx, |editor, window, cx| {
        editor.set_text(script, window, cx);
        editor.focus(window, cx);
    });
    cx.run_until_parked();
}

fn place_cursor(workspace: &Entity<Workspace>, offset: usize, cx: &mut VisualTestContext) {
    let editor = workspace
        .read_with(cx, |workspace, cx| workspace.active_editor(cx))
        .expect("an editor");
    let state = editor.read_with(cx, |editor, _| editor.editor().clone());
    state.update(cx, |state, cx| state.set_selected_range(offset..offset, cx));
}

fn result_ids(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<ResultId> {
    workspace.read_with(cx, |workspace, cx| {
        workspace
            .state()
            .read(cx)
            .tabs
            .iter()
            .flat_map(|tab| tab.results.clone())
            .collect()
    })
}

#[gpui::test]
fn the_first_tab_opens_with_its_editor_focused(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let editor = workspace
        .read_with(cx, |workspace, cx| workspace.active_editor(cx))
        .expect("the first tab has an editor");
    let focused = editor.update_in(cx, |editor, window, cx| editor.is_focused(window, cx));
    assert!(focused, "the editor must take focus when its tab opens");
    let status = workspace.read_with(cx, |workspace, cx| {
        let state = workspace.state().read(cx);
        state.tab_status(state.tabs.active_id().expect("active tab"))
    });
    assert_eq!(status, Some(TabStatus::NotConnected));
}

#[gpui::test]
fn the_explorer_lists_the_connections(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let labels = workspace.read_with(cx, |workspace, cx| {
        workspace.explorer().read(cx).visible_labels(cx)
    });
    assert_eq!(labels, vec!["Local Postgres", "Local MySQL"]);
}

#[gpui::test]
fn expanding_a_connection_loads_its_schemas_lazily(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let tree = workspace.read_with(cx, |workspace, cx| {
        workspace.explorer().read(cx).tree().clone()
    });
    tree.update_in(cx, |tree, window, cx| {
        tree.set_selected_index(Some(0), cx);
        tree.focus(window, cx);
    });
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    let labels = workspace.read_with(cx, |workspace, cx| {
        workspace.explorer().read(cx).visible_labels(cx)
    });
    assert_eq!(
        labels,
        vec!["Local Postgres", "public", "sales", "Local MySQL"]
    );
    let state = workspace.read_with(cx, |workspace, cx| {
        let state = workspace.state().read(cx);
        state
            .connections
            .get(dbased_commands::ConnectionId(1))
            .map(|connection| connection.state)
    });
    assert_eq!(state, Some(dbased_commands::ConnectionState::Connected));
}

#[gpui::test]
fn ctrl_enter_runs_the_script_and_fills_the_grid(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    type_script(&workspace, "select 1000", cx);
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();

    let results = result_ids(&workspace, cx);
    assert_eq!(results.len(), 1);
    let panel = workspace
        .read_with(cx, |workspace, cx| workspace.result_panel(results[0], cx))
        .expect("a result panel");
    let (total, windows) = panel.read_with(cx, |panel, cx| {
        let table = panel.table().expect("a grid").read(cx);
        (
            table.delegate().total_rows(),
            table.delegate().cached_windows(),
        )
    });
    assert_eq!(total, 1_000);
    assert!(windows >= 1, "the grid must have pulled its first window");

    let tab = first_tab(&workspace, cx);
    let status = workspace.read_with(cx, |workspace, cx| {
        workspace.state().read(cx).tab_status(tab)
    });
    assert_eq!(status, Some(TabStatus::Connected));
}

#[gpui::test]
fn the_results_area_starts_with_a_placeholder_until_a_result_arrives(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    assert!(workspace.read_with(cx, |workspace, _| workspace.placeholder_shown()));
    type_script(&workspace, "select 2", cx);
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    assert!(!workspace.read_with(cx, |workspace, _| workspace.placeholder_shown()));
}

#[gpui::test]
fn the_grid_shows_the_rows_the_core_sent(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    type_script(&workspace, "select 50", cx);
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    let results = result_ids(&workspace, cx);
    let panel = workspace
        .read_with(cx, |workspace, cx| workspace.result_panel(results[0], cx))
        .expect("a result panel");
    let (label, null, loaded) = panel.read_with(cx, |panel, cx| {
        let table = panel.table().expect("a grid").read(cx);
        let delegate = table.delegate();
        (
            delegate.cell_text(3, 4, cx),
            delegate.cell_text(5, 5, cx),
            delegate.cell_text(49, 0, cx),
        )
    });
    assert_eq!(label, "row 3");
    assert_eq!(null, "NULL");
    assert_eq!(loaded, "49");
}

#[gpui::test]
fn a_selection_is_what_run_current_executes(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    type_script(&workspace, "select 2; select 9", cx);
    let editor = workspace
        .read_with(cx, |workspace, cx| workspace.active_editor(cx))
        .expect("an editor");
    let state = editor.read_with(cx, |editor, _| editor.editor().clone());
    state.update(cx, |state, cx| state.set_selected_range(0..8, cx));
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    let results = result_ids(&workspace, cx);
    assert_eq!(results.len(), 1);
    let rows = workspace.read_with(cx, |workspace, cx| {
        workspace
            .state()
            .read(cx)
            .result(results[0])
            .map(|result| result.row_count())
    });
    assert_eq!(rows, Some(2));
}

#[gpui::test]
fn without_a_core_running_says_so(cx: &mut TestAppContext) {
    cx.update(init);
    let (ui_link, core_link) = bridge(COMMAND_CAPACITY, EVENT_CAPACITY);
    drop(core_link);
    let (root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(ui_link, window, cx));
        Root::new(workspace, window, cx)
    });
    cx.run_until_parked();
    let workspace = root
        .read_with(cx, |root, _| root.view().clone())
        .downcast::<Workspace>()
        .unwrap_or_else(|_| panic!("the root view is the workspace"));
    let (stopped, notice, tabs) = workspace.read_with(cx, |workspace, cx| {
        let state = workspace.state().read(cx);
        (state.core_stopped, state.notice.clone(), state.tabs.len())
    });
    assert!(stopped);
    assert_eq!(notice.as_deref(), Some("The Core stopped."));
    assert_eq!(tabs, 0);
}

#[gpui::test]
fn ctrl_shift_enter_runs_every_statement(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    type_script(&workspace, "select 2; select 3", cx);
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    assert_eq!(result_ids(&workspace, cx).len(), 2);
}

#[gpui::test]
fn run_current_picks_the_statement_under_the_cursor(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    type_script(&workspace, "select 2; select 7", cx);
    place_cursor(&workspace, 15, cx);
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    let results = result_ids(&workspace, cx);
    assert_eq!(results.len(), 1);
    let rows = workspace.read_with(cx, |workspace, cx| {
        workspace
            .state()
            .read(cx)
            .result(results[0])
            .map(|result| result.row_count())
    });
    assert_eq!(rows, Some(7));
}

#[gpui::test]
fn escape_cancels_a_running_statement(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    type_script(&workspace, "sleep", cx);
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    let tab = first_tab(&workspace, cx);
    let running = workspace.read_with(cx, |workspace, cx| {
        workspace.state().read(cx).tab_status(tab)
    });
    assert_eq!(running, Some(TabStatus::Running));

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let after = workspace.read_with(cx, |workspace, cx| {
        workspace.state().read(cx).tab_status(tab)
    });
    assert_eq!(after, Some(TabStatus::Connected));
    let cancelled = workspace.read_with(cx, |workspace, cx| {
        let state = workspace.state().read(cx);
        let id = state.tabs.get(tab).expect("tab").results[0];
        state.result(id).map(|result| result.status_line())
    });
    assert_eq!(cancelled.as_deref(), Some("Cancelled after 0 rows"));
}

#[gpui::test]
fn escape_without_a_running_statement_does_not_cancel(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    type_script(&workspace, "select 1", cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let notice = workspace.read_with(cx, |workspace, cx| {
        workspace.state().read(cx).notice.clone()
    });
    assert!(notice.is_none());
}

#[gpui::test]
fn a_failed_statement_shows_its_error_without_a_grid(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    type_script(&workspace, "fail now", cx);
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    let results = result_ids(&workspace, cx);
    let panel = workspace
        .read_with(cx, |workspace, cx| workspace.result_panel(results[0], cx))
        .expect("a result panel");
    let has_grid = panel.read_with(cx, |panel, _| panel.table().is_some());
    assert!(!has_grid);
    let line = workspace.read_with(cx, |workspace, cx| {
        workspace
            .state()
            .read(cx)
            .result(results[0])
            .map(|result| result.status_line())
    });
    assert_eq!(
        line.as_deref(),
        Some("Failed: syntax error (SQLSTATE 42601, position 6)")
    );
}

#[gpui::test]
fn new_tab_opens_an_editor_with_focus(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    cx.simulate_keystrokes("ctrl-t");
    cx.run_until_parked();
    let tabs = workspace.read_with(cx, |workspace, cx| workspace.state().read(cx).tabs.len());
    assert_eq!(tabs, 2);
    let editor = workspace
        .read_with(cx, |workspace, cx| workspace.active_editor(cx))
        .expect("an editor");
    let focused = editor.update_in(cx, |editor, window, cx| editor.is_focused(window, cx));
    assert!(focused);
    let title = editor.read_with(cx, |editor, cx| {
        crate::editor_panel::tab_title_for_tests(editor, cx)
    });
    assert_eq!(title.as_deref(), Some("Query 2"));
}

#[gpui::test]
fn picking_another_connection_rebinds_the_tab(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    let old = first_tab(&workspace, cx);
    let editor = workspace
        .read_with(cx, |workspace, cx| workspace.active_editor(cx))
        .expect("an editor");
    editor.update(cx, |_, cx| {
        cx.emit(crate::editor_panel::EditorPanelEvent::ConnectionPicked(
            dbased_commands::ConnectionId(2),
        ));
    });
    cx.run_until_parked();
    let (fresh, connection) = workspace.read_with(cx, |workspace, cx| {
        let state = workspace.state().read(cx);
        let tab = state.tabs.active().expect("tab");
        (tab.id, tab.connection)
    });
    assert_ne!(fresh, old);
    assert_eq!(connection, dbased_commands::ConnectionId(2));
    let panel_tab = editor.read_with(cx, |editor, _| editor.tab());
    assert_eq!(panel_tab, fresh);
}

#[gpui::test]
fn running_again_replaces_the_result_panels(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    type_script(&workspace, "select 2; select 3", cx);
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    let first = result_ids(&workspace, cx);
    type_script(&workspace, "select 5", cx);
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    let second = result_ids(&workspace, cx);
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 1);
    let panels = workspace.read_with(cx, |workspace, cx| {
        first
            .iter()
            .filter(|id| workspace.result_panel(**id, cx).is_some())
            .count()
    });
    assert_eq!(panels, 0, "panels of the previous run are gone");
}

#[gpui::test]
fn the_toolbar_run_matches_the_shortcut(cx: &mut TestAppContext) {
    let (workspace, cx) = launch(cx);
    type_script(&workspace, "select 4", cx);
    workspace.update(cx, |workspace, cx| workspace.run(RunMode::All, cx));
    cx.run_until_parked();
    assert_eq!(result_ids(&workspace, cx).len(), 1);
}
