//! The Core service: turns `Command` values into driver calls and driver results into `Event`
//! values (docs/PROJECT.md Part 11). This is the connections-and-metadata skeleton; tab actors,
//! execution and result streaming land with the next step of 11.5.

mod connector;
mod dispatcher;
mod error_info;
mod keychain;
mod secrets;
mod store;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use dbased_commands::CoreLink;

pub use self::connector::{DefaultConnector, DriverConnector};
pub use self::error_info::error_info;
pub use self::keychain::KeychainSecretStore;
pub use self::secrets::{InMemorySecretStore, SecretStore};
pub use self::store::{ConnectionStore, StoredConnection};

/// Everything the Core needs from the outside; nothing is a process-wide default.
pub struct CoreDependencies {
    pub connection_store: ConnectionStore,
    pub secret_store: Arc<dyn SecretStore>,
    pub connector: Arc<dyn DriverConnector>,
}

pub struct Core {
    dependencies: CoreDependencies,
}

impl Core {
    pub fn new(dependencies: CoreDependencies) -> Self {
        Self { dependencies }
    }

    /// Runs until every command sender is dropped, then closes every driver and returns.
    /// That is the only way out, so a normal exit never skips `Driver::close()`.
    pub async fn run(self, link: CoreLink) {
        dispatcher::Dispatcher::new(self.dependencies, link.events)
            .run(link.commands)
            .await;
    }
}
