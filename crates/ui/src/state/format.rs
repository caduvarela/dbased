use dbased_commands::Value;

/// Longest text shown in a cell; the full value stays in the Core.
const MAX_CELL_CHARS: usize = 512;
const MAX_BYTES_SHOWN: usize = 32;

/// A cell as the grid draws it: display-ready, nothing left to format at paint time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayCell {
    pub text: String,
    pub is_null: bool,
}

pub fn display_value(value: &Value) -> DisplayCell {
    match value {
        Value::Null => DisplayCell {
            text: "NULL".to_string(),
            is_null: true,
        },
        Value::Bool(flag) => text_cell(flag.to_string()),
        Value::Int(number) => text_cell(number.to_string()),
        Value::Float(number) => text_cell(number.to_string()),
        Value::Bytes(bytes) => text_cell(hex_preview(bytes)),
        Value::Decimal(text)
        | Value::Text(text)
        | Value::Timestamp(text)
        | Value::Date(text)
        | Value::Time(text)
        | Value::Uuid(text)
        | Value::Json(text)
        | Value::Unknown(text) => text_cell(single_line(text)),
    }
}

fn text_cell(text: String) -> DisplayCell {
    DisplayCell {
        text,
        is_null: false,
    }
}

fn hex_preview(bytes: &[u8]) -> String {
    let mut text = String::from("\\x");
    for byte in bytes.iter().take(MAX_BYTES_SHOWN) {
        text.push_str(&format!("{byte:02x}"));
    }
    if bytes.len() > MAX_BYTES_SHOWN {
        text.push('…');
    }
    text
}

fn single_line(text: &str) -> String {
    let mut shown: String = text
        .chars()
        .take(MAX_CELL_CHARS)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if text.chars().count() > MAX_CELL_CHARS {
        shown.push('…');
    }
    shown
}

/// Whether a column of this database type is right-aligned.
pub fn is_numeric_type(type_name: &str) -> bool {
    let lowered = type_name.to_ascii_lowercase();
    const NUMERIC_PREFIXES: [&str; 12] = [
        "int",
        "bigint",
        "smallint",
        "tinyint",
        "mediumint",
        "serial",
        "numeric",
        "decimal",
        "float",
        "double",
        "real",
        "money",
    ];
    NUMERIC_PREFIXES
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
}

/// `1234567` becomes `1,234,567`.
pub fn format_count(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (position, digit) in digits.chars().enumerate() {
        if position > 0 && (digits.len() - position).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_is_flagged_and_spelled_out() {
        let cell = display_value(&Value::Null);
        assert!(cell.is_null);
        assert_eq!(cell.text, "NULL");
    }

    #[test]
    fn text_with_newlines_stays_on_one_line() {
        let cell = display_value(&Value::Text("a\nb\tc".to_string()));
        assert_eq!(cell.text, "a b c");
        assert!(!cell.is_null);
    }

    #[test]
    fn long_text_is_cut() {
        let cell = display_value(&Value::Text("x".repeat(MAX_CELL_CHARS + 10)));
        assert_eq!(cell.text.chars().count(), MAX_CELL_CHARS + 1);
        assert!(cell.text.ends_with('…'));
    }

    #[test]
    fn bytes_show_a_hex_prefix() {
        assert_eq!(display_value(&Value::Bytes(vec![0, 255])).text, "\\x00ff");
        let long = display_value(&Value::Bytes(vec![1; 40])).text;
        assert!(long.ends_with('…'));
    }

    #[test]
    fn numeric_types_are_recognised() {
        assert!(is_numeric_type("bigint"));
        assert!(is_numeric_type("double precision"));
        assert!(is_numeric_type("NUMERIC"));
        assert!(!is_numeric_type("text"));
        assert!(!is_numeric_type("timestamp"));
    }

    #[test]
    fn counts_get_thousands_separators() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1,000");
        assert_eq!(format_count(5_000_000), "5,000,000");
    }
}
