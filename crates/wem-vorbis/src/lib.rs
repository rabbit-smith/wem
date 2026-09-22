//! Pure Vorbis Huffman and maptype-1 VQ codebook core, plus the Wwise setup
//! packet syntax codec, the audio packet codec in both directions and
//! canonical LSB-first bit IO.
//!
//! Mirrors the Python `wwise_wem/vorbis` package: `bitio.py`, `codebook.py`,
//! `setup.py`, `packet_encoder.py`, `packet_decoder.py`. This crate never
//! imports application, profiles, or container code
//! (docs/reference/architecture.md).

pub mod bitio;
pub mod codebook;
pub mod floor;
pub mod floor_fit;
pub mod packet_decoder;
pub mod packet_encoder;
pub mod residue;
pub mod setup;
