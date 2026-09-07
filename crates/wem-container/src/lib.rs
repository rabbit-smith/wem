//! WEM encoder container domain: RIFF/WEM models and codecs.
//!
//! Per docs/architecture.md this crate does not import application,
//! profiles, or analysis code; it works on plain bytes only. Mirrors the
//! Python `wwise_wem/container` package (`riff.py`, `fmt.py`, `packets.py`,
//! `wem.py`).

/// Container errors (Python: `ValueError` family).
pub mod error;

/// Pure byte codecs for the Wwise Vorbis `fmt ` payload (66 bytes)
/// (Python: `container/fmt.py`).
pub mod fmt;

/// Pure byte codecs for Wwise size-prefixed packet streams
/// (Python: `container/packets.py`).
pub mod packets;

/// Pure RIFF/RIFX WAVE chunk parsing and construction
/// (Python: `container/riff.py`).
pub mod riff;

/// Pure byte composition for complete Wwise WEM containers
/// (Python: `container/wem.py`).
pub mod wem;

pub use error::ContainerError;
pub use fmt::VorbisFmtFields;
pub use packets::{build_packet_stream, extract_packets, recompute_vorbis_fmt_sizes, PacketWalk};
pub use riff::{build_riff, parse_chunks, Endian, ParsedChunk};
pub use wem::{build_vorbis_wem, load_wem_parts_bytes, WemBuildResult, WemParts};
