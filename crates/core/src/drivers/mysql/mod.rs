//! MySQL driver (docs/PROJECT.md 2.1, 2.2, 3.2).

mod canceller;
mod config;
mod errors;
mod metadata;
mod session;
mod sql;
mod transaction;
mod values;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions};
use sqlx::{Connection, MySqlConnection};

use self::canceller::MysqlCanceller;
use self::errors::map_sqlx_error;
use self::session::{MysqlSession, SessionSettings};
use crate::{
    CancelHandle, Column, CoreError, Driver, DriverFactory, PageRequest, Routine, Schema, Session,
    SessionId, TableRef, ViewRef,
};

pub use self::config::{MysqlConfig, TlsMode};

/// @NOTE: must never derive `Debug`; `MySqlConnectOptions` holds the password (guarded by a test).
pub struct MysqlDriver {
    options: MySqlConnectOptions,
    pool: MySqlPool,
    canceller: Arc<MysqlCanceller>,
    settings: SessionSettings,
}

#[async_trait]
impl DriverFactory for MysqlDriver {
    type Config = MysqlConfig;

    async fn connect(config: MysqlConfig) -> Result<Box<dyn Driver>, CoreError> {
        let options = config.connect_options();
        // @NOTE: one direct connect first, because a pool retries and would hide the typed error.
        MySqlConnection::connect_with(&options)
            .await
            .map_err(map_sqlx_error)?
            .close()
            .await
            .ok();
        let pool = MySqlPoolOptions::new()
            .max_connections(config.pool_max_connections)
            .acquire_timeout(Duration::from_secs(10))
            .connect_lazy_with(options.clone());
        let canceller = Arc::new(MysqlCanceller::new(
            pool.clone(),
            options.clone(),
            config.cancel_acquire_timeout,
        ));
        Ok(Box::new(Self {
            options,
            pool,
            canceller,
            settings: SessionSettings::from_config(&config),
        }))
    }
}

#[async_trait]
impl Driver for MysqlDriver {
    async fn open_session(&self) -> Result<Box<dyn Session>, CoreError> {
        // @NOTE: sessions are opened outside the pool so session state never leaks into pooled work.
        let options = self.options.clone().statement_cache_capacity(0);
        let mut connection = MySqlConnection::connect_with(&options)
            .await
            .map_err(map_sqlx_error)?;
        let connection_id = sqlx::query_scalar::<_, u64>("SELECT CONNECTION_ID()")
            .fetch_one(&mut connection)
            .await
            .map_err(map_sqlx_error)?;
        let session_id = SessionId(connection_id);
        // @TODO: idle timeout and automatic reconnect for sessions (docs/PROJECT.md 2.2).
        Ok(Box::new(MysqlSession::new(
            session_id,
            connection,
            CancelHandle::new(session_id, self.canceller.clone()),
            self.settings.clone(),
        )))
    }

    async fn list_schemas(&self) -> Result<Vec<Schema>, CoreError> {
        metadata::list_schemas(&self.pool).await
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableRef>, CoreError> {
        metadata::list_tables(&self.pool, schema).await
    }

    async fn list_columns(&self, table: &TableRef) -> Result<Vec<Column>, CoreError> {
        metadata::list_columns(&self.pool, table).await
    }

    async fn list_views(&self, schema: &str) -> Result<Vec<ViewRef>, CoreError> {
        metadata::list_views(&self.pool, schema).await
    }

    async fn list_routines(&self, _schema: &str) -> Result<Vec<Routine>, CoreError> {
        Err(CoreError::NotImplemented("listing routines"))
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        quote_identifier(identifier)
    }

    fn paginate(&self, base_sql: &str, page: PageRequest) -> String {
        paginate(base_sql, page)
    }
}

fn quote_identifier(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

fn paginate(base_sql: &str, page: PageRequest) -> String {
    let base = base_sql.trim_end().trim_end_matches(';');
    format!(
        "SELECT * FROM ({base}) AS dbased_page LIMIT {} OFFSET {}",
        page.limit, page.offset
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time guard: the macro fails to build if the type ever implements `Debug`.
    trait AmbiguousIfDebug<A> {
        fn probe() {}
    }
    impl<T: ?Sized> AmbiguousIfDebug<()> for T {}
    impl<T: ?Sized + std::fmt::Debug> AmbiguousIfDebug<u8> for T {}
    macro_rules! assert_not_debug {
        ($type:ty) => {
            let _ = <$type as AmbiguousIfDebug<_>>::probe;
        };
    }

    #[test]
    fn types_holding_connect_options_do_not_implement_debug() {
        assert_not_debug!(MysqlDriver);
        assert_not_debug!(MysqlCanceller);
    }

    #[test]
    fn quotes_identifiers_and_doubles_embedded_backticks() {
        assert_eq!(quote_identifier("users"), "`users`");
        assert_eq!(quote_identifier("we`ird"), "`we``ird`");
    }

    #[test]
    fn paginate_wraps_the_query() {
        let page = PageRequest {
            limit: 200,
            offset: 400,
        };
        assert_eq!(
            paginate("select * from t;\n", page),
            "SELECT * FROM (select * from t) AS dbased_page LIMIT 200 OFFSET 400"
        );
    }
}
