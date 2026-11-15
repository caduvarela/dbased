use std::sync::Arc;

use dbased_commands::ConnectionId;
use keyring_core::{CredentialStore, Entry, Error as KeyringError};
use secrecy::{ExposeSecret, SecretString};

use super::SecretStore;
use crate::CoreError;

/// Keychain service name; every DBased entry lives under it (docs/PROJECT.md 5.1).
const SERVICE_NAME: &str = "dbased";

/// Passwords in the OS keychain through a `keyring-core` credential store.
///
/// The store is owned by this value and entries are built from it, so nothing relies on
/// `keyring-core`'s process-wide default store. Every call blocks (an unlock prompt can wait
/// indefinitely); the Core runs them on blocking threads.
///
/// @TODO: when `new` fails (no Secret Service on Linux), the app wiring falls back to prompting
/// for the password per session (5.1); an opt-in encrypted-file store may come later.
pub struct KeychainSecretStore {
    store: Arc<CredentialStore>,
}

impl KeychainSecretStore {
    /// Opens the platform credential store. Blocking; call it before the async runtime is
    /// busy or from a blocking thread. Fails with `CoreError::SecretStore` when the platform
    /// has no usable store (for example no Secret Service on Linux).
    pub fn new() -> Result<Self, CoreError> {
        let store = platform_store().map_err(|error| map_error(&error))?;
        Ok(Self::with_store(store))
    }

    /// Wraps an already opened store; tests pass `keyring_core::mock::Store`.
    pub fn with_store(store: Arc<CredentialStore>) -> Self {
        Self { store }
    }

    fn entry(&self, id: ConnectionId) -> Result<Entry, CoreError> {
        self.store
            .build(SERVICE_NAME, &entry_user(id), None)
            .map_err(|error| map_error(&error))
    }
}

impl SecretStore for KeychainSecretStore {
    fn get(&self, id: ConnectionId) -> Result<Option<SecretString>, CoreError> {
        match self.entry(id)?.get_password() {
            Ok(password) => Ok(Some(SecretString::from(password))),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(map_error(&error)),
        }
    }

    fn set(&self, id: ConnectionId, password: &SecretString) -> Result<(), CoreError> {
        self.entry(id)?
            .set_password(password.expose_secret())
            .map_err(|error| map_error(&error))
    }

    /// Deleting a password that is not there succeeds, like the in-memory store.
    fn delete(&self, id: ConnectionId) -> Result<(), CoreError> {
        match self.entry(id)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(map_error(&error)),
        }
    }
}

/// The user part of the entry; the connection id is the stable key the config file references.
fn entry_user(id: ConnectionId) -> String {
    format!("connection-{}", id.0)
}

#[cfg(target_os = "linux")]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    let store: Arc<CredentialStore> = zbus_secret_service_keyring_store::Store::new()?;
    Ok(store)
}

#[cfg(target_os = "macos")]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    let store: Arc<CredentialStore> = apple_native_keyring_store::keychain::Store::new()?;
    Ok(store)
}

#[cfg(target_os = "windows")]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    let store: Arc<CredentialStore> = windows_native_keyring_store::Store::new()?;
    Ok(store)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Err(KeyringError::NotSupportedByStore(
        "this platform has no credential store".to_owned(),
    ))
}

/// Stable messages per variant (5.1). The error enum is non-exhaustive, hence the catch-all.
/// Variants that carry secret bytes are never printed.
fn map_error(error: &KeyringError) -> CoreError {
    let message = match error {
        KeyringError::NoStorageAccess(detail) => {
            format!("the keychain is locked or access was denied ({detail})")
        }
        KeyringError::PlatformFailure(detail) => {
            format!("the keychain is unavailable ({detail})")
        }
        KeyringError::BadEncoding(_) | KeyringError::BadDataFormat(..) => {
            "the saved password could not be read".to_owned()
        }
        KeyringError::NoDefaultStore | KeyringError::NotSupportedByStore(_) => {
            "no keychain is available on this system".to_owned()
        }
        _ => "unexpected keychain failure".to_owned(),
    };
    CoreError::SecretStore(message)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use keyring_core::mock;

    use super::*;

    fn mock_store() -> (Arc<CredentialStore>, KeychainSecretStore) {
        let store: Arc<CredentialStore> = mock::Store::new().expect("mock store");
        (store.clone(), KeychainSecretStore::with_store(store))
    }

    fn secret(text: &str) -> SecretString {
        SecretString::from(text.to_owned())
    }

    fn inject_error(store: &Arc<CredentialStore>, id: ConnectionId, error: KeyringError) {
        let entry = store
            .build(SERVICE_NAME, &entry_user(id), None)
            .expect("entry");
        let credential: &mock::Cred = entry.as_any().downcast_ref().expect("mock credential");
        credential.set_error(error);
    }

    fn message(error: CoreError) -> String {
        match error {
            CoreError::SecretStore(message) => message,
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn get_without_entry_is_none() {
        let (_, keychain) = mock_store();
        assert!(keychain.get(ConnectionId(1)).expect("get").is_none());
    }

    #[test]
    fn set_then_get_round_trips() {
        let (_, keychain) = mock_store();
        keychain
            .set(ConnectionId(1), &secret("hunter2"))
            .expect("set");
        let found = keychain.get(ConnectionId(1)).expect("get").expect("some");
        assert_eq!(found.expose_secret(), "hunter2");
        assert!(keychain.get(ConnectionId(2)).expect("get").is_none());
    }

    #[test]
    fn set_overwrites() {
        let (_, keychain) = mock_store();
        keychain.set(ConnectionId(1), &secret("old")).expect("set");
        keychain.set(ConnectionId(1), &secret("new")).expect("set");
        let found = keychain.get(ConnectionId(1)).expect("get").expect("some");
        assert_eq!(found.expose_secret(), "new");
    }

    #[test]
    fn delete_removes_and_is_idempotent() {
        let (_, keychain) = mock_store();
        keychain.set(ConnectionId(1), &secret("x")).expect("set");
        keychain.delete(ConnectionId(1)).expect("delete");
        assert!(keychain.get(ConnectionId(1)).expect("get").is_none());
        keychain.delete(ConnectionId(1)).expect("delete again");
    }

    #[test]
    fn locked_store_maps_to_stable_message() {
        let (store, keychain) = mock_store();
        let detail = "dismissed".to_owned().into();
        inject_error(
            &store,
            ConnectionId(1),
            KeyringError::NoStorageAccess(detail),
        );
        let text = message(keychain.get(ConnectionId(1)).expect_err("error"));
        assert!(text.starts_with("the keychain is locked or access was denied"));
    }

    #[test]
    fn platform_failure_maps_to_stable_message() {
        let (store, keychain) = mock_store();
        let detail = "no bus".to_owned().into();
        inject_error(
            &store,
            ConnectionId(1),
            KeyringError::PlatformFailure(detail),
        );
        let text = message(
            keychain
                .set(ConnectionId(1), &secret("x"))
                .expect_err("error"),
        );
        assert!(text.starts_with("the keychain is unavailable"));
    }

    #[test]
    fn bad_encoding_never_leaks_bytes() {
        let (store, keychain) = mock_store();
        inject_error(
            &store,
            ConnectionId(1),
            KeyringError::BadEncoding(b"topsecret".to_vec()),
        );
        let text = message(keychain.get(ConnectionId(1)).expect_err("error"));
        assert_eq!(text, "the saved password could not be read");
    }

    #[test]
    fn unknown_errors_get_the_catch_all() {
        let (store, keychain) = mock_store();
        let error = KeyringError::TooLong("user".to_owned(), 1);
        inject_error(&store, ConnectionId(1), error);
        let text = message(keychain.delete(ConnectionId(1)).expect_err("error"));
        assert_eq!(text, "unexpected keychain failure");
    }
}
