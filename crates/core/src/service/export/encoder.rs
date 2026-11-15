//! CSV encoding of one row at a time, by hand: quoting is a few lines and the Core never needs
//! a reader (docs/PROJECT.md 3.3).

use dbased_commands::{ColumnMeta, CsvOptions, Value};

use crate::CoreError;

const QUOTE: u8 = b'"';
const LINE_END: &[u8] = b"\n";
const BYTE_ORDER_MARK: &[u8] = b"\xEF\xBB\xBF";

pub(super) struct CsvEncoder {
    delimiter: u8,
}

impl CsvEncoder {
    pub(super) fn new(options: &CsvOptions) -> Result<Self, CoreError> {
        let delimiter = u8::try_from(options.delimiter)
            .ok()
            .filter(|byte| byte.is_ascii() && !matches!(*byte, b'"' | b'\r' | b'\n'))
            .ok_or_else(|| {
                CoreError::Configuration(
                    "the delimiter must be one ASCII character other than a quote or a line break"
                        .into(),
                )
            })?;
        Ok(Self { delimiter })
    }

    pub(super) fn write_byte_order_mark(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(BYTE_ORDER_MARK);
    }

    pub(super) fn write_header(&self, columns: &[ColumnMeta], out: &mut Vec<u8>) {
        let names = columns.iter().map(|column| column.name.as_str());
        for (index, name) in names.enumerate() {
            self.write_separator(index, out);
            self.write_text(name, out);
        }
        out.extend_from_slice(LINE_END);
    }

    pub(super) fn write_row(&self, row: &[Value], out: &mut Vec<u8>) {
        for (index, value) in row.iter().enumerate() {
            self.write_separator(index, out);
            self.write_value(value, out);
        }
        out.extend_from_slice(LINE_END);
    }

    fn write_separator(&self, index: usize, out: &mut Vec<u8>) {
        if index > 0 {
            out.push(self.delimiter);
        }
    }

    fn write_value(&self, value: &Value, out: &mut Vec<u8>) {
        match value {
            // @NOTE: NULL is an unquoted empty field and an empty string is `""`, so a loader
            // that tells them apart (Postgres COPY ... CSV does) round-trips both.
            Value::Null => {}
            Value::Bool(flag) => out.extend_from_slice(if *flag { b"true" } else { b"false" }),
            Value::Int(number) => out.extend_from_slice(number.to_string().as_bytes()),
            Value::Float(number) => self.write_text(&float_text(*number), out),
            Value::Bytes(bytes) => self.write_text(&hex_text(bytes), out),
            Value::Decimal(text)
            | Value::Text(text)
            | Value::Timestamp(text)
            | Value::Date(text)
            | Value::Time(text)
            | Value::Uuid(text)
            | Value::Json(text)
            | Value::Unknown(text) => self.write_text(text, out),
        }
    }

    /// Quotes when the field has a delimiter, a quote or a line break, or when it is empty.
    /// Values are written as the server rendered them: no formula or whitespace "fixing".
    fn write_text(&self, text: &str, out: &mut Vec<u8>) {
        let needs_quotes = text.is_empty()
            || text
                .bytes()
                .any(|byte| byte == self.delimiter || matches!(byte, b'"' | b'\r' | b'\n'));
        if !needs_quotes {
            out.extend_from_slice(text.as_bytes());
            return;
        }
        out.push(QUOTE);
        for byte in text.bytes() {
            if byte == QUOTE {
                out.push(QUOTE);
            }
            out.push(byte);
        }
        out.push(QUOTE);
    }
}

/// The spelling Postgres uses for non-finite floats, so a re-import parses them.
fn float_text(number: f64) -> String {
    if number.is_nan() {
        return "NaN".to_owned();
    }
    if number.is_infinite() {
        let sign = if number < 0.0 { "-" } else { "" };
        return format!("{sign}Infinity");
    }
    number.to_string()
}

/// Postgres `bytea_output = hex` form, the one the grid shows too (docs/PROJECT.md 2.5).
fn hex_text(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(2 + bytes.len() * 2);
    text.push_str("\\x");
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0F)]));
    }
    text
}
