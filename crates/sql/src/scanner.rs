use crate::token::{Dialect, ParameterKind, Span, Token, TokenKind};

const THREE_CHAR_OPERATORS: [&str; 4] = ["->>", "#>>", "<=>", "!~*"];
const TWO_CHAR_OPERATORS: [&str; 20] = [
    "::", ":=", "<=", ">=", "<>", "!=", "||", "&&", "->", "#>", "=>", "<<", ">>", "@>", "<@", "?|",
    "?&", "!~", "~*", "@@",
];
const SINGLE_CHAR_OPERATORS: &[u8] = b"+-*/%<>=!~^&|@#";

/// Lossless, tolerant scanner: every byte of the input lands in exactly one token and
/// broken input (unterminated strings, comments) never fails, it runs to the end.
pub struct Scanner<'a> {
    source: &'a str,
    dialect: Dialect,
    position: usize,
    bracket_depth: usize,
    delimiter: Option<String>,
}

impl<'a> Scanner<'a> {
    pub fn new(source: &'a str, dialect: Dialect) -> Self {
        Self {
            source,
            dialect,
            position: 0,
            bracket_depth: 0,
            delimiter: None,
        }
    }

    pub fn position(&self) -> usize {
        self.position
    }

    /// Moves the scan position forward; used by the splitter to consume a `DELIMITER` line.
    pub fn skip_to(&mut self, position: usize) {
        self.position = position.clamp(self.position, self.source.len());
    }

    /// `None` restores the default, the plain `;` punctuation.
    pub fn set_delimiter(&mut self, delimiter: Option<String>) {
        self.delimiter = delimiter;
    }

    pub fn has_custom_delimiter(&self) -> bool {
        self.delimiter.is_some()
    }

    fn bytes(&self) -> &'a [u8] {
        self.source.as_bytes()
    }

    fn byte_at(&self, offset: usize) -> Option<u8> {
        self.bytes().get(offset).copied()
    }

    fn starts_with_at(&self, offset: usize, pattern: &str) -> bool {
        self.bytes()
            .get(offset..)
            .is_some_and(|rest| rest.starts_with(pattern.as_bytes()))
    }

    fn at_delimiter(&self, offset: usize) -> bool {
        match &self.delimiter {
            Some(delimiter) => self.starts_with_at(offset, delimiter),
            None => false,
        }
    }

    fn scan_token(&mut self) -> TokenKind {
        let start = self.position;
        if self.at_delimiter(start) {
            self.position += self.delimiter.as_ref().map_or(1, String::len);
            return TokenKind::Delimiter;
        }
        let byte = self.bytes()[start];
        let next = self.byte_at(start + 1);
        match byte {
            _ if byte.is_ascii_whitespace() => self.scan_whitespace(),
            b'-' if next == Some(b'-') && self.dashes_start_comment() => self.scan_line_comment(),
            b'#' if self.dialect == Dialect::MySql => self.scan_line_comment(),
            b'/' if next == Some(b'*') => self.scan_block_comment(),
            b'\'' => {
                self.scan_quoted(b'\'', self.dialect == Dialect::MySql);
                TokenKind::String
            }
            b'"' if self.dialect == Dialect::MySql => {
                self.scan_quoted(b'"', true);
                TokenKind::String
            }
            b'"' => {
                self.scan_quoted(b'"', false);
                TokenKind::QuotedIdentifier
            }
            b'`' if self.dialect == Dialect::MySql => {
                self.scan_quoted(b'`', false);
                TokenKind::QuotedIdentifier
            }
            b'$' if self.dialect == Dialect::Postgres => self.scan_dollar(),
            b'0'..=b'9' => self.scan_number(),
            b'.' if next.is_some_and(|b| b.is_ascii_digit()) => self.scan_number(),
            b'?' => self.scan_question_mark(),
            b':' => self.scan_colon(),
            _ if is_word_start(byte, self.dialect) => self.scan_word(),
            b'[' => {
                self.bracket_depth += 1;
                self.advance_punctuation()
            }
            b']' => {
                self.bracket_depth = self.bracket_depth.saturating_sub(1);
                self.advance_punctuation()
            }
            b',' | b';' | b'(' | b')' | b'.' | b'{' | b'}' => self.advance_punctuation(),
            _ if self.scan_operator() => TokenKind::Operator,
            _ => self.scan_unknown(),
        }
    }

    fn advance_punctuation(&mut self) -> TokenKind {
        self.position += 1;
        TokenKind::Punctuation
    }

    fn scan_whitespace(&mut self) -> TokenKind {
        while self
            .byte_at(self.position)
            .is_some_and(|b| b.is_ascii_whitespace())
        {
            self.position += 1;
        }
        TokenKind::Whitespace
    }

    // @NOTE: MySQL only treats `--` as a comment when whitespace or a control character follows.
    fn dashes_start_comment(&self) -> bool {
        if self.dialect == Dialect::Postgres {
            return true;
        }
        match self.byte_at(self.position + 2) {
            None => true,
            Some(byte) => byte <= b' ',
        }
    }

    fn scan_line_comment(&mut self) -> TokenKind {
        while self.byte_at(self.position).is_some_and(|b| b != b'\n') {
            self.position += 1;
        }
        TokenKind::LineComment
    }

    fn scan_block_comment(&mut self) -> TokenKind {
        let executable =
            self.dialect == Dialect::MySql && self.byte_at(self.position + 2) == Some(b'!');
        let nests = self.dialect == Dialect::Postgres;
        self.position += 2;
        let mut depth = 1usize;
        while depth > 0 && self.position < self.source.len() {
            if self.starts_with_at(self.position, "*/") {
                depth -= 1;
                self.position += 2;
            } else if nests && self.starts_with_at(self.position, "/*") {
                depth += 1;
                self.position += 2;
            } else {
                self.position += 1;
            }
        }
        self.position = self.position.min(self.source.len());
        if executable {
            TokenKind::ExecutableComment
        } else {
            TokenKind::BlockComment
        }
    }

    /// Consumes a quoted run starting at the opening quote; a doubled quote is an escape.
    fn scan_quoted(&mut self, quote: u8, backslash_escapes: bool) {
        self.position += 1;
        while let Some(byte) = self.byte_at(self.position) {
            let doubled_quote = byte == quote && self.byte_at(self.position + 1) == Some(quote);
            if (backslash_escapes && byte == b'\\') || doubled_quote {
                self.position += 2;
            } else if byte == quote {
                self.position += 1;
                return;
            } else {
                self.position += 1;
            }
        }
        self.position = self.source.len();
    }

    fn scan_dollar(&mut self) -> TokenKind {
        let start = self.position;
        let next = self.byte_at(start + 1);
        if next.is_some_and(|b| b.is_ascii_digit()) {
            self.position += 1;
            self.consume_while(|b| b.is_ascii_digit());
            return TokenKind::Parameter(ParameterKind::Numbered);
        }
        let Some(tag_end) = self.dollar_tag_end() else {
            self.position += 1;
            return TokenKind::Unknown;
        };
        let tag = &self.source[start..=tag_end];
        let body_start = tag_end + 1;
        self.position = match self.source[body_start..].find(tag) {
            Some(offset) => body_start + offset + tag.len(),
            None => self.source.len(),
        };
        TokenKind::DollarString
    }

    /// Index of the closing `$` of an opening `$tag$` (or `$$`) at the current position.
    fn dollar_tag_end(&self) -> Option<usize> {
        let mut index = self.position + 1;
        let first = self.byte_at(index)?;
        if first == b'$' {
            return Some(index);
        }
        if !(first.is_ascii_alphabetic() || first == b'_' || first >= 0x80) {
            return None;
        }
        while let Some(byte) = self.byte_at(index) {
            if byte == b'$' {
                return Some(index);
            }
            if !(byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80) {
                return None;
            }
            index += 1;
        }
        None
    }

    fn scan_number(&mut self) -> TokenKind {
        self.consume_while(|b| b.is_ascii_digit() || b == b'_');
        if self.byte_at(self.position) == Some(b'.')
            && self
                .byte_at(self.position + 1)
                .is_some_and(|b| b.is_ascii_digit())
        {
            self.position += 1;
            self.consume_while(|b| b.is_ascii_digit() || b == b'_');
        }
        self.consume_exponent();
        // @NOTE: trailing word characters keep `0x1F` and MySQL `1abc` identifiers in one token.
        self.consume_while(|b| b.is_ascii_alphanumeric() || b == b'_');
        TokenKind::Number
    }

    fn consume_exponent(&mut self) {
        if !matches!(self.byte_at(self.position), Some(b'e' | b'E')) {
            return;
        }
        let sign = matches!(self.byte_at(self.position + 1), Some(b'+' | b'-'));
        let digit_at = self.position + 1 + usize::from(sign);
        if self.byte_at(digit_at).is_some_and(|b| b.is_ascii_digit()) {
            self.position = digit_at;
            self.consume_while(|b| b.is_ascii_digit());
        }
    }

    fn scan_word(&mut self) -> TokenKind {
        let start = self.position;
        while let Some(byte) = self.byte_at(self.position) {
            if self.at_delimiter(self.position) || !is_word_continue(byte) {
                break;
            }
            self.position += 1;
        }
        let word = &self.source[start..self.position];
        let escape_prefix = self.dialect == Dialect::Postgres && word.eq_ignore_ascii_case("e");
        if escape_prefix && self.byte_at(self.position) == Some(b'\'') {
            self.scan_quoted(b'\'', true);
            return TokenKind::String;
        }
        TokenKind::Word
    }

    fn scan_question_mark(&mut self) -> TokenKind {
        let jsonb_operator = self.dialect == Dialect::Postgres
            && matches!(self.byte_at(self.position + 1), Some(b'|' | b'&'));
        if jsonb_operator {
            self.position += 2;
            return TokenKind::Operator;
        }
        self.position += 1;
        TokenKind::Parameter(ParameterKind::Anonymous)
    }

    fn scan_colon(&mut self) -> TokenKind {
        let next = self.byte_at(self.position + 1);
        if matches!(next, Some(b':' | b'=')) {
            self.position += 2;
            return TokenKind::Operator;
        }
        // @NOTE: inside `[...]` a colon is an array slice (`a[1:n]`), never a parameter.
        let starts_name = next.is_some_and(|b| b.is_ascii_alphabetic() || b == b'_');
        if starts_name && self.bracket_depth == 0 {
            self.position += 1;
            self.consume_while(|b| b.is_ascii_alphanumeric() || b == b'_');
            return TokenKind::Parameter(ParameterKind::Named);
        }
        self.position += 1;
        TokenKind::Punctuation
    }

    fn scan_operator(&mut self) -> bool {
        let candidates = THREE_CHAR_OPERATORS.iter().chain(TWO_CHAR_OPERATORS.iter());
        for operator in candidates {
            if self.starts_with_at(self.position, operator) {
                self.position += operator.len();
                return true;
            }
        }
        let is_single = self
            .byte_at(self.position)
            .is_some_and(|b| SINGLE_CHAR_OPERATORS.contains(&b));
        if is_single {
            self.position += 1;
        }
        is_single
    }

    fn scan_unknown(&mut self) -> TokenKind {
        let width = self.source[self.position..]
            .chars()
            .next()
            .map_or(1, char::len_utf8);
        self.position += width;
        TokenKind::Unknown
    }

    fn consume_while(&mut self, predicate: impl Fn(u8) -> bool) {
        while self.byte_at(self.position).is_some_and(&predicate) {
            self.position += 1;
        }
    }
}

impl Iterator for Scanner<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        if self.position >= self.source.len() {
            return None;
        }
        let start = self.position;
        let kind = self.scan_token();
        Some(Token {
            kind,
            span: Span {
                start,
                end: self.position,
            },
        })
    }
}

pub fn tokenize(source: &str, dialect: Dialect) -> Vec<Token> {
    Scanner::new(source, dialect).collect()
}

fn is_word_start(byte: u8, dialect: Dialect) -> bool {
    // @NOTE: Postgres identifiers cannot start with `$` (it opens parameters and dollar quotes).
    byte.is_ascii_alphabetic()
        || byte == b'_'
        || byte >= 0x80
        || (byte == b'$' && dialect == Dialect::MySql)
}

fn is_word_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte >= 0x80
}
