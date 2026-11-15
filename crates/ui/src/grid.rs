use std::ops::Range;

use dbased_commands::{ColumnMeta, Command, ResultId, Value};
use gpui::{
    App, Context, Entity, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div,
    px,
};
use gpui_component::{
    ActiveTheme as _, h_flex,
    table::{Column, TableDelegate, TableState},
};

use crate::{
    client::CommandSink,
    state::{CellLookup, FetchRequest, UiState, WindowCache, is_numeric_type},
};

/// Feeds gpui-component's virtual `Table` from a bounded window cache (docs/PROJECT.md 6.3).
/// Rows are pulled with `FetchRows` and arrive as `RowsWindow`; nothing is pushed.
pub struct GridDelegate {
    result: ResultId,
    columns: Vec<Column>,
    cache: WindowCache,
    sink: CommandSink,
    state: Entity<UiState>,
}

impl GridDelegate {
    pub fn new(
        result: ResultId,
        columns: &[ColumnMeta],
        sink: CommandSink,
        state: Entity<UiState>,
    ) -> Self {
        Self {
            result,
            columns: columns.iter().map(grid_column).collect(),
            cache: WindowCache::default(),
            sink,
            state,
        }
    }

    pub fn total_rows(&self) -> u64 {
        self.cache.total_rows()
    }

    pub fn cached_windows(&self) -> usize {
        self.cache.cached_windows()
    }

    pub fn set_total_rows(&mut self, total_rows: u64, cx: &mut App) {
        let requests = self.cache.set_total_rows(total_rows);
        self.fetch(requests, cx);
    }

    pub fn apply_window(&mut self, start: u64, rows: Vec<Vec<Value>>) {
        self.cache.apply_window(start, rows);
    }

    fn fetch(&mut self, requests: Vec<FetchRequest>, cx: &mut App) {
        for request in requests {
            let command = Command::FetchRows {
                result: self.result,
                start: request.start,
                count: request.count,
            };
            if let Err(failure) = self.sink.send(command) {
                self.state.update(cx, |state, cx| {
                    state.send_failed(failure);
                    cx.notify();
                });
                return;
            }
        }
    }
}

fn grid_column(meta: &ColumnMeta) -> Column {
    let column = Column::new(meta.name.clone(), meta.name.clone())
        .width(px(column_width(meta)));
    if is_numeric_type(&meta.type_name) {
        column.text_right()
    } else {
        column
    }
}

fn column_width(meta: &ColumnMeta) -> f32 {
    let name_width = meta.name.chars().count() as f32 * 8.0 + 32.0;
    let type_width = match meta.type_name.as_str() {
        "boolean" | "bool" => 80.0,
        "bigint" | "integer" | "int" | "smallint" => 100.0,
        "timestamp" | "timestamptz" | "datetime" => 180.0,
        "uuid" => 300.0,
        "text" | "jsonb" | "json" | "varchar" => 220.0,
        _ => 140.0,
    };
    name_width.max(type_width).min(420.0)
}

impl TableDelegate for GridDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        usize::try_from(self.cache.total_rows()).unwrap_or(usize::MAX)
    }

    fn column(&self, column_index: usize, _cx: &App) -> Column {
        self.columns[column_index].clone()
    }

    fn visible_rows_changed(
        &mut self,
        visible_range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        let range = visible_range.start as u64..visible_range.end as u64;
        let requests = self.cache.visible_changed(range);
        self.fetch(requests, cx);
    }

    fn render_td(
        &mut self,
        row_index: usize,
        column_index: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        match self.cache.cell(row_index as u64, column_index) {
            CellLookup::Ready(cell) if cell.is_null => div()
                .italic()
                .text_color(cx.theme().muted_foreground)
                .child(SharedString::from(cell.text.clone())),
            CellLookup::Ready(cell) => div().child(SharedString::from(cell.text.clone())),
            CellLookup::Loading => div().child(
                h_flex()
                    .h_full()
                    .items_center()
                    .child(div().h(px(8.0)).w(px(48.0)).rounded_sm().bg(cx.theme().skeleton)),
            ),
        }
    }

    fn cell_text(&self, row_index: usize, column_index: usize, _cx: &App) -> String {
        match self.cache.cell(row_index as u64, column_index) {
            CellLookup::Ready(cell) => cell.text.clone(),
            CellLookup::Loading => String::new(),
        }
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        h_flex()
            .size_full()
            .items_center()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .child("No rows")
    }
}
