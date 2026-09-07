//! Pure Vorbis Huffman and maptype-1 VQ codebook core, plus the Wwise setup
//! packet syntax codec and canonical LSB-first bit IO.
//!
//! Mirrors the Python `wwise_wem/vorbis` package: `bitio.py`, `codebook.py`,
//! `setup.py`. This crate never imports application, profiles, or container
//! code (docs/architecture.md).

pub mod bitio;
pub mod codebook;
pub mod floor;
pub mod floor_fit;
pub mod packet_encoder;
pub mod residue;
pub mod setup;
