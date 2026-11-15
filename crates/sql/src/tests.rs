use crate::*;
use Dialect::{MySql, Postgres};
use TokenKind as K;

fn significant(source: &str, dialect: Dialect) -> Vec<(TokenKind, String)> {
    tokenize(source, dialect)
        .into_iter()
        .filter(|t| t.kind != K::Whitespace)
        .map(|t| (t.kind, t.text(source).to_string()))
        .collect()
}

fn kinds(source: &str, dialect: Dialect) -> Vec<TokenKind> {
    significant(source, dialect)
        .into_iter()
        .map(|(k, _)| k)
        .collect()
}

fn split(source: &str, dialect: Dialect) -> Vec<&str> {
    split_statements(source, dialect)
        .iter()
        .map(|s| s.text(source))
        .collect()
}

fn params(source: &str, dialect: Dialect) -> Vec<String> {
    find_parameters(source, dialect)
        .iter()
        .map(|p| p.text(source).to_string())
        .collect()
}

fn assert_round_trip(source: &str, dialect: Dialect) {
    let tokens = tokenize(source, dialect);
    let rebuilt: String = tokens.iter().map(|t| t.text(source)).collect();
    assert_eq!(rebuilt, source, "{dialect:?}");
    let mut expected_start = 0;
    for token in &tokens {
        assert_eq!(token.span.start, expected_start);
        assert!(!token.span.is_empty());
        expected_start = token.span.end;
    }
    assert_eq!(expected_start, source.len());
}

// Row: nested block comments in Postgres only.
#[test]
fn block_comments_nest_only_on_postgres() {
    let source = "/* a /* b */ c */ x";
    assert_eq!(
        significant(source, Postgres),
        vec![
            (K::BlockComment, "/* a /* b */ c */".into()),
            (K::Word, "x".into())
        ]
    );
    assert_eq!(
        significant(source, MySql)[0],
        (K::BlockComment, "/* a /* b */".into())
    );
}

// Row: `#` comments in MySQL only.
#[test]
fn hash_comment_is_mysql_only() {
    assert_eq!(kinds("# hi\nx", MySql), vec![K::LineComment, K::Word]);
    assert_eq!(
        kinds("a # b", Postgres),
        vec![K::Word, K::Operator, K::Word]
    );
}

// Row: backtick identifiers in MySQL only.
#[test]
fn backtick_identifier_is_mysql_only() {
    assert_eq!(
        significant("`a``b`", MySql),
        vec![(K::QuotedIdentifier, "`a``b`".into())]
    );
    assert_eq!(
        kinds("`a`", Postgres),
        vec![K::Unknown, K::Word, K::Unknown]
    );
}

// Row: double quotes are identifiers on Postgres, strings on MySQL.
#[test]
fn double_quotes_by_dialect() {
    assert_eq!(kinds("\"a\"\"b\"", Postgres), vec![K::QuotedIdentifier]);
    assert_eq!(kinds("\"a\\\"b\"", MySql), vec![K::String]);
}

// Row: `E''` strings and backslashes in Postgres.
#[test]
fn postgres_escape_strings() {
    assert_eq!(
        significant(r"E'it\'s' x", Postgres),
        vec![(K::String, r"E'it\'s'".into()), (K::Word, "x".into())]
    );
    assert_eq!(
        significant(r"e'\\' x", Postgres)[0],
        (K::String, r"e'\\'".into())
    );
    // A plain string ends at a backslash-quote on Postgres.
    assert_eq!(
        significant(r"'a\' x", Postgres),
        vec![(K::String, r"'a\'".into()), (K::Word, "x".into())]
    );
}

// Row: MySQL backslash escapes in strings, doubled quotes on both.
#[test]
fn mysql_backslash_escapes() {
    assert_eq!(
        significant(r"'it\'s' x", MySql),
        vec![(K::String, r"'it\'s'".into()), (K::Word, "x".into())]
    );
    assert_eq!(kinds("'it''s' x", Postgres), vec![K::String, K::Word]);
    assert_eq!(kinds("'it''s' x", MySql), vec![K::String, K::Word]);
}

// Row: dollar quotes (Postgres), tags, unterminated, not on MySQL.
#[test]
fn dollar_quotes() {
    assert_eq!(
        significant("$$a;b$$ $t$x $$ y$t$ z", Postgres),
        vec![
            (K::DollarString, "$$a;b$$".into()),
            (K::DollarString, "$t$x $$ y$t$".into()),
            (K::Word, "z".into())
        ]
    );
    assert_eq!(kinds("$a$ never closed", Postgres), vec![K::DollarString]);
    assert_eq!(
        kinds("a$b$c", Postgres),
        vec![K::Word],
        "dollar inside an identifier"
    );
    assert_eq!(kinds("$$x$$", MySql), vec![K::Word]);
}

// Row: MySQL `--` needs trailing whitespace.
#[test]
fn mysql_double_dash_needs_whitespace() {
    assert_eq!(
        kinds("a --b", MySql),
        vec![K::Word, K::Operator, K::Operator, K::Word]
    );
    assert_eq!(kinds("a -- b", MySql), vec![K::Word, K::LineComment]);
    assert_eq!(kinds("a --", MySql), vec![K::Word, K::LineComment]);
    assert_eq!(kinds("a --b", Postgres), vec![K::Word, K::LineComment]);
}

// Row: MySQL executable comments.
#[test]
fn mysql_executable_comment() {
    assert_eq!(
        kinds("/*!50001 DROP TABLE t */ /* c */", MySql),
        vec![K::ExecutableComment, K::BlockComment]
    );
    assert_eq!(kinds("/*! x */", Postgres), vec![K::BlockComment]);
}

// Row: `::` casts.
#[test]
fn double_colon_is_a_cast_operator() {
    assert_eq!(
        significant("a::int", Postgres),
        vec![
            (K::Word, "a".into()),
            (K::Operator, "::".into()),
            (K::Word, "int".into())
        ]
    );
    assert!(params("a::int, b::text", Postgres).is_empty());
}

// Row: parameter markers, and where they are ignored.
#[test]
fn parameter_detection() {
    assert_eq!(params("select :a, ?, $1", Postgres), vec![":a", "?", "$1"]);
    assert_eq!(params("select :a, ?, $1", MySql), vec![":a", "?"]);
    assert!(params("select ':a ? $1' -- :b ?\n/* :c */", Postgres).is_empty());
    assert!(params("select $$ :a ? $1 $$, $t$ :b $t$", Postgres).is_empty());
    assert!(params("select \"col:a\"", Postgres).is_empty());
    assert!(params("select `x:b`", MySql).is_empty());
    assert!(params("select a[1:n], b[:n]", Postgres).is_empty());
    assert_eq!(params("select a[1:n], :x", Postgres), vec![":x"]);
    assert!(params("select @v := 1", MySql).is_empty());
    assert!(params("select 1 ? 2:3", MySql).iter().all(|p| p == "?"));
}

#[test]
fn jsonb_question_operators_are_not_parameters() {
    assert!(params("a ?| b, a ?& c", Postgres).is_empty());
    assert_eq!(params("a ? b", Postgres), vec!["?"]);
}

#[test]
fn parameter_names_and_kinds() {
    let source = "select :user_id, $12, ?";
    let found = find_parameters(source, Postgres);
    assert_eq!(found[0].kind, ParameterKind::Named);
    assert_eq!(found[0].name(source), Some("user_id"));
    assert_eq!(found[1].kind, ParameterKind::Numbered);
    assert_eq!(found[2].kind, ParameterKind::Anonymous);
    assert_eq!(found[2].name(source), None);
}

#[test]
fn numbers_operators_and_punctuation() {
    assert_eq!(
        significant("1.5e-3 .5 0x1F t1.c 1_000", Postgres)
            .iter()
            .map(|(k, _)| *k)
            .collect::<Vec<_>>(),
        vec![
            K::Number,
            K::Number,
            K::Number,
            K::Word,
            K::Punctuation,
            K::Word,
            K::Number
        ]
    );
    assert_eq!(
        kinds("a->>'k' >= <> (b), [c];", Postgres),
        vec![
            K::Word,
            K::Operator,
            K::String,
            K::Operator,
            K::Operator,
            K::Punctuation,
            K::Word,
            K::Punctuation,
            K::Punctuation,
            K::Punctuation,
            K::Word,
            K::Punctuation,
            K::Punctuation
        ]
    );
}

#[test]
fn unterminated_input_never_fails() {
    for source in ["'abc", "\"abc", "`abc", "/* abc", "$$ abc", "E'abc\\"] {
        for dialect in [Postgres, MySql] {
            assert_round_trip(source, dialect);
        }
    }
    assert_eq!(kinds("'abc; select 2", Postgres), vec![K::String]);
}

// Splitting.

#[test]
fn splits_on_top_level_semicolons() {
    assert_eq!(
        split("select 1; select 2;\n  select 3", Postgres),
        vec!["select 1", "select 2", "select 3"]
    );
}

#[test]
fn skips_empty_and_comment_only_statements() {
    assert_eq!(
        split(";; \n ; -- c\n;/* x */; select 1;", Postgres),
        vec!["select 1"]
    );
    assert!(split("", Postgres).is_empty());
    assert!(split("  -- only a comment", MySql).is_empty());
}

#[test]
fn ignores_semicolons_in_strings_comments_and_dollar_quotes() {
    let source =
        "select 'a;b'; select \"c;d\"; /* ; */ select 1 -- ;\n; select $$ ; $$; select $t$ ; $t$";
    assert_eq!(split(source, Postgres).len(), 5);
    assert_eq!(
        split("select 'a;b'; select `c;d`; # ;\nselect 1", MySql).len(),
        3
    );
}

#[test]
fn keeps_trailing_statement_without_semicolon() {
    let source = "select 1;\nselect 2";
    let statements = split_statements(source, Postgres);
    assert_eq!(statements.len(), 2);
    assert!(statements[0].terminator.is_some());
    assert!(statements[1].terminator.is_none());
    assert_eq!(statements[1].text(source), "select 2");
}

#[test]
fn statement_spans_are_trimmed_byte_offsets() {
    let source = "  select 'é';  ";
    let statements = split_statements(source, Postgres);
    assert_eq!(statements[0].span, Span { start: 2, end: 13 });
    assert_eq!(statements[0].terminator, Some(Span { start: 13, end: 14 }));
}

#[test]
fn unterminated_string_swallows_the_rest_into_one_statement() {
    assert_eq!(split("select 1; select 'oops; select 3", Postgres).len(), 2);
}

#[test]
fn mysql_compound_bodies_are_one_statement() {
    let procedure = "CREATE PROCEDURE p() BEGIN SELECT 1; IF x THEN SELECT 2; END IF; \
                     WHILE a DO SET a = 1; END WHILE; END;";
    let source = format!("{procedure} select 9");
    assert_eq!(
        split(&source, MySql),
        vec![procedure.trim_end_matches(';'), "select 9"]
    );
}

#[test]
fn mysql_case_statement_and_expression_close_correctly() {
    let source = "CREATE FUNCTION f() RETURNS INT BEGIN \
                  DECLARE x INT; SET x = CASE WHEN 1 THEN 2 ELSE 3 END; \
                  CASE x WHEN 2 THEN SELECT 1; END CASE; RETURN x; END; select 2";
    let statements = split(source, MySql);
    assert_eq!(statements.len(), 2);
    assert!(statements[0].ends_with("RETURN x; END"));
}

#[test]
fn mysql_trigger_with_definer_and_event_name_tables() {
    let source = "CREATE DEFINER=`root`@`localhost` TRIGGER t BEFORE INSERT ON x FOR EACH ROW \
                  BEGIN SET NEW.a = 1; SET NEW.b = 2; END; select 1";
    assert_eq!(split(source, MySql).len(), 2);
    // A table named like a routine kind must not open a body.
    assert_eq!(
        split("CREATE TABLE event (a int); select 1", MySql).len(),
        2
    );
}

#[test]
fn bare_begin_is_a_transaction_start() {
    assert_eq!(
        split("BEGIN; select 1; COMMIT;", Postgres),
        vec!["BEGIN", "select 1", "COMMIT"]
    );
    assert_eq!(split("BEGIN; select 1; COMMIT;", MySql).len(), 3);
}

#[test]
fn postgres_begin_atomic_body() {
    let function =
        "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT 2; END";
    let source = format!("{function}; select 3");
    assert_eq!(split(&source, Postgres), vec![function, "select 3"]);
    // A plain-language body in dollar quotes needs no tracking.
    assert_eq!(
        split("CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql; select 2", Postgres).len(),
        2
    );
}

#[test]
fn unclosed_body_keeps_everything_in_one_statement() {
    assert_eq!(
        split("CREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2;", MySql).len(),
        1
    );
}

#[test]
fn mysql_delimiter_directive() {
    let source = "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END$$\nDELIMITER ;\nselect 2;";
    assert_eq!(
        split(source, MySql),
        vec!["CREATE PROCEDURE p() BEGIN SELECT 1; END", "select 2"]
    );
}

#[test]
fn mysql_delimiter_with_slashes_and_inner_semicolons() {
    let source = "delimiter //\nselect 1; select 2//\nselect 3//\ndelimiter ;\nselect 4;";
    assert_eq!(
        split(source, MySql),
        vec!["select 1; select 2", "select 3", "select 4"]
    );
}

#[test]
fn delimiter_is_not_a_directive_mid_line_or_on_postgres() {
    assert_eq!(split("select delimiter; select 1", MySql).len(), 2);
    assert_eq!(
        split("DELIMITER //\nselect 1;", Postgres),
        vec!["DELIMITER //\nselect 1"]
    );
}

#[test]
fn delimiter_directive_never_appears_in_statements() {
    let source = "DELIMITER $$\nselect 1$$\nDELIMITER ;\n";
    let statements = split_statements(source, MySql);
    assert_eq!(statements.len(), 1);
    assert!(!statements[0].text(source).contains("DELIMITER"));
}

// Round trips.

const SAMPLES: [&str; 14] = [
    "select 1; -- c\nselect 'a''b', \"q\", `bt` /* x /* y */ */ from t where a::int = :p and b ? $1",
    "CREATE FUNCTION f() RETURNS int AS $body$ select 'é;' $body$ LANGUAGE sql;",
    "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END$$\nDELIMITER ;",
    "select E'a\\'b', 1.5e+3, 0xFF, a->>'k', x#>'{a}', a ?| b, arr[1:2], 日本語, ü",
    "/*! SET x=1 */; # c\nselect --1, - -2",
    "$",
    "$1$ x",
    ":",
    "\\ \u{b} \u{0}",
    "'",
    "/*",
    "--",
    "é'é",
    "select 'x\\",
];

#[test]
fn round_trip_on_samples_and_every_prefix() {
    for sample in SAMPLES {
        for dialect in [Postgres, MySql] {
            assert_round_trip(sample, dialect);
            for end in (0..=sample.len()).filter(|&i| sample.is_char_boundary(i)) {
                let prefix = &sample[..end];
                assert_round_trip(prefix, dialect);
                // Splitting and parameter scans must hold up on every half-typed prefix too.
                for statement in split_statements(prefix, dialect) {
                    assert!(!statement.text(prefix).is_empty());
                }
                find_parameters(prefix, dialect);
            }
        }
    }
}

#[test]
fn round_trip_on_pseudo_random_fragments() {
    let fragments = [
        "select",
        " ",
        "\n",
        ";",
        "'",
        "\"",
        "`",
        "$$",
        "$a$",
        "$1",
        "?",
        ":x",
        "::",
        "--",
        "# ",
        "/*",
        "*/",
        "\\",
        "é",
        "(",
        ")",
        "[",
        "]",
        "1",
        "1.5",
        "DELIMITER ;",
        "BEGIN",
        "END",
        "CASE",
        "CREATE PROCEDURE p()",
        "E'",
        "->>",
        "/*!",
    ];
    let mut state = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..3000 {
        let mut source = String::new();
        for _ in 0..(1 + state % 24) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            source.push_str(fragments[(state % fragments.len() as u64) as usize]);
        }
        for dialect in [Postgres, MySql] {
            assert_round_trip(&source, dialect);
            for statement in split_statements(&source, dialect) {
                assert!(statement.span.end <= source.len());
                assert!(source.is_char_boundary(statement.span.start));
            }
            find_parameters(&source, dialect);
        }
    }
}

fn statement_text_at(source: &str, cursor: usize) -> Option<&str> {
    let statements = split_statements(source, Postgres);
    statement_at(&statements, cursor).map(|statement| statement.text(source))
}

#[test]
fn statement_at_picks_the_statement_containing_the_cursor() {
    let source = "select 1; select 2;\nselect 3";
    assert_eq!(statement_text_at(source, 0), Some("select 1"));
    assert_eq!(statement_text_at(source, 4), Some("select 1"));
    assert_eq!(statement_text_at(source, 12), Some("select 2"));
    assert_eq!(statement_text_at(source, source.len()), Some("select 3"));
}

#[test]
fn statement_at_prefers_the_statement_before_a_cursor_in_a_gap_or_on_a_terminator() {
    let source = "select 1;\n\n  select 2";
    // On the terminator and right after it.
    assert_eq!(statement_text_at(source, 8), Some("select 1"));
    assert_eq!(statement_text_at(source, 9), Some("select 1"));
    // In the blank gap between the statements.
    assert_eq!(statement_text_at(source, 10), Some("select 1"));
    assert_eq!(statement_text_at(source, 11), Some("select 1"));
    // At the first character of the next one.
    assert_eq!(statement_text_at(source, 13), Some("select 2"));
}

#[test]
fn statement_at_falls_back_to_the_first_statement_after_the_cursor() {
    let source = "   \n\n  select 1;";
    assert_eq!(statement_text_at(source, 0), Some("select 1"));
    assert_eq!(statement_text_at(source, 3), Some("select 1"));
}

#[test]
fn statement_at_is_none_without_statements() {
    assert_eq!(statement_text_at("", 0), None);
    assert_eq!(statement_text_at("-- only a comment\n;", 3), None);
}

#[test]
fn statement_at_keeps_compound_bodies_whole() {
    let source =
        "create function f() returns int as $$ begin; select 1; end $$ language sql; select 2";
    let inside = source.find("select 1").unwrap_or(0);
    assert_eq!(
        statement_text_at(source, inside),
        Some("create function f() returns int as $$ begin; select 1; end $$ language sql")
    );
}
