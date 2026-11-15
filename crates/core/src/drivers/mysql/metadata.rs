use sqlx::mysql::MySqlPool;

use super::errors::map_sqlx_error;
use crate::{Column, CoreError, Schema, TableRef, ViewRef};

// @NOTE: every information_schema column is cast to CHAR: the catalog reports some of them as
// binary strings, which sqlx refuses to read as text.

/// Databases that belong to the server, hidden like Postgres' catalogs.
pub(super) async fn list_schemas(pool: &MySqlPool) -> Result<Vec<Schema>, CoreError> {
    let names = sqlx::query_scalar::<_, String>(
        "SELECT CAST(SCHEMA_NAME AS CHAR) FROM information_schema.SCHEMATA \
         WHERE SCHEMA_NAME NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys') \
         ORDER BY SCHEMA_NAME",
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx_error)?;
    Ok(names.into_iter().map(|name| Schema { name }).collect())
}

pub(super) async fn list_tables(
    pool: &MySqlPool,
    schema: &str,
) -> Result<Vec<TableRef>, CoreError> {
    let names = sqlx::query_scalar::<_, String>(
        "SELECT CAST(TABLE_NAME AS CHAR) FROM information_schema.TABLES \
         WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME",
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

pub(super) async fn list_views(pool: &MySqlPool, schema: &str) -> Result<Vec<ViewRef>, CoreError> {
    let names = sqlx::query_scalar::<_, String>(
        "SELECT CAST(TABLE_NAME AS CHAR) FROM information_schema.TABLES \
         WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'VIEW' ORDER BY TABLE_NAME",
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
    pool: &MySqlPool,
    table: &TableRef,
) -> Result<Vec<Column>, CoreError> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT CAST(COLUMN_NAME AS CHAR), CAST(COLUMN_TYPE AS CHAR), CAST(IS_NULLABLE AS CHAR) \
         FROM information_schema.COLUMNS \
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
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
            nullable: nullable == "YES",
        })
        .collect())
}
