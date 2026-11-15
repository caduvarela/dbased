use std::rc::Rc;

use dbased_commands::{Command, ConnectionId, Event, ResultId, TabId, UiLink};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    dock::{DockArea, DockPlacement, DockSkin, PanelStyle, panel_handle},
    h_flex, v_flex,
};

use crate::{
    actions::{
        CancelExecution, CloseActiveTab, NewTab, RUNNING_CONTEXT, RunAll, RunCurrent,
        WORKSPACE_CONTEXT,
    },
    client::CommandSink,
    editor_panel::{EditorPanel, EditorPanelEvent, RunMode},
    explorer_panel::ExplorerPanel,
    results_panel::{ResultPanel, ResultsPlaceholder},
    state::UiState,
};

const EXPLORER_WIDTH: f32 = 260.0;
const RESULTS_HEIGHT: f32 = 300.0;

/// The main window's view: toolbar, dock (explorer, editor tabs, result tabs) and status bar.
pub struct Workspace {
    state: Entity<UiState>,
    sink: CommandSink,
    dock: Entity<DockArea>,
    _dock_skin: Rc<DockSkin>,
    #[cfg(test)]
    explorer: Entity<ExplorerPanel>,
    placeholder: Entity<ResultsPlaceholder>,
    placeholder_shown: bool,
    editors: Vec<Entity<EditorPanel>>,
    results: Vec<Entity<ResultPanel>>,
    focus_handle: FocusHandle,
    subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(link: UiLink, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let UiLink { commands, events } = link;
        let sink = CommandSink::new(commands);
        let state = cx.new(|_| UiState::new());
        let (dock, skin) = DockSkin::dock_area("workspace", Some(1), window, cx);
        skin.set_panel_style(PanelStyle::TabBar, cx);
        skin.set_close_button_visible(true, cx);
        let explorer = cx.new(|cx| ExplorerPanel::new(state.clone(), sink.clone(), cx));
        dock.update(cx, |dock, cx| {
            dock.add_panel_view(
                panel_handle(explorer.clone()),
                DockPlacement::Left,
                Some(px(EXPLORER_WIDTH)),
                window,
                cx,
            );
        });
        let placeholder = cx.new(ResultsPlaceholder::new);
        dock.update(cx, |dock, cx| {
            dock.add_panel_view(
                panel_handle(placeholder.clone()),
                DockPlacement::Bottom,
                Some(px(RESULTS_HEIGHT)),
                window,
                cx,
            );
        });
        let observe_state = cx.observe_in(&state, window, |workspace, _, window, cx| {
            workspace.sync_panels(window, cx);
            cx.notify();
        });
        let workspace = Self {
            state,
            sink,
            dock,
            _dock_skin: skin,
            #[cfg(test)]
            explorer,
            placeholder,
            placeholder_shown: true,
            editors: Vec::new(),
            results: Vec::new(),
            focus_handle: cx.focus_handle(),
            subscriptions: vec![observe_state],
        };
        workspace.pump_events(events, window, cx);
        workspace
    }

    #[cfg(test)]
    pub fn state(&self) -> &Entity<UiState> {
        &self.state
    }

    #[cfg(test)]
    pub fn explorer(&self) -> &Entity<ExplorerPanel> {
        &self.explorer
    }

    #[cfg(test)]
    pub fn placeholder_shown(&self) -> bool {
        self.placeholder_shown
    }

    pub fn editor_for(&self, tab: TabId, cx: &App) -> Option<Entity<EditorPanel>> {
        self.editors
            .iter()
            .find(|editor| editor.read(cx).tab() == tab)
            .cloned()
    }

    pub fn result_panel(&self, result: ResultId, cx: &App) -> Option<Entity<ResultPanel>> {
        self.results
            .iter()
            .find(|panel| panel.read(cx).result() == result)
            .cloned()
    }

    pub fn active_editor(&self, cx: &App) -> Option<Entity<EditorPanel>> {
        let tab = self.state.read(cx).tabs.active_id()?;
        self.editor_for(tab, cx)
    }

    /// Drains the Core's events on GPUI's executor: wait for one, take whatever else is
    /// ready, apply the batch in one update (docs/PROJECT.md 11.3).
    fn pump_events(
        &self,
        events: async_channel::Receiver<Event>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let Ok(first) = events.recv().await else {
                    this.update(cx, |workspace, cx| workspace.core_stopped(cx))
                        .ok();
                    return;
                };
                let mut batch = vec![first];
                while let Ok(event) = events.try_recv() {
                    batch.push(event);
                }
                let applied = this.update_in(cx, |workspace, window, cx| {
                    workspace.apply_events(batch, window, cx);
                });
                if applied.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn core_stopped(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.core_stopped();
            cx.notify();
        });
    }

    pub fn apply_events(
        &mut self,
        events: Vec<Event>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut commands = Vec::new();
        for event in events {
            if let Event::RowsWindow {
                result,
                start,
                rows,
            } = event
            {
                self.route_rows(result, start, rows, cx);
                continue;
            }
            let produced = self.state.update(cx, |state, cx| {
                cx.notify();
                state.apply(event)
            });
            commands.extend(produced);
        }
        self.sync_panels(window, cx);
        self.dispatch(commands, cx);
        cx.notify();
    }

    fn route_rows(
        &mut self,
        result: ResultId,
        start: u64,
        rows: Vec<Vec<dbased_commands::Value>>,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.result_panel(result, cx) else {
            return;
        };
        panel.update(cx, |panel, cx| panel.apply_window(start, rows, cx));
    }

    /// Sends commands; a full or closed channel becomes a notice, never a freeze.
    fn dispatch(&mut self, commands: Vec<Command>, cx: &mut Context<Self>) {
        for command in commands {
            if let Err(failure) = self.sink.send(command) {
                self.state.update(cx, |state, cx| {
                    state.send_failed(failure);
                    cx.notify();
                });
                return;
            }
        }
    }

    /// Makes the panels match the state: one editor panel per tab, one result panel per
    /// result.
    fn sync_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editors(window, cx);
        self.sync_results(window, cx);
    }

    fn sync_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open: Vec<TabId> = self.state.read(cx).tabs.iter().map(|tab| tab.id).collect();
        let stale: Vec<Entity<EditorPanel>> = self
            .editors
            .iter()
            .filter(|panel| !open.contains(&panel.read(cx).tab()))
            .cloned()
            .collect();
        for panel in stale {
            self.editors.retain(|known| known != &panel);
            self.dock
                .update(cx, |dock, cx| dock.remove_panel(panel, window, cx));
        }
        for tab in open {
            if self.editor_for(tab, cx).is_some() {
                continue;
            }
            self.add_editor(tab, window, cx);
        }
    }

    fn add_editor(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.clone();
        let sink = self.sink.clone();
        let panel = cx.new(|cx| EditorPanel::new(tab, state, sink, window, cx));
        let subscription = cx.subscribe_in(
            &panel,
            window,
            |workspace, panel, event: &EditorPanelEvent, window, cx| match event {
                EditorPanelEvent::ConnectionPicked(connection) => {
                    workspace.rebind_editor(panel.clone(), *connection, window, cx);
                }
            },
        );
        self.subscriptions.push(subscription);
        self.editors.push(panel.clone());
        self.dock.update(cx, |dock, cx| {
            dock.add_panel_view(
                panel_handle(panel.clone()),
                DockPlacement::Center,
                None,
                window,
                cx,
            );
        });
        panel.update(cx, |panel, cx| panel.focus(window, cx));
    }

    fn rebind_editor(
        &mut self,
        panel: Entity<EditorPanel>,
        connection: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = panel.read(cx).tab();
        let rebound = self.state.update(cx, |state, cx| {
            cx.notify();
            state.rebind_tab(tab, connection)
        });
        let Some((fresh, commands)) = rebound else {
            return;
        };
        panel.update(cx, |panel, cx| {
            panel.set_tab(fresh);
            cx.notify();
        });
        self.sync_panels(window, cx);
        self.dispatch(commands, cx);
        panel.update(cx, |panel, cx| panel.focus(window, cx));
    }

    fn sync_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted: Vec<ResultId> = {
            let state = self.state.read(cx);
            state
                .tabs
                .iter()
                .flat_map(|tab| tab.results.iter().copied())
                .collect()
        };
        let stale: Vec<Entity<ResultPanel>> = self
            .results
            .iter()
            .filter(|panel| !wanted.contains(&panel.read(cx).result()))
            .cloned()
            .collect();
        for panel in stale {
            self.results.retain(|known| known != &panel);
            self.dock
                .update(cx, |dock, cx| dock.remove_panel(panel, window, cx));
        }
        if !wanted.is_empty() {
            self.set_placeholder(false, window, cx);
        }
        for result in wanted {
            if self.result_panel(result, cx).is_some() {
                continue;
            }
            let state = self.state.clone();
            let sink = self.sink.clone();
            let panel = cx.new(|cx| ResultPanel::new(result, state, sink, window, cx));
            self.results.push(panel.clone());
            self.dock.update(cx, |dock, cx| {
                dock.add_panel_view(
                    panel_handle(panel),
                    DockPlacement::Bottom,
                    Some(px(RESULTS_HEIGHT)),
                    window,
                    cx,
                );
            });
        }
        if self.results.is_empty() {
            self.set_placeholder(true, window, cx);
        }
    }

    fn set_placeholder(&mut self, shown: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.placeholder_shown == shown {
            return;
        }
        self.placeholder_shown = shown;
        let placeholder = self.placeholder.clone();
        self.dock.update(cx, |dock, cx| {
            if shown {
                dock.add_panel_view(
                    panel_handle(placeholder),
                    DockPlacement::Bottom,
                    Some(px(RESULTS_HEIGHT)),
                    window,
                    cx,
                );
            } else {
                dock.remove_panel(placeholder, window, cx);
            }
        });
    }

    // User intents.

    pub fn run(&mut self, mode: RunMode, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor(cx) else {
            return;
        };
        let (tab, script, target) = {
            let editor = editor.read(cx);
            let (script, target) = editor.run_request(mode, cx);
            (editor.tab(), script, target)
        };
        let commands = self.state.update(cx, |state, cx| {
            cx.notify();
            state.execute(tab, script, target)
        });
        self.dispatch(commands, cx);
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.state.read(cx).tabs.active_id() else {
            return;
        };
        let commands = self.state.update(cx, |state, cx| {
            cx.notify();
            state.cancel(tab)
        });
        self.dispatch(commands, cx);
    }

    pub fn new_tab(&mut self, cx: &mut Context<Self>) {
        let opened = self.state.update(cx, |state, cx| {
            cx.notify();
            let connection = state.default_connection()?;
            state.open_tab(connection)
        });
        if let Some((_, commands)) = opened {
            self.dispatch(commands, cx);
        }
    }

    pub fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.state.read(cx).tabs.active_id() else {
            return;
        };
        let commands = self.state.update(cx, |state, cx| {
            cx.notify();
            state.close_tab(tab)
        });
        self.sync_panels(window, cx);
        self.dispatch(commands, cx);
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let active = state.tabs.active_id();
        let can_run = active.is_some_and(|tab| state.can_run(tab));
        let can_cancel = active.is_some_and(|tab| state.can_cancel(tab));
        let can_open = state.default_connection().is_some();
        h_flex()
            .h(px(38.0))
            .px_2()
            .gap_1()
            .flex_shrink_0()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().title_bar)
            .child(
                Button::new("run-current")
                    .label("Run")
                    .small()
                    .primary()
                    .disabled(!can_run)
                    .tooltip("Run the selection or the current statement (Ctrl+Enter)")
                    .on_click(
                        cx.listener(|workspace, _, _, cx| workspace.run(RunMode::Current, cx)),
                    ),
            )
            .child(
                Button::new("run-all")
                    .label("Run all")
                    .small()
                    .outline()
                    .disabled(!can_run)
                    .tooltip("Run the whole script (Ctrl+Shift+Enter)")
                    .on_click(cx.listener(|workspace, _, _, cx| workspace.run(RunMode::All, cx))),
            )
            .child(
                Button::new("cancel")
                    .label("Cancel")
                    .small()
                    .outline()
                    .disabled(!can_cancel)
                    .tooltip("Cancel the running statement (Esc)")
                    .on_click(cx.listener(|workspace, _, _, cx| workspace.cancel(cx))),
            )
            .child(div().w(px(8.0)))
            .child(
                Button::new("new-tab")
                    .label("New tab")
                    .small()
                    .ghost()
                    .disabled(!can_open)
                    .tooltip("Open a new editor tab (Ctrl+T)")
                    .on_click(cx.listener(|workspace, _, _, cx| workspace.new_tab(cx))),
            )
            .into_any_element()
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let tab_text = state
            .tabs
            .active()
            .and_then(|tab| {
                let status = state.tab_status(tab.id)?;
                Some(format!(
                    "{} · {}",
                    state.connections.name(tab.connection),
                    status.label()
                ))
            })
            .unwrap_or_else(|| "No open tab".to_string());
        let notice = state.notice.clone();
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        h_flex()
            .h(px(24.0))
            .px_3()
            .gap_3()
            .flex_shrink_0()
            .items_center()
            .text_xs()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().status_bar)
            .child(div().text_color(muted).child(SharedString::from(tab_text)))
            .children(notice.map(|message| {
                h_flex()
                    .id("notice")
                    .gap_2()
                    .text_color(danger)
                    .child(SharedString::from(message))
                    .child(div().cursor_pointer().child("×"))
                    .on_click(cx.listener(|workspace, _, _, cx| {
                        workspace.state.update(cx, |state, cx| {
                            state.dismiss_notice();
                            cx.notify();
                        });
                    }))
            }))
            .into_any_element()
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = {
            let state = self.state.read(cx);
            state
                .tabs
                .active_id()
                .is_some_and(|tab| state.can_cancel(tab))
        };
        let context = if running {
            format!("{WORKSPACE_CONTEXT} {RUNNING_CONTEXT}")
        } else {
            WORKSPACE_CONTEXT.to_string()
        };
        let toolbar = self.render_toolbar(cx);
        let status_bar = self.render_status_bar(cx);
        v_flex()
            .id("workspace")
            .size_full()
            .key_context(context.as_str())
            .track_focus(&self.focus_handle)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|workspace, _: &RunCurrent, _, cx| {
                workspace.run(RunMode::Current, cx);
            }))
            .on_action(cx.listener(|workspace, _: &RunAll, _, cx| {
                workspace.run(RunMode::All, cx);
            }))
            .on_action(cx.listener(|workspace, _: &CancelExecution, _, cx| {
                workspace.cancel(cx);
            }))
            .on_action(cx.listener(|workspace, _: &NewTab, _, cx| workspace.new_tab(cx)))
            .on_action(cx.listener(|workspace, _: &CloseActiveTab, window, cx| {
                workspace.close_active_tab(window, cx);
            }))
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(self.dock.clone()))
            .child(status_bar)
    }
}
