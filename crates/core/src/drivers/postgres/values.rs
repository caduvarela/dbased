use dbased_commands::Value;
use sqlx::postgres::PgRow;
use sqlx::{Column as _, Row as _, TypeInfo as _};

use crate::CoreError;

/// Parses Postgres `bytea` hex output (`\x` followed by hex digits).
pub(super) fn parse_bytea_hex(text: &str) -> Option<Vec<u8>> {
    let digits = text.strip_prefix("\\x")?.as_bytes();
    if digits.len() % 2 != 0 {
        return None;
    }
    let (pairs, _) = digits.as_chunks::<2>();
    pairs
        .iter()
        .map(|[high, low]| Some(hex_digit(*high)? << 4 | hex_digit(*low)?))
        .collect()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Maps one server-rendered cell to the common model (docs/PROJECT.md 2.5).
/// A text that does not fit its variant falls back to `Unknown`, never to an
/// error or a rounded value.
pub(super) fn value_from_text(type_name: &str, text: String) -> Value {
    match type_name.to_ascii_uppercase().as_str() {
        "BOOL" => match text.as_str() {
            "t" => Value::Bool(true),
            "f" => Value::Bool(false),
            _ => Value::Unknown(text),
        },
        "INT2" | "INT4" | "INT8" | "OID" => match text.parse::<i64>() {
            Ok(number) => Value::Int(number),
            Err(_) => Value::Unknown(text),
        },
        "FLOAT4" | "FLOAT8" => match text.parse::<f64>() {
            Ok(number) if number.is_finite() => Value::Float(number),
            _ => Value::Unknown(text),
        },
        "NUMERIC" => Value::Decimal(text),
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "\"CHAR\"" | "CHAR" => Value::Text(text),
        "BYTEA" => match parse_bytea_hex(&text) {
            Some(bytes) => Value::Bytes(bytes),
            None => Value::Unknown(text),
        },
        "UUID" => Value::Uuid(text),
        "JSON" | "JSONB" => Value::Json(text),
        "TIMESTAMP" | "TIMESTAMPTZ" => Value::Timestamp(text),
        "DATE" => Value::Date(text),
        "TIME" | "TIMETZ" => Value::Time(text),
        _ => Value::Unknown(text),
    }
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
/// format, so a string read is faithful for every type (docs/PROJECT.md 1.2 rule 3).
pub(super) fn convert_row(row: &PgRow) -> Result<Vec<Value>, CoreError> {
    let mut values = Vec::with_capacity(row.columns().len());
    for (index, column) in row.columns().iter().enumerate() {
        let text = match row.try_get_unchecked::<Option<String>, _>(index) {
            Ok(text) => text,
            Err(_) => lossy_text(row, index)?,
        };
        let value = match text {
            None => Value::Null,
            Some(text) => value_from_text(column.type_info().name(), text),
        };
        values.push(value);
    }
    Ok(values)
}

// @NOTE: server text that is not valid UTF-8 (a mismatched client encoding) must still render.
fn lossy_text(row: &PgRow, index: usize) -> Result<Option<String>, CoreError> {
    let bytes = row
        .try_get_unchecked::<Option<&[u8]>, _>(index)
        .map_err(|error| CoreError::Protocol(format!("cannot read column {index}: {error}")))?;
    Ok(bytes.map(|bytes| String::from_utf8_lossy(bytes).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(type_name: &str, text: &str) -> Value {
        value_from_text(type_name, text.to_owned())
    }

    #[test]
    fn bytea_hex_is_parsed() {
        assert_eq!(
            parse_bytea_hex("\\xdeadBEEF"),
            Some(vec![0xde, 0xad, 0xbe, 0xef])
        );
        assert_eq!(parse_bytea_hex("\\x"), Some(vec![]));
    }

    #[test]
    fn bytea_hex_rejects_malformed_input() {
        assert_eq!(parse_bytea_hex("\\xabc"), None);
        assert_eq!(parse_bytea_hex("\\xzz"), None);
        assert_eq!(parse_bytea_hex("abcd"), None);
    }

    #[test]
    fn scalar_types_map_to_their_variants() {
        assert_eq!(map("BOOL", "t"), Value::Bool(true));
        assert_eq!(map("INT2", "-7"), Value::Int(-7));
        assert_eq!(map("INT8", "9223372036854775807"), Value::Int(i64::MAX));
        assert_eq!(map("FLOAT8", "1.5"), Value::Float(1.5));
        assert_eq!(map("NUMERIC", "0.10"), Value::Decimal("0.10".into()));
        assert_eq!(map("VARCHAR", "x"), Value::Text("x".into()));
        assert_eq!(
            map("TIMESTAMPTZ", "2024-01-01 00:00:00+00"),
            Value::Timestamp("2024-01-01 00:00:00+00".into())
        );
        assert_eq!(map("DATE", "2024-01-01"), Value::Date("2024-01-01".into()));
        assert_eq!(
            map("TIMETZ", "01:00:00+00"),
            Value::Time("01:00:00+00".into())
        );
        assert_eq!(map("JSONB", "{\"a\": 1}"), Value::Json("{\"a\": 1}".into()));
        assert_eq!(map("BYTEA", "\\x00ff"), Value::Bytes(vec![0, 255]));
    }

    #[test]
    fn misfits_and_exotic_types_become_unknown() {
        assert_eq!(
            map("INT8", "not a number"),
            Value::Unknown("not a number".into())
        );
        assert_eq!(map("FLOAT8", "NaN"), Value::Unknown("NaN".into()));
        assert_eq!(map("FLOAT8", "Infinity"), Value::Unknown("Infinity".into()));
        assert_eq!(map("BYTEA", "\\xzz"), Value::Unknown("\\xzz".into()));
        assert_eq!(map("INTERVAL", "1 day"), Value::Unknown("1 day".into()));
        assert_eq!(map("INT4[]", "{1,2}"), Value::Unknown("{1,2}".into()));
        assert_eq!(map("?", "(1,2)"), Value::Unknown("(1,2)".into()));
        assert_eq!(map("MONEY", "$1.00"), Value::Unknown("$1.00".into()));
    }

    #[test]
    fn estimated_size_grows_with_content() {
        let small = estimated_size(&[Value::Int(1)]);
        let large = estimated_size(&[Value::Text("x".repeat(100))]);
        assert!(large >= small + 100);
    }
}
