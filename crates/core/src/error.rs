use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("query failed: {0}")]
    Query(String),
    #[error("operation not supported by this driver: {0}")]
    Unsupported(&'static str),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
