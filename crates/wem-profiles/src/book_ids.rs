//! Map Wwise setup 10-bit book IDs to installed static codebook tables
//! (Python: `wwise_wem_reference.profiles.book_ids`).
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

/// One decoded codebook table of the compiled carrier.
#[derive(Debug, Clone)]
pub struct BookTable {
    name: &'static str,
    rows: Vec<CodebookRow>,
}

impl BookTable {
    /// Build one table from the compiled carrier ([`crate::generated`]).
    ///
    /// The rows were checked when the carrier was generated, so this only
    /// converts them into the codec's row type.
    pub fn from_generated(
        name: &'static str,
        rows: &'static [crate::tables::CodebookRowTable],
    ) -> Result<Self, ProfileError> {
        let (static_name, _expected_count) = BOOK_COUNTS
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .ok_or_else(|| {
                ProfileError::tables(format!("missing decoded codebook table {name:?}"))
            })?;
        let rows = rows.iter().map(crate::carrier::codebook_row).collect();
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
