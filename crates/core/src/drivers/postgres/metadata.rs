use sqlx::postgres::PgPool;

use super::errors::map_sqlx_error;
use crate::{Column, CoreError, Schema, TableRef, ViewRef};

pub(super) async fn list_schemas(pool: &PgPool) -> Result<Vec<Schema>, CoreError> {
    let names = sqlx::query_scalar::<_, String>(
        "SELECT nspname FROM pg_namespace \
         WHERE nspname <> 'information_schema' AND nspname NOT LIKE 'pg\\_%' \
         ORDER BY nspname",
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx_error)?;
    Ok(names.into_iter().map(|name| Schema { name }).collect())
}

pub(super) async fn list_tables(pool: &PgPool, schema: &str) -> Result<Vec<TableRef>, CoreError> {
    let names = sqlx::query_scalar::<_, String>(
        "SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relkind IN ('r', 'p') ORDER BY c.relname",
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx_error)?;
    Ok(names
        .into_iter()
        .map(|name| TableRef {
            schema: schema.to_owned(),
            name,
        })
        .collect())
}

pub(super) async fn list_views(pool: &PgPool, schema: &str) -> Result<Vec<ViewRef>, CoreError> {
    let names = sqlx::query_scalar::<_, String>(
        "SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relkind IN ('v', 'm') ORDER BY c.relname",
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx_error)?;
    Ok(names
        .into_iter()
        .map(|name| ViewRef {
            schema: schema.to_owned(),
            name,
        })
        .collect())
}

pub(super) async fn list_columns(
    pool: &PgPool,
    table: &TableRef,
) -> Result<Vec<Column>, CoreError> {
    let rows = sqlx::query_as::<_, (String, String, bool)>(
        "SELECT a.attname, format_type(a.atttypid, a.atttypmod), NOT a.attnotnull \
         FROM pg_attribute a \
         JOIN pg_class c ON c.oid = a.attrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
         ORDER BY a.attnum",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx_error)?;
    Ok(rows
        .into_iter()
        .map(|(name, data_type, nullable)| Column {
            name,
            data_type,
            nullable,
        })
        .collect())
}
