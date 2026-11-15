use serde::{Deserialize, Serialize};

/// A single database cell, normalized across drivers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
