use std::time::Duration;

use secrecy::SecretString;
use sqlx::mysql::{MySqlConnectOptions, MySqlSslMode};

use crate::SessionLimits;

/// Transport security as the user chose it (docs/PROJECT.md 5.3).
// @TODO: share one `TlsMode` with the Postgres driver once a third consumer needs it.
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
pub struct MysqlConfig {
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

impl MysqlConfig {
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
    pub(super) fn connect_options(&self) -> MySqlConnectOptions {
        use secrecy::ExposeSecret;

        // @NOTE: sqlx rewrites `sql_mode` and `time_zone` on connect by default. A SQL client must
        // show what the server would do for the user, so both are left alone.
        MySqlConnectOptions::new()
            .host(&self.host)
            .port(self.port)
            .username(&self.user)
            .password(self.password.expose_secret())
            .database(&self.database)
            .ssl_mode(self.tls.into())
            .pipes_as_concat(false)
            .no_engine_substitution(false)
            .timezone(None)
    }
}

impl From<TlsMode> for MySqlSslMode {
    fn from(mode: TlsMode) -> Self {
        match mode {
            TlsMode::Disable => MySqlSslMode::Disabled,
            TlsMode::Prefer => MySqlSslMode::Preferred,
            TlsMode::Require => MySqlSslMode::Required,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_defaults_to_require() {
        assert_eq!(TlsMode::default(), TlsMode::Require);
        let config = MysqlConfig::new("h", 3306, "u", SecretString::from("p"), "d");
        assert!(matches!(
            config.connect_options().get_ssl_mode(),
            MySqlSslMode::Required
        ));
    }

    #[test]
    fn explicit_modes_are_mapped() {
        assert!(matches!(
            MySqlSslMode::from(TlsMode::Disable),
            MySqlSslMode::Disabled
        ));
        assert!(matches!(
            MySqlSslMode::from(TlsMode::Prefer),
            MySqlSslMode::Preferred
        ));
    }

    #[test]
    fn debug_output_hides_the_password() {
        let config = MysqlConfig::new("h", 3306, "u", SecretString::from("hunter2"), "d");
        assert!(!format!("{config:?}").contains("hunter2"));
    }
}
