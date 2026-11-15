use crate::{
    ConnectionId, ConnectionSettings, ConnectionState, ConnectionSummary, DatabaseKind,
    MetadataItem, MetadataItemKind, MetadataNode, TlsMode,
};

pub fn initial_connections() -> Vec<ConnectionSummary> {
    vec![
        connection(1, "Local Postgres", DatabaseKind::Postgres, 5432),
        connection(2, "Local MySQL", DatabaseKind::MySql, 3306),
    ]
}

fn connection(id: u64, name: &str, kind: DatabaseKind, port: u16) -> ConnectionSummary {
    ConnectionSummary {
        id: ConnectionId(id),
        settings: ConnectionSettings {
            name: name.to_string(),
            kind,
            host: "127.0.0.1".to_string(),
            port,
            user: "dbased".to_string(),
            database: "dbased".to_string(),
            tls: TlsMode::Prefer,
        },
        password_saved: true,
        state: ConnectionState::Disconnected,
    }
}

fn item(kind: MetadataItemKind, name: &str) -> MetadataItem {
    MetadataItem {
        kind,
        name: name.to_string(),
    }
}

fn column(name: &str, type_name: &str, nullable: bool) -> MetadataItem {
    item(
        MetadataItemKind::Column {
            type_name: type_name.to_string(),
            nullable,
        },
        name,
    )
}

/// The same small catalog for both fake connections.
pub fn children(node: &MetadataNode) -> Vec<MetadataItem> {
    match node {
        MetadataNode::Connection => vec![
            item(MetadataItemKind::Schema, "public"),
            item(MetadataItemKind::Schema, "sales"),
        ],
        MetadataNode::Schema { schema } if schema == "public" => vec![
            item(MetadataItemKind::Table, "orders"),
            item(MetadataItemKind::Table, "users"),
            item(MetadataItemKind::View, "active_users"),
        ],
        MetadataNode::Schema { schema } if schema == "sales" => {
            vec![item(MetadataItemKind::Table, "invoices")]
        }
        MetadataNode::Table { schema, name } if schema == "public" && name == "users" => vec![
            column("id", "bigint", false),
            column("email", "text", false),
            column("nickname", "text", true),
        ],
        MetadataNode::Table { schema, name } if schema == "public" && name == "orders" => vec![
            column("id", "bigint", false),
            column("user_id", "bigint", false),
            column("total", "numeric", false),
        ],
        MetadataNode::Table { schema, name } if schema == "sales" && name == "invoices" => vec![
            column("id", "bigint", false),
            column("paid_at", "timestamp", true),
        ],
        _ => Vec::new(),
    }
}
