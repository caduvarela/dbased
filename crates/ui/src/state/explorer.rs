use std::collections::{HashMap, HashSet};

use dbased_commands::{
    Command, ConnectionId, ConnectionState, DatabaseKind, ErrorInfo, MetadataItem,
    MetadataItemKind, MetadataNode, PasswordSource, RequestId,
};

use super::{connections::ConnectionsModel, ids::IdAllocator};

/// Identifies one expandable node of the explorer tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeKey {
    Connection(ConnectionId),
    Schema {
        connection: ConnectionId,
        schema: String,
    },
    Table {
        connection: ConnectionId,
        schema: String,
        name: String,
    },
}

impl NodeKey {
    pub fn connection(&self) -> ConnectionId {
        match self {
            NodeKey::Connection(connection)
            | NodeKey::Schema { connection, .. }
            | NodeKey::Table { connection, .. } => *connection,
        }
    }

    pub fn metadata_node(&self) -> MetadataNode {
        match self {
            NodeKey::Connection(_) => MetadataNode::Connection,
            NodeKey::Schema { schema, .. } => MetadataNode::Schema {
                schema: schema.clone(),
            },
            NodeKey::Table { schema, name, .. } => MetadataNode::Table {
                schema: schema.clone(),
                name: name.clone(),
            },
        }
    }

    fn from_node(connection: ConnectionId, node: &MetadataNode) -> Self {
        match node {
            MetadataNode::Connection => NodeKey::Connection(connection),
            MetadataNode::Schema { schema } => NodeKey::Schema {
                connection,
                schema: schema.clone(),
            },
            MetadataNode::Table { schema, name } => NodeKey::Table {
                connection,
                schema: schema.clone(),
                name: name.clone(),
            },
        }
    }

    /// Stable id for the tree widget.
    pub fn tree_id(&self) -> String {
        format!("{self:?}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Connection {
        database: DatabaseKind,
        state: ConnectionState,
    },
    Schema,
    Table,
    View,
    Column {
        nullable: bool,
    },
    /// A row that is not data: "Loading...", "(empty)" or an error message.
    Placeholder,
}

/// One node of the tree as the view draws it; the view never needs to know the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplorerNode {
    pub id: String,
    /// Present for the nodes the user can expand.
    pub key: Option<NodeKey>,
    pub label: String,
    pub detail: Option<String>,
    pub kind: NodeKind,
    pub expanded: bool,
    pub children: Vec<ExplorerNode>,
}

#[derive(Debug, Clone)]
enum LoadState {
    Loading(RequestId),
    Loaded(Vec<MetadataItem>),
    Failed(String),
}

/// Expansion and lazy-loaded children of the explorer tree (docs/PROJECT.md 4.1).
#[derive(Debug, Default)]
pub struct ExplorerModel {
    expanded: HashSet<NodeKey>,
    loads: HashMap<NodeKey, LoadState>,
}

impl ExplorerModel {
    pub fn is_expanded(&self, key: &NodeKey) -> bool {
        self.expanded.contains(key)
    }

    /// Expands or collapses a node; expanding an unloaded node asks the Core for its children.
    pub fn toggle(
        &mut self,
        key: &NodeKey,
        connections: &ConnectionsModel,
        ids: &mut IdAllocator,
    ) -> Vec<Command> {
        if self.expanded.remove(key) {
            return Vec::new();
        }
        self.expanded.insert(key.clone());
        let needs_load = !matches!(
            self.loads.get(key),
            Some(LoadState::Loading(_) | LoadState::Loaded(_))
        );
        if !needs_load {
            return Vec::new();
        }
        let mut commands = Vec::new();
        if let NodeKey::Connection(id) = key
            && let Some(connection) = connections.get(*id)
            && connection.state == ConnectionState::Disconnected
        {
            let password = if connection.password_saved {
                PasswordSource::Saved
            } else {
                PasswordSource::None
            };
            commands.push(Command::Connect {
                request: ids.request(),
                id: *id,
                password,
            });
        }
        let request = ids.request();
        self.loads.insert(key.clone(), LoadState::Loading(request));
        commands.push(Command::LoadChildren {
            request,
            connection: key.connection(),
            node: key.metadata_node(),
        });
        commands
    }

    /// Returns true when the event answered a pending load.
    pub fn apply_children(
        &mut self,
        request: RequestId,
        connection: ConnectionId,
        node: &MetadataNode,
        items: Vec<MetadataItem>,
    ) -> bool {
        let key = NodeKey::from_node(connection, node);
        if !matches!(self.loads.get(&key), Some(LoadState::Loading(pending)) if *pending == request)
        {
            return false;
        }
        self.loads.insert(key, LoadState::Loaded(items));
        true
    }

    /// Returns true when the failed request was a pending load.
    pub fn apply_failure(&mut self, request: RequestId, error: &ErrorInfo) -> bool {
        let failed = self.loads.iter().find_map(|(key, load)| match load {
            LoadState::Loading(pending) if *pending == request => Some(key.clone()),
            _ => None,
        });
        let Some(key) = failed else {
            return false;
        };
        self.loads
            .insert(key, LoadState::Failed(error.message.clone()));
        true
    }

    /// Drops everything cached for a connection (it went away or disconnected).
    pub fn forget_connection(&mut self, connection: ConnectionId) {
        self.expanded.retain(|key| key.connection() != connection);
        self.loads.retain(|key, _| key.connection() != connection);
    }

    pub fn retain_connections(&mut self, connections: &ConnectionsModel) {
        self.expanded
            .retain(|key| connections.get(key.connection()).is_some());
        self.loads
            .retain(|key, _| connections.get(key.connection()).is_some());
    }

    /// The whole tree, ready to draw.
    pub fn nodes(&self, connections: &ConnectionsModel) -> Vec<ExplorerNode> {
        connections
            .items()
            .iter()
            .map(|connection| {
                let key = NodeKey::Connection(connection.id);
                let mut node = self.expandable(
                    key,
                    connection.settings.name.clone(),
                    NodeKind::Connection {
                        database: connection.settings.kind,
                        state: connection.state,
                    },
                );
                node.detail = Some(format!(
                    "{}:{}",
                    connection.settings.host, connection.settings.port
                ));
                node
            })
            .collect()
    }

    fn expandable(&self, key: NodeKey, label: String, kind: NodeKind) -> ExplorerNode {
        let expanded = self.is_expanded(&key);
        let children = self.children_of(&key);
        ExplorerNode {
            id: key.tree_id(),
            key: Some(key),
            label,
            detail: None,
            kind,
            expanded,
            children,
        }
    }

    fn children_of(&self, key: &NodeKey) -> Vec<ExplorerNode> {
        let placeholder = |text: &str| {
            vec![ExplorerNode {
                id: format!("{}/placeholder", key.tree_id()),
                key: None,
                label: text.to_string(),
                detail: None,
                kind: NodeKind::Placeholder,
                expanded: false,
                children: Vec::new(),
            }]
        };
        match self.loads.get(key) {
            None | Some(LoadState::Loading(_)) => placeholder("Loading..."),
            Some(LoadState::Failed(message)) => placeholder(&format!("Failed: {message}")),
            Some(LoadState::Loaded(items)) if items.is_empty() => placeholder("(empty)"),
            Some(LoadState::Loaded(items)) => items
                .iter()
                .map(|item| self.child_node(key, item))
                .collect(),
        }
    }

    fn child_node(&self, parent: &NodeKey, item: &MetadataItem) -> ExplorerNode {
        let connection = parent.connection();
        match &item.kind {
            MetadataItemKind::Schema => self.expandable(
                NodeKey::Schema {
                    connection,
                    schema: item.name.clone(),
                },
                item.name.clone(),
                NodeKind::Schema,
            ),
            MetadataItemKind::Table | MetadataItemKind::View => {
                let kind = if item.kind == MetadataItemKind::Table {
                    NodeKind::Table
                } else {
                    NodeKind::View
                };
                let schema = match parent {
                    NodeKey::Schema { schema, .. } => schema.clone(),
                    _ => String::new(),
                };
                self.expandable(
                    NodeKey::Table {
                        connection,
                        schema,
                        name: item.name.clone(),
                    },
                    item.name.clone(),
                    kind,
                )
            }
            MetadataItemKind::Column {
                type_name,
                nullable,
            } => ExplorerNode {
                id: format!("{}/column/{}", parent.tree_id(), item.name),
                key: None,
                label: item.name.clone(),
                detail: Some(type_name.clone()),
                kind: NodeKind::Column {
                    nullable: *nullable,
                },
                expanded: false,
                children: Vec::new(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use dbased_commands::{ConnectionSettings, ConnectionSummary, ErrorKind, TlsMode};

    use super::*;

    fn connections() -> ConnectionsModel {
        let mut model = ConnectionsModel::default();
        model.replace(vec![ConnectionSummary {
            id: ConnectionId(1),
            settings: ConnectionSettings {
                name: "Local".to_string(),
                kind: DatabaseKind::Postgres,
                host: "127.0.0.1".to_string(),
                port: 5432,
                user: "u".to_string(),
                database: "d".to_string(),
                tls: TlsMode::Prefer,
            },
            password_saved: true,
            state: ConnectionState::Disconnected,
        }]);
        model
    }

    fn schema_item(name: &str) -> MetadataItem {
        MetadataItem {
            kind: MetadataItemKind::Schema,
            name: name.to_string(),
        }
    }

    #[test]
    fn expanding_a_disconnected_connection_connects_then_loads() {
        let mut explorer = ExplorerModel::default();
        let mut ids = IdAllocator::default();
        let commands = explorer.toggle(
            &NodeKey::Connection(ConnectionId(1)),
            &connections(),
            &mut ids,
        );
        assert!(matches!(
            commands.as_slice(),
            [
                Command::Connect {
                    password: PasswordSource::Saved,
                    ..
                },
                Command::LoadChildren {
                    node: MetadataNode::Connection,
                    ..
                }
            ]
        ));
    }

    #[test]
    fn the_tree_shows_a_placeholder_until_children_arrive() {
        let mut explorer = ExplorerModel::default();
        let mut ids = IdAllocator::default();
        let connections = connections();
        let key = NodeKey::Connection(ConnectionId(1));
        explorer.toggle(&key, &connections, &mut ids);
        let nodes = explorer.nodes(&connections);
        assert!(nodes[0].expanded);
        assert_eq!(nodes[0].children[0].label, "Loading...");
        assert_eq!(nodes[0].children[0].kind, NodeKind::Placeholder);
    }

    #[test]
    fn loaded_children_replace_the_placeholder() {
        let mut explorer = ExplorerModel::default();
        let mut ids = IdAllocator::default();
        let connections = connections();
        let key = NodeKey::Connection(ConnectionId(1));
        explorer.toggle(&key, &connections, &mut ids);
        let applied = explorer.apply_children(
            RequestId(2),
            ConnectionId(1),
            &MetadataNode::Connection,
            vec![schema_item("public")],
        );
        assert!(applied);
        let nodes = explorer.nodes(&connections);
        assert_eq!(nodes[0].children.len(), 1);
        assert_eq!(nodes[0].children[0].label, "public");
        assert!(nodes[0].children[0].key.is_some());
    }

    #[test]
    fn collapsing_and_expanding_again_does_not_reload() {
        let mut explorer = ExplorerModel::default();
        let mut ids = IdAllocator::default();
        let connections = connections();
        let key = NodeKey::Connection(ConnectionId(1));
        explorer.toggle(&key, &connections, &mut ids);
        explorer.apply_children(
            RequestId(2),
            ConnectionId(1),
            &MetadataNode::Connection,
            Vec::new(),
        );
        assert!(explorer.toggle(&key, &connections, &mut ids).is_empty());
        assert!(!explorer.is_expanded(&key));
        assert!(explorer.toggle(&key, &connections, &mut ids).is_empty());
        assert!(explorer.is_expanded(&key));
    }

    #[test]
    fn a_failed_load_shows_the_message_and_can_be_retried() {
        let mut explorer = ExplorerModel::default();
        let mut ids = IdAllocator::default();
        let connections = connections();
        let key = NodeKey::Connection(ConnectionId(1));
        explorer.toggle(&key, &connections, &mut ids);
        let error = ErrorInfo {
            kind: ErrorKind::Connection,
            message: "refused".to_string(),
            database: None,
        };
        assert!(explorer.apply_failure(RequestId(2), &error));
        assert_eq!(
            explorer.nodes(&connections)[0].children[0].label,
            "Failed: refused"
        );
        explorer.toggle(&key, &connections, &mut ids);
        let retry = explorer.toggle(&key, &connections, &mut ids);
        assert!(
            retry
                .iter()
                .any(|command| matches!(command, Command::LoadChildren { .. }))
        );
    }

    #[test]
    fn a_stale_answer_is_ignored() {
        let mut explorer = ExplorerModel::default();
        let mut ids = IdAllocator::default();
        let key = NodeKey::Connection(ConnectionId(1));
        explorer.toggle(&key, &connections(), &mut ids);
        let applied = explorer.apply_children(
            RequestId(99),
            ConnectionId(1),
            &MetadataNode::Connection,
            Vec::new(),
        );
        assert!(!applied);
    }

    #[test]
    fn forgetting_a_connection_collapses_and_clears_it() {
        let mut explorer = ExplorerModel::default();
        let mut ids = IdAllocator::default();
        let key = NodeKey::Connection(ConnectionId(1));
        explorer.toggle(&key, &connections(), &mut ids);
        explorer.forget_connection(ConnectionId(1));
        assert!(!explorer.is_expanded(&key));
    }
}
