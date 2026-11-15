#![allow(clippy::unwrap_used, clippy::expect_used)]

use dbased_commands::{ColumnMeta, CsvOptions, Value};

use super::encoder::CsvEncoder;

fn encoder(delimiter: char) -> CsvEncoder {
    let options = CsvOptions {
        delimiter,
        ..CsvOptions::default()
    };
    CsvEncoder::new(&options).unwrap()
}

fn row_text(delimiter: char, row: &[Value]) -> String {
    let mut out = Vec::new();
    encoder(delimiter).write_row(row, &mut out);
    String::from_utf8(out).unwrap()
}

fn field(value: Value) -> String {
    row_text(',', &[value]).trim_end_matches('\n').to_owned()
}

#[test]
fn plain_text_is_not_quoted() {
    assert_eq!(field(Value::Text("hello world".into())), "hello world");
}

#[test]
fn escaping_matrix() {
    let cases = [
        ("a,b", "\"a,b\""),
        ("say \"hi\"", "\"say \"\"hi\"\"\""),
        ("\"", "\"\"\"\""),
        ("two\nlines", "\"two\nlines\""),
        ("crlf\r\nbreak", "\"crlf\r\nbreak\""),
        ("lone\rcr", "\"lone\rcr\""),
        ("", "\"\""),
        (" padded ", " padded "),
        ("semi;colon", "semi;colon"),
        ("=1+1", "=1+1"),
        ("accent \u{e9} \u{1F600}", "accent \u{e9} \u{1F600}"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            field(Value::Text(input.into())),
            expected,
            "input {input:?}"
        );
    }
}

#[test]
fn null_is_empty_and_empty_text_is_quoted() {
    assert_eq!(row_text(',', &[Value::Null, Value::Null]), ",\n");
    assert_eq!(
        row_text(',', &[Value::Null, Value::Text(String::new())]),
        ",\"\"\n"
    );
}

#[test]
fn the_delimiter_decides_what_gets_quoted() {
    assert_eq!(
        row_text(';', &[Value::Text("a;b".into()), Value::Text("c,d".into())]),
        "\"a;b\";c,d\n"
    );
    assert_eq!(
        row_text('\t', &[Value::Text("a\tb".into()), Value::Int(1)]),
        "\"a\tb\"\t1\n"
    );
    assert_eq!(
        row_text('|', &[Value::Text("a|b".into()), Value::Int(1)]),
        "\"a|b\"|1\n"
    );
}

#[test]
fn every_value_variant_has_a_faithful_text() {
    let cases = [
        (Value::Null, ""),
        (Value::Bool(true), "true"),
        (Value::Bool(false), "false"),
        (Value::Int(-42), "-42"),
        (Value::Int(i64::MIN), "-9223372036854775808"),
        (Value::Float(1.5), "1.5"),
        (Value::Float(1.0), "1"),
        (Value::Float(f64::NAN), "NaN"),
        (Value::Float(f64::INFINITY), "Infinity"),
        (Value::Float(f64::NEG_INFINITY), "-Infinity"),
        (
            Value::Decimal("12345678901234567890.123456789".into()),
            "12345678901234567890.123456789",
        ),
        (Value::Decimal("-0.10".into()), "-0.10"),
        (Value::Text("t".into()), "t"),
        (Value::Bytes(vec![0x00, 0xde, 0xAD, 0xff]), "\\x00deadff"),
        (Value::Bytes(Vec::new()), "\\x"),
        (
            Value::Timestamp("2026-11-15 10:30:00.123456+00".into()),
            "2026-11-15 10:30:00.123456+00",
        ),
        (Value::Date("2026-11-15".into()), "2026-11-15"),
        (Value::Time("10:30:00".into()), "10:30:00"),
        (
            Value::Uuid("550e8400-e29b-41d4-a716-446655440000".into()),
            "550e8400-e29b-41d4-a716-446655440000",
        ),
        (Value::Unknown("[1,5)".into()), "\"[1,5)\""),
    ];
    for (value, expected) in cases {
        assert_eq!(field(value.clone()), expected, "value {value:?}");
    }
}

#[test]
fn json_text_is_quoted_because_it_carries_commas_and_quotes() {
    assert_eq!(
        field(Value::Json("{\"a\": 1, \"b\": [1, 2]}".into())),
        "\"{\"\"a\"\": 1, \"\"b\"\": [1, 2]}\""
    );
    assert_eq!(field(Value::Json("[1]".into())), "[1]");
}

#[test]
fn unknown_text_with_a_comma_is_quoted() {
    assert_eq!(field(Value::Unknown("(1,2)".into())), "\"(1,2)\"");
}

#[test]
fn header_is_escaped_like_any_field() {
    let columns = ["id", "a,b", "say \"x\"", ""].map(|name| ColumnMeta {
        name: name.into(),
        type_name: "text".into(),
        nullable: None,
    });
    let mut out = Vec::new();
    encoder(',').write_header(&columns, &mut out);
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "id,\"a,b\",\"say \"\"x\"\"\",\"\"\n"
    );
}

#[test]
fn a_row_is_joined_by_the_delimiter_and_ends_with_a_line_feed() {
    let row = [Value::Int(1), Value::Text("x".into()), Value::Null];
    assert_eq!(row_text(',', &row), "1,x,\n");
}

#[test]
fn the_byte_order_mark_is_the_utf8_one() {
    let mut out = Vec::new();
    encoder(',').write_byte_order_mark(&mut out);
    assert_eq!(out, [0xEF, 0xBB, 0xBF]);
}

#[test]
fn unusable_delimiters_are_rejected() {
    for delimiter in ['"', '\n', '\r', '\u{e9}', '\u{1F600}'] {
        let options = CsvOptions {
            delimiter,
            ..CsvOptions::default()
        };
        assert!(
            CsvEncoder::new(&options).is_err(),
            "delimiter {delimiter:?}"
        );
    }
}
