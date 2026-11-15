//! Real-keychain round trip. Needs a running Secret Service, so it is Linux-only, behind the
//! `integration` feature, and skips (does not fail) when none is available.
#![cfg(all(feature = "integration", target_os = "linux"))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use dbased_commands::ConnectionId;
use dbased_core::{KeychainSecretStore, SecretStore};
use secrecy::{ExposeSecret, SecretString};

#[test]
fn round_trips_through_the_secret_service() {
    let keychain = match KeychainSecretStore::new() {
        Ok(keychain) => keychain,
        Err(error) => {
            eprintln!("skipped: no Secret Service available ({error})");
            return;
        }
    };
    // @NOTE: a throwaway id far from real ones; real entries are never touched.
    let id = ConnectionId(u64::MAX - u64::from(std::process::id()));
    let password = SecretString::from("integration-test-password".to_owned());

    // @NOTE: a locked keyring would prompt; treat any store failure as "not usable here".
    if let Err(error) = keychain.set(id, &password) {
        eprintln!("skipped: the Secret Service is not usable ({error})");
        return;
    }
    let found = keychain.get(id).expect("get").expect("saved password");
    assert_eq!(found.expose_secret(), "integration-test-password");
    keychain.delete(id).expect("delete");
    assert!(keychain.get(id).expect("get after delete").is_none());
}
