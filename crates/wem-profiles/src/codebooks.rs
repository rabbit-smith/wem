//! Checked package-resource adapters for Wwise Vorbis codebooks
//! (Python: `profiles/codebooks.py`).

use wem_vorbis::codebook::Codebook;

use crate::book_ids::{BookTables, T219_COUNT, T282_COUNT, T97_COUNT};
use crate::error::ProfileError;

/// A resolved book descriptor (Python `resolve_book_id` return dict).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBook {
    pub book_id: i64,
    /// Table name ("t97" / "t219" / "t282").
    pub table: &'static str,
    /// Index within the named table.
    pub index: i64,
    /// Python sets `lengthlist` on t219 resolutions only when the row
    /// carries one; when false the static fields come from the table row
    /// lookup instead.
    pub from_direct_fields: bool,
}

/// Resolve a 10-bit setup book ID against the installed tables
/// (Python `resolve_book_id`).
///
/// Mirrors Python's error dicts: negative ids and out-of-range ids map to
/// distinct rejection reasons.
pub fn resolve_book_id(book_id: i64, tables: &BookTables) -> Result<ResolvedBook, ProfileError> {
    if book_id < 0 {
        return Err(ProfileError::tables(format!(
            "book id is outside the installed codebook tables: {book_id}"
        )));
    }
    if book_id < T97_COUNT as i64 {
        return Ok(ResolvedBook {
            book_id,
            table: "t97",
            index: book_id,
            from_direct_fields: false,
        });
    }
    let j = book_id - T97_COUNT as i64;
    if j < T219_COUNT as i64 {
        if tables.get("t219").is_none() {
            return Err(ProfileError::tables(format!(
                "book id is outside the installed codebook tables: {book_id}"
            )));
        }
        let has_lengthlist = tables
            .get("t219")
            .and_then(|t| t.rows().get(j as usize))
            .and_then(|row| row.lengthlist.as_ref())
            .is_some();
        return Ok(ResolvedBook {
            book_id,
            table: "t219",
            index: j,
            from_direct_fields: has_lengthlist,
        });
    }
    let k = book_id - T97_COUNT as i64 - T219_COUNT as i64;
    if k < T282_COUNT as i64 {
        if tables.get("t282").is_none() {
            return Err(ProfileError::tables(format!(
                "book id is outside the installed codebook tables: {book_id}"
            )));
        }
        return Ok(ResolvedBook {
            book_id,
            table: "t282",
            index: k,
            from_direct_fields: false,
        });
    }
    Err(ProfileError::tables(format!(
        "book id is outside the installed codebook tables: {book_id}"
    )))
}

/// Build a runtime codebook from a resolved book descriptor
/// (Python `load_codebook`).
///
/// Both Python branches (direct fields vs table lookup) read the same row
/// data; the static codebook is therefore built from the table row itself,
/// while the resolved identity is carried by the `Codebook` wrapper.
pub fn load_codebook(
    resolved: &ResolvedBook,
    tables: &BookTables,
) -> Result<Codebook, ProfileError> {
    let table = tables.get(resolved.table).ok_or_else(|| {
        ProfileError::tables(format!(
            "missing decoded codebook table {:?}",
            resolved.table
        ))
    })?;
    let row = table.rows().get(resolved.index as usize).ok_or_else(|| {
        ProfileError::tables(format!(
            "book id is outside the installed codebook tables: {}",
            resolved.book_id
        ))
    })?;
    let static_codebook = row.to_static().map_err(ProfileError::Codebook)?;
    Codebook::from_static(
        static_codebook,
        Some(resolved.book_id),
        Some(resolved.table.to_string()),
        Some(resolved.index),
    )
    .map_err(ProfileError::Codebook)
}

/// Resolve setup book identifiers and build their runtime codebooks
/// (Python `load_setup_codebooks`).
pub fn load_setup_codebooks(
    book_ids: &[u64],
    tables: &BookTables,
) -> Result<Vec<Codebook>, ProfileError> {
    book_ids
        .iter()
        .map(|&book_id| {
            let resolved = resolve_book_id(book_id as i64, tables)?;
            load_codebook(&resolved, tables)
        })
        .collect()
}
