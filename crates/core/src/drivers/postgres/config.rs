use std::time::Duration;

use secrecy::SecretString;
use sqlx::postgres::{PgConnectOptions, PgSslMode};

use crate::SessionLimits;

/// Transport security as the user chose it (docs/PROJECT.md 5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TlsMode {
    /// Plaintext only; an explicit, warned choice.
    Disable,
    /// TLS when offered, silent plaintext fallback; an explicit, warned choice.
    Prefer,
    #[default]
    Require,
}

#[derive(Debug, Clone)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: SecretString,
    pub database: String,
    pub tls: TlsMode,
    pub limits: SessionLimits,
    /// How long to keep reading after a cancel before giving up on the session (3.2).
    pub drain_timeout: Duration,
    pub health_check_timeout: Duration,
    /// Short on purpose: a cancel must not queue behind busy pool users.
    pub cancel_acquire_timeout: Duration,
    pub pool_max_connections: u32,
}

impl PostgresConfig {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        password: SecretString,
        database: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            user: user.into(),
            password,
            database: database.into(),
            tls: TlsMode::default(),
            limits: SessionLimits::default(),
            drain_timeout: Duration::from_secs(10),
            health_check_timeout: Duration::from_secs(5),
            cancel_acquire_timeout: Duration::from_secs(2),
            pool_max_connections: 5,
        }
    }

    /// Typed builder only: the password never sits in a formatted URL (2.2).
    pub(super) fn connect_options(&self) -> PgConnectOptions {
        use secrecy::ExposeSecret;

        PgConnectOptions::new_without_pgpass()
            .host(&self.host)
            .port(self.port)
            .username(&self.user)
            .password(self.password.expose_secret())
            .database(&self.database)
            .application_name("dbased")
            .ssl_mode(self.tls.into())
    }
}

impl From<TlsMode> for PgSslMode {
    fn from(mode: TlsMode) -> Self {
        match mode {
            TlsMode::Disable => PgSslMode::Disable,
            TlsMode::Prefer => PgSslMode::Prefer,
            TlsMode::Require => PgSslMode::Require,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_defaults_to_require() {
        assert_eq!(TlsMode::default(), TlsMode::Require);
        let config = PostgresConfig::new("h", 5432, "u", SecretString::from("p"), "d");
        assert!(matches!(
            config.connect_options().get_ssl_mode(),
            PgSslMode::Require
        ));
    }

    #[test]
    fn explicit_modes_are_mapped() {
        assert!(matches!(
            PgSslMode::from(TlsMode::Disable),
            PgSslMode::Disable
        ));
        assert!(matches!(
            PgSslMode::from(TlsMode::Prefer),
            PgSslMode::Prefer
        ));
    }

    #[test]
    fn debug_output_hides_the_password() {
        let config = PostgresConfig::new("h", 5432, "u", SecretString::from("hunter2"), "d");
        assert!(!format!("{config:?}").contains("hunter2"));
    }
}
