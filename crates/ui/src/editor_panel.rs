use dbased_commands::{ConnectionId, ExecuteTarget, TabId};
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    input::{Editor, EditorState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};

use crate::{
    client::CommandSink,
    state::{TabStatus, UiState},
};

/// What the user asked of a tab's editor that the workspace must act on.
pub enum EditorPanelEvent {
    ConnectionPicked(ConnectionId),
}

/// Which part of the script a run covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// The selection if there is one, else the statement under the cursor.
    Current,
    All,
}

/// One SQL editor tab (docs/PROJECT.md 6.1, 9.1): the editor, plus a strip with the
/// tab's connection picker and its session state.
pub struct EditorPanel {
    tab: TabId,
    state: Entity<UiState>,
    sink: CommandSink,
    editor: Entity<EditorState>,
}

impl EditorPanel {
    pub fn new(
        tab: TabId,
        state: Entity<UiState>,
        sink: CommandSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .line_number(true)
        });
        Self {
            tab,
            state,
            sink,
            editor,
        }
    }

    pub fn tab(&self) -> TabId {
        self.tab
    }

    pub fn set_tab(&mut self, tab: TabId) {
        self.tab = tab;
    }

    pub fn editor(&self) -> &Entity<EditorState> {
        &self.editor
    }

    pub fn text(&self, cx: &App) -> String {
        self.editor.read(cx).value().to_string()
    }

    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_string();
        self.editor
            .update(cx, |editor, cx| editor.set_value(text, window, cx));
    }

    /// Docs/PROJECT.md 6.1: the editor takes keyboard focus when its tab opens or activates.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.editor.update(cx, |editor, cx| editor.focus(window, cx));
    }

    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.editor.read(cx).focus_handle(cx).is_focused(window)
    }

    /// The script and the part of it to run.
    pub fn run_request(&self, mode: RunMode, cx: &App) -> (String, ExecuteTarget) {
        let editor = self.editor.read(cx);
        let script = editor.value().to_string();
        let target = match mode {
            RunMode::All => ExecuteTarget::All,
            RunMode::Current => {
                let selection = editor.selected_range();
                if selection.start == selection.end {
                    ExecuteTarget::Current {
                        cursor: editor.cursor(),
                    }
                } else {
                    ExecuteTarget::Selection {
                        start: selection.start,
                        end: selection.end,
                    }
                }
            }
        };
        (script, target)
    }

    fn connection_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let label = match state.tabs.get(self.tab) {
            Some(tab) => state.connections.name(tab.connection),
            None => "No connection".to_string(),
        };
        let busy = state.tabs.get(self.tab).is_some_and(|tab| tab.activity.is_busy());
        let current = state.tabs.get(self.tab).map(|tab| tab.connection);
        let panel = cx.weak_entity();
        let app_state = self.state.clone();
        Button::new(("connection-picker", self.tab.0))
            .label(SharedString::from(label))
            .small()
            .ghost()
            .disabled(busy)
            .tooltip("Connection of this tab")
            .dropdown_menu(move |menu, _window, cx| {
                let entries: Vec<(ConnectionId, String)> = app_state
                    .read(cx)
                    .connections
                    .items()
                    .iter()
                    .map(|connection| (connection.id, connection.settings.name.clone()))
                    .collect();
                entries.into_iter().fold(menu, |menu, (id, name)| {
                    let panel = panel.clone();
                    menu.item(
                        PopupMenuItem::new(name)
                            .checked(Some(id) == current)
                            .on_click(move |_: &ClickEvent, _window, cx| {
                                panel
                                    .update(cx, |_, cx| {
                                        cx.emit(EditorPanelEvent::ConnectionPicked(id));
                                    })
                                    .ok();
                            }),
                    )
                })
            })
            .into_any_element()
    }
}

impl EventEmitter<PanelEvent> for EditorPanel {}
impl EventEmitter<EditorPanelEvent> for EditorPanel {}

impl Focusable for EditorPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.read(cx).focus_handle(cx)
    }
}

impl BasePanel for EditorPanel {
    fn panel_name(&self) -> &'static str {
        "editor"
    }

    fn zoomable(&self, _cx: &App) -> bool {
        false
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !active {
            return;
        }
        let tab = self.tab;
        self.state.update(cx, |state, cx| {
            state.tabs.set_active(tab);
            cx.notify();
        });
        self.focus(window, cx);
    }

    fn on_removed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.tab;
        let commands = self.state.update(cx, |state, cx| {
            cx.notify();
            state.close_tab(tab)
        });
        for command in commands {
            if let Err(failure) = self.sink.send(command) {
                self.state.update(cx, |state, _| state.send_failed(failure));
            }
        }
    }
}

impl Panel for EditorPanel {
    fn tab_name(&self, cx: &App) -> Option<SharedString> {
        let state = self.state.read(cx);
        state
            .tabs
            .get(self.tab)
            .map(|tab| SharedString::from(tab.title.clone()))
    }

    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let title = state
            .tabs
            .get(self.tab)
            .map(|tab| tab.title.clone())
            .unwrap_or_else(|| "Query".to_string());
        SharedString::from(title)
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl Render for EditorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let status = state.tab_status(self.tab);
        let last_run = state
            .tabs
            .get(self.tab)
            .and_then(|tab| tab.last_run.as_ref())
            .map(|run| run.label());
        let status_color = match status {
            Some(TabStatus::Running | TabStatus::Connecting | TabStatus::Cancelling) => {
                cx.theme().info
            }
            Some(TabStatus::Connected) => cx.theme().success,
            Some(TabStatus::TransactionAborted | TabStatus::SessionReset(_)) => cx.theme().danger,
            _ => cx.theme().muted_foreground,
        };
        let status_label = status.map(|status| status.label()).unwrap_or("");
        let picker = self.connection_picker(cx);
        let strip = h_flex()
            .h(px(30.0))
            .px_2()
            .gap_2()
            .flex_shrink_0()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(picker)
            .child(
                div()
                    .text_xs()
                    .text_color(status_color)
                    .child(status_label),
            )
            .children(last_run.map(|label| {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(label))
            }));
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(strip)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(Editor::new(&self.editor).size_full()),
            )
    }
}
