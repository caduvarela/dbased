use crate::token::{Dialect, Token, TokenKind};

const ROUTINE_KINDS: [&str; 4] = ["PROCEDURE", "FUNCTION", "TRIGGER", "EVENT"];
const OTHER_OBJECT_KINDS: [&str; 10] = [
    "TABLE",
    "VIEW",
    "INDEX",
    "DATABASE",
    "SCHEMA",
    "SEQUENCE",
    "TYPE",
    "EXTENSION",
    "DOMAIN",
    "ROLE",
];
const OWN_END_KEYWORDS: [&str; 4] = ["IF", "LOOP", "WHILE", "REPEAT"];
// @NOTE: modifiers (`OR REPLACE`, `DEFINER = user`) sit between CREATE and the object kind.
const MAX_WORDS_BEFORE_OBJECT_KIND: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Detection {
    Start,
    AfterCreate(usize),
    Routine,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Nothing,
    End,
    Begin,
}

/// Tracks `BEGIN`/`CASE` ... `END` nesting inside one `CREATE PROCEDURE/FUNCTION/TRIGGER/EVENT`
/// statement so its inner semicolons do not split it. It is not a grammar: it only counts.
#[derive(Debug)]
pub(crate) struct BlockTracker {
    dialect: Dialect,
    detection: Detection,
    pending: Pending,
    depth: usize,
}

impl BlockTracker {
    pub(crate) fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            detection: Detection::Start,
            pending: Pending::Nothing,
            depth: 0,
        }
    }

    /// True while a compound body is open, counting a just-seen `END` as already closing.
    pub(crate) fn in_block(&self) -> bool {
        let closing = usize::from(self.pending == Pending::End);
        self.depth.saturating_sub(closing) > 0
    }

    /// Feeds one non-trivia token of the current statement.
    pub(crate) fn feed(&mut self, token: &Token, source: &str) {
        let word = (token.kind == TokenKind::Word).then(|| token.text(source));
        if self.resolve_pending(word) {
            return;
        }
        let Some(word) = word else {
            self.detect(None);
            return;
        };
        self.detect(Some(word));
        if self.detection != Detection::Routine {
            return;
        }
        if word.eq_ignore_ascii_case("BEGIN") {
            match self.dialect {
                Dialect::MySql => self.depth += 1,
                Dialect::Postgres => self.pending = Pending::Begin,
            }
        } else if word.eq_ignore_ascii_case("CASE") {
            self.depth += 1;
        } else if word.eq_ignore_ascii_case("END") {
            self.pending = Pending::End;
        }
    }

    /// Settles a pending `BEGIN`/`END` against the token that follows it.
    /// Returns true when that token was fully consumed by the decision.
    fn resolve_pending(&mut self, word: Option<&str>) -> bool {
        let pending = std::mem::replace(&mut self.pending, Pending::Nothing);
        let is = |expected: &str| word.is_some_and(|w| w.eq_ignore_ascii_case(expected));
        match pending {
            Pending::Nothing => false,
            // @NOTE: only `BEGIN ATOMIC` opens a Postgres body; a bare `BEGIN` starts a transaction.
            Pending::Begin if is("ATOMIC") => {
                self.depth += 1;
                true
            }
            Pending::Begin => false,
            Pending::End if OWN_END_KEYWORDS.iter().any(|keyword| is(keyword)) => true,
            Pending::End if is("CASE") => {
                self.depth = self.depth.saturating_sub(1);
                true
            }
            Pending::End => {
                self.depth = self.depth.saturating_sub(1);
                false
            }
        }
    }

    fn detect(&mut self, word: Option<&str>) {
        self.detection = match (self.detection, word) {
            (Detection::Start, Some(word)) if word.eq_ignore_ascii_case("CREATE") => {
                Detection::AfterCreate(0)
            }
            (Detection::Start, _) => Detection::Other,
            (Detection::AfterCreate(seen), word) => {
                let matches_kind = |kinds: &[&str]| {
                    word.is_some_and(|w| kinds.iter().any(|k| w.eq_ignore_ascii_case(k)))
                };
                if matches_kind(&ROUTINE_KINDS) {
                    Detection::Routine
                } else if matches_kind(&OTHER_OBJECT_KINDS) || seen >= MAX_WORDS_BEFORE_OBJECT_KIND
                {
                    Detection::Other
                } else {
                    Detection::AfterCreate(seen + 1)
                }
            }
            (state, _) => state,
        };
    }
}
