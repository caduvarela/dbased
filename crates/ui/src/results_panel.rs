use dbased_commands::{ResultId, ResultStatus, Value};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _,
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    table::{DataTable, TableState},
    v_flex,
};

use crate::{
    client::CommandSink,
    grid::GridDelegate,
    state::{ResultModel, UiState},
};

/// One result tab: a status strip over a virtual grid, or a message for statements
/// that return no rows (docs/PROJECT.md 9.2).
pub struct ResultPanel {
    result: ResultId,
    state: Entity<UiState>,
    sink: CommandSink,
    table: Option<Entity<TableState<GridDelegate>>>,
    focus_handle: FocusHandle,
    _observe_state: Subscription,
}

impl ResultPanel {
    pub fn new(
        result: ResultId,
        state: Entity<UiState>,
        sink: CommandSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe_state = cx.observe(&state, |panel, _, cx| panel.sync_from_state(cx));
        let table = grid_for(result, &state, &sink, window, cx);
        let mut panel = Self {
            result,
            state,
            sink,
            table,
            focus_handle: cx.focus_handle(),
            _observe_state: observe_state,
        };
        panel.sync_from_state(cx);
        panel
    }

    pub fn result(&self) -> ResultId {
        self.result
    }

    pub fn table(&self) -> Option<&Entity<TableState<GridDelegate>>> {
        self.table.as_ref()
    }

    /// Stores a window the Core sent in answer to a `FetchRows`.
    pub fn apply_window(&mut self, start: u64, rows: Vec<Vec<Value>>, cx: &mut Context<Self>) {
        let Some(table) = &self.table else {
            return;
        };
        table.update(cx, |table, cx| {
            table.delegate_mut().apply_window(start, rows);
            cx.notify();
        });
    }

    fn sync_from_state(&mut self, cx: &mut Context<Self>) {
        let Some(total) = self
            .state
            .read(cx)
            .result(self.result)
            .map(ResultModel::row_count)
        else {
            return;
        };
        if let Some(table) = &self.table {
            table.update(cx, |table, cx| {
                if table.delegate().total_rows() == total {
                    return;
                }
                table.delegate_mut().set_total_rows(total, cx);
                cx.notify();
            });
        }
        cx.notify();
    }

    fn tab_title(&self, cx: &App) -> String {
        let state = self.state.read(cx);
        let Some(result) = state.result(self.result) else {
            return "Result".to_string();
        };
        let tab_title = state
            .tabs
            .get(result.tab)
            .map(|tab| tab.title.clone())
            .unwrap_or_default();
        format!("{tab_title} · {}", result.title())
    }
}

fn grid_for(
    result: ResultId,
    state: &Entity<UiState>,
    sink: &CommandSink,
    window: &mut Window,
    cx: &mut Context<ResultPanel>,
) -> Option<Entity<TableState<GridDelegate>>> {
    let columns = {
        let model = state.read(cx).result(result)?;
        if !model.has_grid() {
            return None;
        }
        model.columns.clone()
    };
    let delegate = GridDelegate::new(result, &columns, sink.clone(), state.clone());
    Some(cx.new(|cx| {
        TableState::new(delegate, window, cx)
            .sortable(false)
            .col_movable(false)
            .loop_selection(false)
    }))
}

impl EventEmitter<PanelEvent> for ResultPanel {}

impl Focusable for ResultPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ResultPanel {
    fn panel_name(&self) -> &'static str {
        "result"
    }

    fn zoomable(&self, _cx: &App) -> bool {
        false
    }

    fn on_removed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let commands = self
            .state
            .update(cx, |state, _| state.release_result(self.result));
        for command in commands {
            if let Err(failure) = self.sink.send(command) {
                self.state.update(cx, |state, _| state.send_failed(failure));
            }
        }
    }
}

impl Panel for ResultPanel {
    fn tab_name(&self, cx: &App) -> Option<SharedString> {
        Some(self.tab_title(cx).into())
    }

    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        SharedString::from(self.tab_title(cx))
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl Render for ResultPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(model) = self.state.read(cx).result(self.result).cloned() else {
            return v_flex().size_full().into_any_element();
        };
        let status_color = match model.status() {
            Some(ResultStatus::Failed) => cx.theme().danger,
            Some(ResultStatus::Cancelled | ResultStatus::Truncated) => cx.theme().warning,
            _ => cx.theme().muted_foreground,
        };
        let strip = h_flex()
            .h(px(26.0))
            .px_3()
            .flex_shrink_0()
            .items_center()
            .text_xs()
            .text_color(status_color)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(SharedString::from(model.status_line()));
        let body = match &self.table {
            Some(table) => div()
                .flex_1()
                .min_h_0()
                .child(DataTable::new(table).stripe(true))
                .into_any_element(),
            None => message_body(&model, cx).into_any_element(),
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(strip)
            .child(body)
            .into_any_element()
    }
}

fn message_body(model: &ResultModel, cx: &App) -> impl IntoElement {
    let mut lines = Vec::new();
    if let Some(error) = model
        .outcome
        .as_ref()
        .and_then(|outcome| outcome.error.as_ref())
    {
        lines.push(error.message.clone());
        lines.extend(error.detail.clone());
        lines.extend(error.hint.clone().map(|hint| format!("Hint: {hint}")));
    }
    let mut body = v_flex()
        .flex_1()
        .p_3()
        .gap_1()
        .text_sm()
        .text_color(cx.theme().foreground);
    for line in lines {
        body = body.child(SharedString::from(line));
    }
    body
}
