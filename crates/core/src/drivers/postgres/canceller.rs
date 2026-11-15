use std::time::Duration;

use async_trait::async_trait;
use sqlx::postgres::{PgConnectOptions, PgPool};
use sqlx::{Connection, PgConnection};
use tokio::time::timeout;

use super::errors::map_sqlx_error;
use crate::{Canceller, CoreError, SessionId};

const ONE_OFF_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CANCEL_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Cancels from a connection that is never a session's own (docs/PROJECT.md 3.2).
pub(super) struct PostgresCanceller {
    pool: PgPool,
    options: PgConnectOptions,
    acquire_timeout: Duration,
}

impl PostgresCanceller {
    pub(super) fn new(pool: PgPool, options: PgConnectOptions, acquire_timeout: Duration) -> Self {
        Self {
            pool,
            options,
            acquire_timeout,
        }
    }

    async fn cancel_through_pool(&self, backend_pid: i32) -> Result<bool, CoreError> {
        let acquired = timeout(self.acquire_timeout, self.pool.acquire()).await;
        let Ok(Ok(mut connection)) = acquired else {
            return self.cancel_through_one_off_connection(backend_pid).await;
        };
        match run_cancel(&mut connection, backend_pid).await {
            Ok(cancelled) => Ok(cancelled),
            Err(error) => {
                // @NOTE: a pool connection that ended abnormally is never returned to the pool.
                connection.detach().close_hard().await.ok();
                Err(error)
            }
        }
    }

    async fn cancel_through_one_off_connection(&self, backend_pid: i32) -> Result<bool, CoreError> {
        let connecting = PgConnection::connect_with(&self.options);
        let mut connection = timeout(ONE_OFF_CONNECT_TIMEOUT, connecting)
            .await
            .map_err(|_| CoreError::Timeout("opening a cancel connection"))?
            .map_err(map_sqlx_error)?;
        let result = run_cancel(&mut connection, backend_pid).await;
        connection.close().await.ok();
        result
    }
}

async fn run_cancel(connection: &mut PgConnection, backend_pid: i32) -> Result<bool, CoreError> {
    let query = sqlx::query_scalar::<_, bool>("SELECT pg_cancel_backend($1)")
        .bind(backend_pid)
        .fetch_one(connection);
    timeout(CANCEL_QUERY_TIMEOUT, query)
        .await
        .map_err(|_| CoreError::Timeout("sending the cancel"))?
        .map_err(map_sqlx_error)
}

#[async_trait]
impl Canceller for PostgresCanceller {
    async fn cancel(&self, session: SessionId) -> Result<(), CoreError> {
        let backend_pid = i32::try_from(session.0)
            .map_err(|_| CoreError::CancelFailed("session id is not a Postgres pid".into()))?;
        let signalled = self.cancel_through_pool(backend_pid).await?;
        if !signalled {
            return Err(CoreError::CancelFailed(
                "the server did not signal the backend (already gone or not permitted)".into(),
            ));
        }
        Ok(())
    }
}
