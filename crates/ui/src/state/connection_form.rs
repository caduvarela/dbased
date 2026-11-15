use dbased_commands::{
    ConnectionId, ConnectionSettings, ConnectionSummary, DatabaseKind, PasswordSource,
    PasswordUpdate, TlsMode,
};
use secrecy::{ExposeSecret as _, SecretString};

const POSTGRES_PORT: u16 = 5432;
const MYSQL_PORT: u16 = 3306;

pub fn default_port(kind: DatabaseKind) -> u16 {
    match kind {
        DatabaseKind::Postgres => POSTGRES_PORT,
        DatabaseKind::MySql => MYSQL_PORT,
    }
}

pub fn tls_label(tls: TlsMode) -> &'static str {
    match tls {
        TlsMode::Disable => "Disable",
        TlsMode::Prefer => "Prefer",
        TlsMode::Require => "Require",
    }
}

/// Shown next to the TLS choice when the connection may travel unencrypted
/// (docs/PROJECT.md 5.3: an unencrypted connection is never an accident).
pub fn tls_warning(tls: TlsMode) -> Option<&'static str> {
    match tls {
        TlsMode::Require => None,
        TlsMode::Prefer => Some("Prefer falls back to plaintext when the server has no TLS."),
        TlsMode::Disable => Some("Disable sends everything, including the password, unencrypted."),
    }
}

/// The connection dialog's fields as the user typed them. The password is a
/// `SecretString`, so `Debug` never prints it (docs/PROJECT.md 5.1).
#[derive(Debug, Clone)]
pub struct ConnectionForm {
    pub name: String,
    pub kind: DatabaseKind,
    pub host: String,
    /// Raw text: it may not be a number yet.
    pub port: String,
    pub database: String,
    pub user: String,
    pub password: SecretString,
    pub save_password: bool,
    pub tls: TlsMode,
    editing: Option<ConnectionId>,
    password_was_saved: bool,
}

/// One message per invalid field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormErrors {
    pub name: Option<&'static str>,
    pub host: Option<&'static str>,
    pub port: Option<&'static str>,
    pub user: Option<&'static str>,
}

impl FormErrors {
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.host.is_none() && self.port.is_none() && self.user.is_none()
    }
}

impl Default for ConnectionForm {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionForm {
    /// A new connection: Postgres on its default port, TLS required (docs/PROJECT.md
    /// 5.3), password saved unless the user unchecks it.
    pub fn new() -> Self {
        Self {
            name: String::new(),
            kind: DatabaseKind::Postgres,
            host: "localhost".to_string(),
            port: POSTGRES_PORT.to_string(),
            database: String::new(),
            user: String::new(),
            password: SecretString::from(String::new()),
            save_password: true,
            tls: TlsMode::Require,
            editing: None,
            password_was_saved: false,
        }
    }

    /// Edit an existing connection. The password is never sent back to the UI, so the
    /// field starts empty and an empty field means "keep what is saved".
    pub fn from_summary(summary: &ConnectionSummary) -> Self {
        let settings = &summary.settings;
        Self {
            name: settings.name.clone(),
            kind: settings.kind,
            host: settings.host.clone(),
            port: settings.port.to_string(),
            database: settings.database.clone(),
            user: settings.user.clone(),
            password: SecretString::from(String::new()),
            save_password: summary.password_saved,
            tls: settings.tls,
            editing: Some(summary.id),
            password_was_saved: summary.password_saved,
        }
    }

    pub fn editing(&self) -> Option<ConnectionId> {
        self.editing
    }

    pub fn password_was_saved(&self) -> bool {
        self.password_was_saved
    }

    /// Switches the driver. The port follows only while it is empty or still the old
    /// driver's default, so a port the user chose is never overwritten.
    pub fn set_kind(&mut self, kind: DatabaseKind) {
        if self.kind == kind {
            return;
        }
        let untouched =
            self.port.trim().is_empty() || self.port.trim() == default_port(self.kind).to_string();
        if untouched {
            self.port = default_port(kind).to_string();
        }
        self.kind = kind;
    }

    pub fn errors(&self) -> FormErrors {
        FormErrors {
            name: required(&self.name, "Name is required"),
            host: required(&self.host, "Host is required"),
            port: self.port_error(),
            user: required(&self.user, "User is required"),
        }
    }

    fn port_error(&self) -> Option<&'static str> {
        let text = self.port.trim();
        if text.is_empty() {
            return Some("Port is required");
        }
        match text.parse::<u16>() {
            Ok(port) if port > 0 => None,
            _ => Some("Port must be between 1 and 65535"),
        }
    }

    /// The settings to send, or what is wrong. The database is optional: both drivers
    /// have a default.
    pub fn settings(&self) -> Result<ConnectionSettings, FormErrors> {
        let errors = self.errors();
        if !errors.is_empty() {
            return Err(errors);
        }
        let port = self
            .port
            .trim()
            .parse::<u16>()
            .map_err(|_| errors.clone())?;
        Ok(ConnectionSettings {
            name: self.name.trim().to_string(),
            kind: self.kind,
            host: self.host.trim().to_string(),
            port,
            user: self.user.trim().to_string(),
            database: self.database.trim().to_string(),
            tls: self.tls,
        })
    }

    fn has_typed_password(&self) -> bool {
        !self.password.expose_secret().is_empty()
    }

    /// What `SaveConnection` does with the password: a typed one is stored when the box
    /// is checked, an empty one leaves what is stored alone, and unchecking forgets it.
    pub fn password_update(&self) -> PasswordUpdate {
        if !self.save_password {
            return PasswordUpdate::Forget;
        }
        if self.has_typed_password() {
            return PasswordUpdate::Save(self.password.clone());
        }
        PasswordUpdate::Keep
    }

    /// What `TestConnection` authenticates with.
    pub fn password_source(&self) -> PasswordSource {
        if self.has_typed_password() {
            return PasswordSource::Typed(self.password.clone());
        }
        if self.editing.is_some() && self.password_was_saved {
            return PasswordSource::Saved;
        }
        PasswordSource::None
    }
}

fn required(text: &str, message: &'static str) -> Option<&'static str> {
    text.trim().is_empty().then_some(message)
}

#[cfg(test)]
mod tests;
