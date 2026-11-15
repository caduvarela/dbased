use crate::{ColumnMeta, Value};

/// Row count ceiling of docs/PROJECT.md Part 10.
pub const MAX_ROWS: u64 = 5_000_000;

const COLUMNS: [(&str, &str, bool); 13] = [
    ("id", "bigint", false),
    ("flag", "boolean", false),
    ("score", "double precision", false),
    ("price", "numeric", false),
    ("label", "text", false),
    ("note", "text", true),
    ("blob", "bytea", false),
    ("created", "timestamp", false),
    ("day", "date", false),
    ("at", "time", false),
    ("uid", "uuid", false),
    ("doc", "jsonb", false),
    ("other", "unknown", false),
];

/// The fixed columns of every `select <n>` result; one per `Value` variant.
pub fn select_columns() -> Vec<ColumnMeta> {
    COLUMNS
        .iter()
        .map(|(name, type_name, nullable)| ColumnMeta {
            name: (*name).to_string(),
            type_name: (*type_name).to_string(),
            nullable: Some(*nullable),
        })
        .collect()
}

/// Computes one row from its index; nothing is stored, so 5M rows cost nothing.
pub fn select_row(index: u64) -> Vec<Value> {
    let month = index % 12 + 1;
    let day = index % 28 + 1;
    let hour = index % 24;
    let minute = index % 60;
    let second = (index / 60) % 60;
    let note = if index.is_multiple_of(5) {
        Value::Null
    } else {
        Value::Text(format!("note {index}"))
    };
    vec![
        Value::Int(index as i64),
        Value::Bool(index.is_multiple_of(2)),
        Value::Float(index as f64 * 0.5),
        Value::Decimal(format!("{index}.{:02}", index % 100)),
        Value::Text(format!("row {index}")),
        note,
        Value::Bytes(index.to_be_bytes().to_vec()),
        Value::Timestamp(format!(
            "2026-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"
        )),
        Value::Date(format!("2026-{month:02}-{day:02}")),
        Value::Time(format!("{hour:02}:{minute:02}:{second:02}")),
        Value::Uuid(format!("{index:08x}-0000-4000-8000-{:012x}", index * 7)),
        Value::Json(format!("{{\"index\":{index}}}")),
        Value::Unknown(format!("unknown {index}")),
    ]
}
