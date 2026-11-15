use dbased_commands::Value;
use sqlx::mysql::MySqlRow;
use sqlx::{Column as _, Row as _, TypeInfo as _};

use crate::CoreError;

/// Types whose text-protocol cell is raw bytes rather than text (docs/PROJECT.md 2.5).
fn is_binary_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "BIT"
            | "BINARY"
            | "VARBINARY"
            | "TINYBLOB"
            | "BLOB"
            | "MEDIUMBLOB"
            | "LONGBLOB"
            | "GEOMETRY"
    )
}

/// Maps one server-rendered cell to the common model (docs/PROJECT.md 2.5).
/// A text that does not fit its variant falls back to `Unknown`, never to an
/// error or a rounded value. Zero-dates stay the text the server rendered.
pub(super) fn value_from_text(type_name: &str, text: String) -> Value {
    match type_name {
        "BOOLEAN" => match text.as_str() {
            "0" => Value::Bool(false),
            "1" => Value::Bool(true),
            _ => integer_or_unknown(text),
        },
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" | "TINYINT UNSIGNED"
        | "SMALLINT UNSIGNED" | "MEDIUMINT UNSIGNED" | "INT UNSIGNED" | "BIGINT UNSIGNED"
        | "YEAR" => integer_or_unknown(text),
        "FLOAT" | "DOUBLE" => match text.parse::<f64>() {
            Ok(number) if number.is_finite() => Value::Float(number),
            _ => Value::Unknown(text),
        },
        "DECIMAL" => Value::Decimal(text),
        "CHAR" | "VARCHAR" | "TINYTEXT" | "TEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM" | "SET" => {
            Value::Text(text)
        }
        "JSON" => Value::Json(text),
        "DATETIME" | "TIMESTAMP" => Value::Timestamp(text),
        "DATE" => Value::Date(text),
        "TIME" => Value::Time(text),
        _ => Value::Unknown(text),
    }
}

// @NOTE: `BIGINT UNSIGNED` above `i64::MAX` does not fit `Value::Int` and must not be rounded.
fn integer_or_unknown(text: String) -> Value {
    match text.parse::<i64>() {
        Ok(number) => Value::Int(number),
        Err(_) => Value::Unknown(text),
    }
}

/// Maps one raw cell. `None` is SQL NULL.
pub(super) fn value_from_bytes(type_name: &str, bytes: Option<&[u8]>) -> Value {
    let Some(bytes) = bytes else {
        return Value::Null;
    };
    if is_binary_type(type_name) {
        return Value::Bytes(bytes.to_vec());
    }
    // @NOTE: text that is not valid UTF-8 (a mismatched column charset) must still render.
    value_from_text(type_name, String::from_utf8_lossy(bytes).into_owned())
}

/// Approximate in-memory size of a row, for the memory cap (2.3).
pub(super) fn estimated_size(row: &[Value]) -> usize {
    row.iter()
        .map(|value| {
            std::mem::size_of::<Value>()
                + match value {
                    Value::Decimal(text)
                    | Value::Text(text)
                    | Value::Timestamp(text)
                    | Value::Date(text)
                    | Value::Time(text)
                    | Value::Uuid(text)
                    | Value::Json(text)
                    | Value::Unknown(text) => text.len(),
                    Value::Bytes(bytes) => bytes.len(),
                    Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) => 0,
                }
        })
        .sum()
}

/// Reads every cell of a text-protocol row. `raw_sql` results are always text
/// format, so the server's own rendering reaches the grid (docs/PROJECT.md 1.2 rule 2).
pub(super) fn convert_row(row: &MySqlRow) -> Result<Vec<Value>, CoreError> {
    let mut values = Vec::with_capacity(row.columns().len());
    for (index, column) in row.columns().iter().enumerate() {
        let bytes = row
            .try_get_unchecked::<Option<&[u8]>, _>(index)
            .map_err(|error| CoreError::Protocol(format!("cannot read column {index}: {error}")))?;
        values.push(value_from_bytes(column.type_info().name(), bytes));
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(type_name: &str, text: &str) -> Value {
        value_from_text(type_name, text.to_owned())
    }

    #[test]
    fn integers_map_to_int_and_overflow_to_unknown() {
        assert_eq!(map("TINYINT", "-7"), Value::Int(-7));
        assert_eq!(map("BIGINT", "9223372036854775807"), Value::Int(i64::MAX));
        assert_eq!(
            map("BIGINT UNSIGNED", "18446744073709551615"),
            Value::Unknown("18446744073709551615".into())
        );
        assert_eq!(map("INT UNSIGNED", "4294967295"), Value::Int(4_294_967_295));
        assert_eq!(map("YEAR", "2024"), Value::Int(2024));
    }

    #[test]
    fn booleans_only_for_zero_and_one() {
        assert_eq!(map("BOOLEAN", "1"), Value::Bool(true));
        assert_eq!(map("BOOLEAN", "0"), Value::Bool(false));
        assert_eq!(map("BOOLEAN", "5"), Value::Int(5));
    }

    #[test]
    fn exact_and_text_types_keep_the_server_text() {
        assert_eq!(map("DECIMAL", "0.10"), Value::Decimal("0.10".into()));
        assert_eq!(map("DOUBLE", "1.5"), Value::Float(1.5));
        assert_eq!(map("DOUBLE", "inf"), Value::Unknown("inf".into()));
        assert_eq!(map("ENUM", "happy"), Value::Text("happy".into()));
        assert_eq!(map("SET", "a,b"), Value::Text("a,b".into()));
        assert_eq!(map("JSON", "{\"a\": 1}"), Value::Json("{\"a\": 1}".into()));
        assert_eq!(map("TIME", "-838:59:59"), Value::Time("-838:59:59".into()));
    }

    #[test]
    fn zero_dates_are_rendered_faithfully() {
        assert_eq!(map("DATE", "0000-00-00"), Value::Date("0000-00-00".into()));
        assert_eq!(
            map("DATETIME", "0000-00-00 00:00:00"),
            Value::Timestamp("0000-00-00 00:00:00".into())
        );
    }

    #[test]
    fn binary_types_become_bytes_and_null_stays_null() {
        assert_eq!(
            value_from_bytes("BLOB", Some(&[0, 255])),
            Value::Bytes(vec![0, 255])
        );
        assert_eq!(value_from_bytes("BIT", Some(&[5])), Value::Bytes(vec![5]));
        assert_eq!(value_from_bytes("VARCHAR", None), Value::Null);
        assert_eq!(
            value_from_bytes("VARCHAR", Some(b"ab")),
            Value::Text("ab".into())
        );
        assert_eq!(
            value_from_bytes("VARCHAR", Some(&[0x61, 0xff])),
            Value::Text("a\u{fffd}".into())
        );
    }

    #[test]
    fn exotic_types_become_unknown() {
        assert_eq!(map("NULL", "x"), Value::Unknown("x".into()));
        assert_eq!(map("WHATEVER", "x"), Value::Unknown("x".into()));
    }

    #[test]
    fn estimated_size_grows_with_content() {
        let small = estimated_size(&[Value::Int(1)]);
        let large = estimated_size(&[Value::Text("x".repeat(100))]);
        assert!(large >= small + 100);
    }
}
