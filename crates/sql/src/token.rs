/// SQL dialect the scanner follows. Only the differences that change tokenization live here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    Postgres,
    MySql,
}

/// Half-open byte range into the scanned text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn contains(&self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    /// Returns an empty string when the span does not fit `source`, so callers never panic.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        source.get(self.start..self.end).unwrap_or("")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParameterKind {
    /// `?`
    Anonymous,
    /// `$1` (Postgres native placeholder)
    Numbered,
    /// `:name`
    Named,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Whitespace,
    /// `-- ...` (and `# ...` on MySQL), up to but excluding the newline.
    LineComment,
    BlockComment,
    /// MySQL `/*! ... */`: a comment for splitting, but the server executes its body.
    ExecutableComment,
    /// Single-quoted string, MySQL double-quoted string, Postgres `E'...'`.
    String,
    /// Postgres `$tag$ ... $tag$`.
    DollarString,
    /// Postgres `"..."` and MySQL `` `...` ``.
    QuotedIdentifier,
    Number,
    /// Keyword or unquoted identifier; the scanner keeps no keyword table.
    Word,
    Operator,
    Punctuation,
    Parameter(ParameterKind),
    /// MySQL custom statement delimiter set by a `DELIMITER` directive.
    Delimiter,
    /// A character that starts no known token.
    Unknown,
}

impl TokenKind {
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            TokenKind::Whitespace
                | TokenKind::LineComment
                | TokenKind::BlockComment
                | TokenKind::ExecutableComment
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        self.span.text(source)
    }
}
