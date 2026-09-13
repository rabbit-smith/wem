//! Map Wwise setup 10-bit book IDs to installed static codebook tables
//! (Python: `profiles/book_ids.py`).
//!
//! The Wwise 2013 codebook registry assigns IDs contiguously:
//!   t97  -> IDs 0 .. 96
//!   t219 -> IDs 97 .. 315
//!   t282 -> IDs 316 .. 597
//!
//! Extend the registry when another installed profile requires an
//! additional table.

use wem_vorbis::codebook::CodebookRow;

use crate::error::ProfileError;
use crate::resources::ResourceRef;

/// Number of rows in the installed t97 table.
pub const T97_COUNT: usize = 97;
/// Number of rows in the installed t219 table.
pub const T219_COUNT: usize = 219;
/// Number of rows in the installed t282 table (2ch/48k residue books).
pub const T282_COUNT: usize = 282;

const BOOK_COUNTS: &[(&str, usize)] = &[
    ("t97", T97_COUNT),
    ("t219", T219_COUNT),
    ("t282", T282_COUNT),
];

/// One loaded, checksum-verified decoded book table.
#[derive(Debug, Clone)]
pub struct BookTable {
    name: &'static str,
    rows: Vec<CodebookRow>,
}

impl BookTable {
    /// Load one checked installed table (Python `load_book_table`).
    pub fn load(name: &str, ref_: &ResourceRef) -> Result<Self, ProfileError> {
        let (static_name, expected_count) = BOOK_COUNTS
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .ok_or_else(|| ProfileError::UnknownBookTable {
                table: name.to_string(),
            })?;
        let payload = ref_.read_json()?;
        let rows_value = payload
            .as_array()
            .ok_or_else(|| ProfileError::BookTableMalformed {
                table: name.to_string(),
                expected_count: *expected_count,
            })?;
        if rows_value.len() != *expected_count || rows_value.iter().any(|row| !row.is_object()) {
            return Err(ProfileError::BookTableMalformed {
                table: name.to_string(),
                expected_count: *expected_count,
            });
        }
        let mut rows = Vec::with_capacity(rows_value.len());
        for row in rows_value {
            rows.push(parse_row(name, row)?);
        }
        Ok(Self {
            name: static_name,
            rows,
        })
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn rows(&self) -> &[CodebookRow] {
        &self.rows
    }
}

/// Loose `int(value)`-style conversion: integers pass through, finite JSON
/// floats truncate toward zero (mirroring Python `int()` on numbers).
fn loose_int(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(n) if n.is_i64() => n.as_i64(),
        serde_json::Value::Number(n) if n.is_u64() => {
            n.as_u64().and_then(|v| i64::try_from(v).ok())
        }
        serde_json::Value::Number(n) if n.is_f64() => {
            n.as_f64().and_then(|v| v.is_finite().then_some(v as i64))
        }
        _ => None,
    }
}

/// `int(value) or default` semantics for the book rows' optional fields:
/// missing/null/falsy (0, false) values yield the default.
fn int_or(value: Option<&serde_json::Value>, default: i64) -> i64 {
    match value {
        None | Some(serde_json::Value::Null) => default,
        Some(serde_json::Value::Bool(true)) => 1,
        Some(serde_json::Value::Bool(false)) => default,
        Some(other) => loose_int(other).filter(|v| *v != 0).unwrap_or(default),
    }
}

fn parse_row(table: &str, row: &serde_json::Value) -> Result<CodebookRow, ProfileError> {
    let map = row.as_object().expect("checked to be an object");
    let required = move |field: &'static str| -> Result<&serde_json::Value, ProfileError> {
        map.get(field)
            .ok_or_else(move || ProfileError::BookRowMissingField {
                table: table.to_string(),
                field,
            })
    };
    let dim = loose_int(required("dim")?).ok_or_else(|| ProfileError::BookRowMissingField {
        table: table.to_string(),
        field: "dim",
    })?;
    let entries =
        loose_int(required("entries")?).ok_or_else(|| ProfileError::BookRowMissingField {
            table: table.to_string(),
            field: "entries",
        })?;

    let lengthlist = match map.get("lengthlist") {
        Some(serde_json::Value::Array(items)) => Some(
            items
                .iter()
                .map(|v| {
                    loose_int(v).ok_or_else(|| ProfileError::BookRowMissingField {
                        table: table.to_string(),
                        field: "lengthlist",
                    })
                })
                .collect::<Result<_, _>>()?,
        ),
        Some(serde_json::Value::Null) | None => None,
        Some(_) => {
            return Err(ProfileError::BookRowMissingField {
                table: table.to_string(),
                field: "lengthlist",
            })
        }
    };

    let quantlist = match map.get("quantlist") {
        Some(serde_json::Value::Array(items)) => Some(
            items
                .iter()
                .map(|v| {
                    loose_int(v).ok_or_else(|| ProfileError::BookRowMissingField {
                        table: table.to_string(),
                        field: "quantlist",
                    })
                })
                .collect::<Result<_, _>>()?,
        ),
        Some(serde_json::Value::Null) | None => None,
        Some(_) => {
            return Err(ProfileError::BookRowMissingField {
                table: table.to_string(),
                field: "quantlist",
            })
        }
    };

    Ok(CodebookRow {
        i: map.get("i").and_then(loose_int),
        dim,
        entries,
        lengthlist,
        maptype: int_or(map.get("maptype"), 0),
        q_min: int_or(map.get("q_min"), 0),
        q_delta: int_or(map.get("q_delta"), 0),
        q_quant: int_or(map.get("q_quant"), 0),
        q_sequencep: int_or(map.get("q_sequencep"), 0),
        quantlist,
        quantvals: map.get("quantvals").and_then(loose_int),
    })
}

/// The installed book tables (Python `tables` mapping).
///
/// The 10-bit book_id encoding reserves fixed ranges for each table, but a
/// profile carries only the tables its setup references: the t97 floor table
/// is installed for every profile; t219 (residue) is used by the 6ch profile
/// and t282 (residue) by the 2ch/48k profile.
#[derive(Debug, Clone)]
pub struct BookTables {
    t97: BookTable,
    t219: Option<BookTable>,
    t282: Option<BookTable>,
}

impl BookTables {
    pub fn new(t97: BookTable, t219: Option<BookTable>, t282: Option<BookTable>) -> Self {
        Self { t97, t219, t282 }
    }

    /// Fetch a table by its name.
    pub fn get(&self, name: &str) -> Option<&BookTable> {
        match name {
            "t97" => Some(&self.t97),
            "t219" => self.t219.as_ref(),
            "t282" => self.t282.as_ref(),
            _ => None,
        }
    }
}
