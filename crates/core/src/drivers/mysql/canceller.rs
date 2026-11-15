use std::time::Duration;

use async_trait::async_trait;
use sqlx::mysql::{MySqlConnectOptions, MySqlPool};
use sqlx::{Connection, MySqlConnection};
use tokio::time::timeout;

use super::errors::{NO_SUCH_THREAD, error_number, map_sqlx_error};
use crate::{Canceller, CoreError, SessionId};

const ONE_OFF_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CANCEL_QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const ONE_OFF_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// Cancels from a connection that is never a session's own (docs/PROJECT.md 3.2).
///
/// @NOTE: must never derive `Debug`; `MySqlConnectOptions` holds the password.
pub(super) struct MysqlCanceller {
    pool: MySqlPool,
    options: MySqlConnectOptions,
    acquire_timeout: Duration,
}

impl MysqlCanceller {
    pub(super) fn new(
        pool: MySqlPool,
        options: MySqlConnectOptions,
        acquire_timeout: Duration,
    ) -> Self {
        Self {
            pool,
            options,
            acquire_timeout,
        }
    }

    async fn cancel_through_pool(&self, connection_id: u64) -> Result<bool, CoreError> {
        let acquired = timeout(self.acquire_timeout, self.pool.acquire()).await;
        let Ok(Ok(mut connection)) = acquired else {
            return self.cancel_through_one_off_connection(connection_id).await;
        };
        match run_cancel(&mut connection, connection_id).await {
            Ok(cancelled) => Ok(cancelled),
            Err(pool_error) => {
                // @NOTE: a pool connection that ended abnormally is never returned to the pool.
                connection.detach().close_hard().await.ok();
                // A broken pooled connection must not decide the outcome: retry on a fresh one.
                self.cancel_through_one_off_connection(connection_id)
                    .await
                    .map_err(|_| pool_error)
            }
        }
    }

    async fn cancel_through_one_off_connection(
        &self,
        connection_id: u64,
    ) -> Result<bool, CoreError> {
        let connecting = MySqlConnection::connect_with(&self.options);
        let mut connection = timeout(ONE_OFF_CONNECT_TIMEOUT, connecting)
            .await
            .map_err(|_| CoreError::Timeout("opening a cancel connection"))?
            .map_err(map_sqlx_error)?;
        let result = run_cancel(&mut connection, connection_id).await;
        timeout(ONE_OFF_CLOSE_TIMEOUT, connection.close())
            .await
            .ok();
        result
    }
}

/// `Ok(false)` means the server no longer knows that connection id.
async fn run_cancel(
    connection: &mut MySqlConnection,
    connection_id: u64,
) -> Result<bool, CoreError> {
    // @NOTE: `KILL` accepts a bound parameter, so the statement stays a static string and the
    // id never becomes SQL text.
    let query = sqlx::query("KILL QUERY ?")
        .bind(connection_id)
        .execute(connection);
    match timeout(CANCEL_QUERY_TIMEOUT, query)
        .await
        .map_err(|_| CoreError::Timeout("sending the cancel"))?
    {
        Ok(_) => Ok(true),
        Err(sqlx::Error::Database(error))
            if error_number(error.as_ref()) == Some(NO_SUCH_THREAD) =>
        {
            Ok(false)
        }
        Err(error) => Err(map_sqlx_error(error)),
    }
}

#[async_trait]
impl Canceller for MysqlCanceller {
    async fn cancel_backend(&self, session: SessionId) -> Result<(), CoreError> {
        let signalled = self.cancel_through_pool(session.0).await?;
        if !signalled {
            return Err(CoreError::CancelFailed(
                "the server does not know this connection id (already gone or not permitted)"
                    .into(),
            ));
        }
        Ok(())
    }
}
