use std::{cell::RefCell, collections::HashMap, rc::Rc};

use dbased_commands::ConnectionState;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _,
    dock::{BasePanel, Panel, PanelEvent},
    h_flex,
    list::ListItem,
    tree::{TreeEvent, TreeItem, TreeState, tree},
    v_flex,
};

use crate::{
    client::CommandSink,
    state::{ExplorerNode, NodeKey, NodeKind, UiState, kind_label},
};

/// What the row renderer needs to know about a tree item, by item id.
#[derive(Clone)]
struct RowInfo {
    glyph: &'static str,
    detail: Option<String>,
    muted: bool,
    online: bool,
}

type RowTable = Rc<RefCell<HashMap<SharedString, RowInfo>>>;

/// Connection and schema tree (docs/PROJECT.md 4.1): children load when a node expands.
pub struct ExplorerPanel {
    state: Entity<UiState>,
    sink: CommandSink,
    tree: Entity<TreeState>,
    keys: HashMap<SharedString, NodeKey>,
    rows: RowTable,
    focus_handle: FocusHandle,
    _observe_state: Subscription,
    _observe_tree: Subscription,
}

impl ExplorerPanel {
    pub fn new(state: Entity<UiState>, sink: CommandSink, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let observe_state = cx.observe(&state, |panel, _, cx| panel.rebuild(cx));
        let observe_tree = cx.subscribe(&tree, |panel, _, event: &TreeEvent, cx| {
            panel.on_tree_event(event, cx);
        });
        let mut panel = Self {
            state,
            sink,
            tree,
            keys: HashMap::new(),
            rows: Rc::new(RefCell::new(HashMap::new())),
            focus_handle: cx.focus_handle(),
            _observe_state: observe_state,
            _observe_tree: observe_tree,
        };
        panel.rebuild(cx);
        panel
    }

    /// Labels of the rows currently in the tree, in display order.
    #[cfg(test)]
    pub fn visible_labels(&self, cx: &App) -> Vec<String> {
        let tree = self.tree.read(cx);
        (0..)
            .map_while(|index| tree.entry(index))
            .map(|entry| entry.item().label.to_string())
            .collect()
    }

    #[cfg(test)]
    pub fn tree(&self) -> &Entity<TreeState> {
        &self.tree
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let nodes = {
            let state = self.state.read(cx);
            state.explorer.nodes(&state.connections)
        };
        self.keys.clear();
        self.rows.borrow_mut().clear();
        let items: Vec<TreeItem> = nodes.iter().map(|node| self.tree_item(node)).collect();
        let selected = self
            .tree
            .read(cx)
            .selected_item()
            .map(|item| item.id.clone());
        self.tree.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            if let Some(id) = selected
                && let Some(index) = tree.index_of(&id)
            {
                tree.set_selected_index(Some(index), cx);
            }
        });
        cx.notify();
    }

    fn tree_item(&mut self, node: &ExplorerNode) -> TreeItem {
        let id = SharedString::from(node.id.clone());
        if let Some(key) = &node.key {
            self.keys.insert(id.clone(), key.clone());
        }
        self.rows.borrow_mut().insert(id.clone(), row_info(node));
        let children: Vec<TreeItem> = node
            .children
            .iter()
            .map(|child| self.tree_item(child))
            .collect();
        TreeItem::new(id, node.label.clone())
            .expanded(node.expanded)
            .children(children)
    }

    fn on_tree_event(&mut self, event: &TreeEvent, cx: &mut Context<Self>) {
        let (id, expand) = match event {
            TreeEvent::Expanded(id) => (id, true),
            TreeEvent::Collapsed(id) => (id, false),
        };
        let Some(key) = self.keys.get(id).cloned() else {
            return;
        };
        // @NOTE: toggling only when the model disagrees keeps the two views from drifting.
        if self.state.read(cx).explorer.is_expanded(&key) == expand {
            return;
        }
        let commands = self.state.update(cx, |state, cx| {
            cx.notify();
            state.toggle_node(&key)
        });
        for command in commands {
            if let Err(failure) = self.sink.send(command) {
                self.state.update(cx, |state, cx| {
                    state.send_failed(failure);
                    cx.notify();
                });
                break;
            }
        }
    }
}

fn row_info(node: &ExplorerNode) -> RowInfo {
    let (glyph, muted, online) = match &node.kind {
        NodeKind::Connection { state, .. } => ("●", false, *state == ConnectionState::Connected),
        NodeKind::Schema => ("▤", false, false),
        NodeKind::Table => ("▦", false, false),
        NodeKind::View => ("◫", false, false),
        NodeKind::Column { .. } => ("·", false, false),
        NodeKind::Placeholder => ("", true, false),
    };
    let detail = match &node.kind {
        NodeKind::Connection { database, .. } => Some(kind_label(*database).to_string()),
        _ => node.detail.clone(),
    };
    RowInfo {
        glyph,
        detail,
        muted,
        online,
    }
}

impl EventEmitter<PanelEvent> for ExplorerPanel {}

impl Focusable for ExplorerPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ExplorerPanel {
    fn panel_name(&self) -> &'static str {
        "explorer"
    }

    fn closable(&self, _cx: &App) -> bool {
        false
    }

    fn zoomable(&self, _cx: &App) -> bool {
        false
    }
}

impl Panel for ExplorerPanel {
    fn tab_name(&self, _cx: &App) -> Option<SharedString> {
        Some("Explorer".into())
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        "Explorer"
    }
}

impl Render for ExplorerPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows.clone();
        let success = cx.theme().success;
        let muted_foreground = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        let body = if self.state.read(cx).connections.is_empty() {
            div()
                .p_3()
                .text_sm()
                .text_color(muted_foreground)
                .child("No connections yet.")
                .into_any_element()
        } else {
            tree(&self.tree, move |index, entry, selected, _window, _cx| {
                let info = rows.borrow().get(&entry.item().id).cloned();
                let info = info.unwrap_or(RowInfo {
                    glyph: "",
                    detail: None,
                    muted: true,
                    online: false,
                });
                let glyph_color = if info.online {
                    success
                } else {
                    muted_foreground
                };
                let label_color = if info.muted {
                    muted_foreground
                } else {
                    foreground
                };
                let mut line = h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(12.0)).text_color(glyph_color).child(info.glyph))
                    .child(
                        div()
                            .text_color(label_color)
                            .child(SharedString::from(entry.item().label.to_string())),
                    );
                if let Some(detail) = info.detail {
                    line = line.child(
                        div()
                            .text_xs()
                            .text_color(muted_foreground)
                            .child(SharedString::from(detail)),
                    );
                }
                ListItem::new(index)
                    .selected(selected)
                    .pl(px(8.0 + 14.0 * entry.depth() as f32))
                    .py_0p5()
                    .child(line)
            })
            .into_any_element()
        };
        v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .child(div().flex_1().min_h_0().child(body))
    }
}
