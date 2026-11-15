use dbased_commands::{ConnectionId, ConnectionState, ConnectionSummary, DatabaseKind};

/// The connection list as the Core last reported it.
#[derive(Debug, Default)]
pub struct ConnectionsModel {
    items: Vec<ConnectionSummary>,
}

impl ConnectionsModel {
    pub fn replace(&mut self, items: Vec<ConnectionSummary>) {
        self.items = items;
    }

    pub fn set_state(&mut self, id: ConnectionId, state: ConnectionState) {
        if let Some(connection) = self.items.iter_mut().find(|item| item.id == id) {
            connection.state = state;
        }
    }

    pub fn get(&self, id: ConnectionId) -> Option<&ConnectionSummary> {
        self.items.iter().find(|item| item.id == id)
    }

    pub fn state_of(&self, id: Option<ConnectionId>) -> Option<ConnectionState> {
        self.get(id?).map(|connection| connection.state)
    }

    pub fn items(&self) -> &[ConnectionSummary] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn first_id(&self) -> Option<ConnectionId> {
        self.items.first().map(|item| item.id)
    }

    /// Name to show for `id`, also when the connection no longer exists.
    pub fn name(&self, id: ConnectionId) -> String {
        match self.get(id) {
            Some(connection) => connection.settings.name.clone(),
            None => format!("connection {}", id.0),
        }
    }
}

pub fn kind_label(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::Postgres => "PostgreSQL",
        DatabaseKind::MySql => "MySQL",
    }
}
