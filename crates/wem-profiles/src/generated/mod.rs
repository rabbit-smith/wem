//! Generated encoder profile tables. Do not edit; regenerate with
//! `python3 scripts/generate_profile_code.py`.
//!
//! Source material: the profile tree (`--help` names the location).
//! Every float is stored as its IEEE bit pattern, never as a decimal literal.

//!
//! The compiled carrier: one typed table set per installed profile.

pub mod codebooks;
pub mod wwise2013_2ch_48000;
pub mod wwise2013_6ch_44100;

use crate::tables::ProfileTables;

/// Every installed profile's typed tables, in deterministic (sorted)
/// order.
#[rustfmt::skip]
pub static PROFILES: [&ProfileTables; 2] = [
    &wwise2013_2ch_48000::TABLES,
    &wwise2013_6ch_44100::TABLES,
];
