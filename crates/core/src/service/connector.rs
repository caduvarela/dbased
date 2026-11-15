use std::sync::Arc;

use async_trait::async_trait;
use dbased_commands::{ConnectionSettings, DatabaseKind, TlsMode};
use secrecy::SecretString;

use crate::drivers::postgres::{PostgresConfig, PostgresDriver, TlsMode as PostgresTlsMode};
use crate::{CoreError, Driver, DriverFactory};

/// Builds a driver from stored settings. Injected so tests can script the database side.
#[async_trait]
pub trait DriverConnector: Send + Sync {
    async fn connect(
        &self,
        settings: &ConnectionSettings,
        password: SecretString,
    ) -> Result<Arc<dyn Driver>, CoreError>;
}

/// The production connector.
pub struct DefaultConnector;

#[async_trait]
impl DriverConnector for DefaultConnector {
    async fn connect(
        &self,
        settings: &ConnectionSettings,
        password: SecretString,
    ) -> Result<Arc<dyn Driver>, CoreError> {
        match settings.kind {
            DatabaseKind::Postgres => {
                let mut config = PostgresConfig::new(
                    settings.host.clone(),
                    settings.port,
                    settings.user.clone(),
                    password,
                    settings.database.clone(),
                );
                config.tls = match settings.tls {
                    TlsMode::Disable => PostgresTlsMode::Disable,
                    TlsMode::Prefer => PostgresTlsMode::Prefer,
                    TlsMode::Require => PostgresTlsMode::Require,
                };
                let driver = PostgresDriver::connect(config).await?;
                Ok(Arc::from(driver))
            }
            DatabaseKind::MySql => Err(CoreError::NotImplemented("the MySQL driver")),
        }
    }
}
