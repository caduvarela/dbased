use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use dbased_commands::ConnectionId;
use secrecy::SecretString;

use crate::CoreError;

/// Password storage keyed by connection (docs/PROJECT.md 5.1). Calls may block (a keychain
/// can prompt), so the Core runs them on blocking threads.
pub trait SecretStore: Send + Sync {
    fn get(&self, id: ConnectionId) -> Result<Option<SecretString>, CoreError>;
    fn set(&self, id: ConnectionId, password: &SecretString) -> Result<(), CoreError>;
    fn delete(&self, id: ConnectionId) -> Result<(), CoreError>;
}

/// For tests; nothing is persisted.
#[derive(Default)]
pub struct InMemorySecretStore {
    secrets: Mutex<HashMap<ConnectionId, SecretString>>,
}

impl InMemorySecretStore {
    fn lock(&self) -> Result<MutexGuard<'_, HashMap<ConnectionId, SecretString>>, CoreError> {
        self.secrets
            .lock()
            .map_err(|_| CoreError::SecretStore("the in-memory store is poisoned".to_owned()))
    }
}

impl SecretStore for InMemorySecretStore {
    fn get(&self, id: ConnectionId) -> Result<Option<SecretString>, CoreError> {
        Ok(self.lock()?.get(&id).cloned())
    }

    fn set(&self, id: ConnectionId, password: &SecretString) -> Result<(), CoreError> {
        self.lock()?.insert(id, password.clone());
        Ok(())
    }

    fn delete(&self, id: ConnectionId) -> Result<(), CoreError> {
        self.lock()?.remove(&id);
        Ok(())
    }
}
