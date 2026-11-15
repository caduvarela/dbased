#![allow(clippy::unwrap_used, clippy::expect_used)]

use dbased_commands::{ConnectionState, ConnectionSummary};

use super::*;

fn filled() -> ConnectionForm {
    let mut form = ConnectionForm::new();
    form.name = "Prod".to_string();
    form.user = "app".to_string();
    form
}

fn summary(password_saved: bool) -> ConnectionSummary {
    ConnectionSummary {
        id: ConnectionId(7),
        settings: ConnectionSettings {
            name: "Staging".to_string(),
            kind: DatabaseKind::MySql,
            host: "db.example".to_string(),
            port: 3307,
            user: "root".to_string(),
            database: "shop".to_string(),
            tls: TlsMode::Prefer,
        },
        password_saved,
        state: ConnectionState::Disconnected,
    }
}

#[test]
fn a_new_form_has_the_safe_defaults() {
    let form = ConnectionForm::new();
    assert_eq!(form.kind, DatabaseKind::Postgres);
    assert_eq!(form.port, "5432");
    assert_eq!(form.tls, TlsMode::Require);
    assert!(form.save_password);
    assert_eq!(form.editing(), None);
}

#[test]
fn required_fields_are_reported_together() {
    let mut form = ConnectionForm::new();
    form.host = "  ".to_string();
    let errors = form.errors();
    assert!(errors.name.is_some());
    assert!(errors.host.is_some());
    assert!(errors.user.is_some());
    assert!(errors.port.is_none());
    assert!(form.settings().is_err());
}

#[test]
fn the_database_is_optional() {
    let form = filled();
    let settings = form.settings().expect("valid");
    assert_eq!(settings.database, "");
}

#[test]
fn the_port_must_be_a_number_between_1_and_65535() {
    let mut form = filled();
    for bad in ["", "0", "65536", "-1", "abc", "54.3"] {
        form.port = bad.to_string();
        assert!(form.errors().port.is_some(), "{bad:?} must be rejected");
    }
    for good in ["1", "5432", " 65535 "] {
        form.port = good.to_string();
        assert!(form.errors().port.is_none(), "{good:?} must be accepted");
    }
}

#[test]
fn settings_are_trimmed() {
    let mut form = filled();
    form.name = "  Prod ".to_string();
    form.host = " db ".to_string();
    form.port = " 6543 ".to_string();
    let settings = form.settings().expect("valid");
    assert_eq!(settings.name, "Prod");
    assert_eq!(settings.host, "db");
    assert_eq!(settings.port, 6543);
}

#[test]
fn the_default_port_follows_the_driver() {
    assert_eq!(default_port(DatabaseKind::Postgres), 5432);
    assert_eq!(default_port(DatabaseKind::MySql), 3306);
    let mut form = filled();
    form.set_kind(DatabaseKind::MySql);
    assert_eq!(form.port, "3306");
    form.set_kind(DatabaseKind::Postgres);
    assert_eq!(form.port, "5432");
}

#[test]
fn a_port_the_user_chose_survives_a_driver_change() {
    let mut form = filled();
    form.port = "6000".to_string();
    form.set_kind(DatabaseKind::MySql);
    assert_eq!(form.port, "6000");
}

#[test]
fn an_empty_port_gets_the_new_drivers_default() {
    let mut form = filled();
    form.port.clear();
    form.set_kind(DatabaseKind::MySql);
    assert_eq!(form.port, "3306");
}

#[test]
fn the_password_update_follows_the_checkbox() {
    let mut form = filled();
    form.password = SecretString::from("hunter2".to_string());
    assert!(matches!(
        form.password_update(),
        PasswordUpdate::Save(secret) if secret.expose_secret() == "hunter2"
    ));
    form.save_password = false;
    assert!(matches!(form.password_update(), PasswordUpdate::Forget));
    form.save_password = true;
    form.password = SecretString::from(String::new());
    assert!(matches!(form.password_update(), PasswordUpdate::Keep));
}

#[test]
fn the_password_source_prefers_what_was_typed() {
    let mut form = ConnectionForm::from_summary(&summary(true));
    assert!(matches!(form.password_source(), PasswordSource::Saved));
    form.password = SecretString::from("typed".to_string());
    assert!(matches!(form.password_source(), PasswordSource::Typed(_)));
    let fresh = ConnectionForm::new();
    assert!(matches!(fresh.password_source(), PasswordSource::None));
    let unsaved = ConnectionForm::from_summary(&summary(false));
    assert!(matches!(unsaved.password_source(), PasswordSource::None));
}

#[test]
fn editing_starts_from_the_saved_settings() {
    let form = ConnectionForm::from_summary(&summary(true));
    assert_eq!(form.editing(), Some(ConnectionId(7)));
    assert_eq!(form.kind, DatabaseKind::MySql);
    assert_eq!(form.port, "3307");
    assert!(form.save_password);
    assert!(form.password_was_saved());
    assert_eq!(form.settings().expect("valid").database, "shop");
}

#[test]
fn debug_output_never_contains_the_password() {
    let mut form = filled();
    form.password = SecretString::from("hunter2".to_string());
    let printed = format!("{form:?} {:?}", form.password_update());
    assert!(!printed.contains("hunter2"));
}

#[test]
fn only_encrypted_modes_have_no_warning() {
    assert!(tls_warning(TlsMode::Require).is_none());
    assert!(tls_warning(TlsMode::Prefer).is_some());
    assert!(tls_warning(TlsMode::Disable).is_some());
}
