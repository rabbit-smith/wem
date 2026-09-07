//! Container errors (Python: `ValueError` family).

/// Errors raised by the pure container codecs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerError {
    /// Bytes do not start with `RIFF`/`RIFX`.
    NotRiff,
    /// RIFF header extent truncated.
    TruncatedRiff { at: usize },
    /// Chunk id not exactly 4 bytes.
    BadChunkId { id: String },
    /// Vorbis fmt payload shorter than 66 bytes.
    FmtTooShort { got: usize },
    /// fmt payload is not 66 bytes where 66 is required.
    FmtSize { got: usize },
    /// seek_table_size exceeds the data payload.
    BadSeekTableSize {
        seek_table_size: usize,
        data_size: usize,
    },
    /// Packet size prefix overflows u16.
    PacketTooLarge { len: usize },
    /// Data payload truncated inside a size prefix.
    TruncatedPacketStream { at: usize },
    /// Missing required chunk.
    MissingChunk { id: &'static str },
    /// Packet walk failed (Python `extract_packets` `ok=False`).
    PacketWalkFailed { position: usize, size: u16 },
    /// Unknown endianness marker.
    BadEndian { marker: [u8; 4] },
}

impl std::fmt::Display for ContainerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerError::NotRiff => write!(f, "not RIFF/RIFX"),
            ContainerError::TruncatedRiff { at } => {
                write!(f, "RIFF header truncated at offset {at}")
            }
            ContainerError::BadChunkId { id } => write!(f, "bad chunk id {id}"),
            ContainerError::FmtTooShort { got } => {
                write!(f, "vorbis fmt expected 66 bytes, got {got}")
            }
            ContainerError::FmtSize { got } => {
                write!(f, "vorbis fmt must be 66 bytes, got {got}")
            }
            ContainerError::BadSeekTableSize {
                seek_table_size,
                data_size: _,
            } => write!(f, "bad seek_table_size={seek_table_size}"),
            ContainerError::PacketTooLarge { len } => write!(f, "packet too large: {len}"),
            ContainerError::TruncatedPacketStream { at } => {
                write!(f, "packet stream truncated at {at}")
            }
            ContainerError::MissingChunk { id } => write!(f, "missing {id}"),
            ContainerError::PacketWalkFailed { position, size } => {
                write!(f, "packet walk failed at {position} (size {size})")
            }
            ContainerError::BadEndian { marker } => {
                write!(f, "unknown endianness marker {marker:?}")
            }
        }
    }
}

impl std::error::Error for ContainerError {}
