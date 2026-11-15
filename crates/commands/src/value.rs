/// A single database cell, normalized across drivers.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Decimal(String),
    Text(String),
    Bytes(Vec<u8>),
    Timestamp(String),
    Date(String),
    Time(String),
    Uuid(String),
    Json(String),
    Unknown(String),
}

/// Column metadata; travels once per result, ahead of the rows (docs/PROJECT.md 2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMeta {
    pub name: String,
    /// The database's own type name, as reported with the result.
    pub type_name: String,
    /// `None` when the protocol does not tell (text-protocol results do not).
    pub nullable: Option<bool>,
}
