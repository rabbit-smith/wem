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
    /// The packet walk could not consume the whole data payload (Python
    /// `extract_packets` `ok=False`).
    PacketWalkFailed {
        /// Byte offset in the data payload where the walk stopped.
        position: usize,
        /// The size declared by the prefix at `position`, when the walk
        /// stopped on a size prefix that overruns the payload. `None` when it
        /// stopped on trailing bytes that are too few to be a size prefix:
        /// the walk observed no size there, and a number here would be an
        /// invention rather than an observation.
        declared_size: Option<u16>,
        /// Bytes present from `position` to the end of the payload — what the
        /// walk actually had left. One byte for the trailing-bytes case.
        remaining: usize,
    },
    /// Unknown endianness marker.
    BadEndian { marker: [u8; 4] },
    /// Terminal overlap excess exceeds the u16 the fmt field stores.
    TerminalExcessTooLarge { excess: u64 },
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
            ContainerError::PacketWalkFailed {
                position,
                declared_size,
                remaining,
            } => match declared_size {
                Some(size) => write!(
                    f,
                    "packet walk failed at {position}: size {size} declared, \
                     but only {remaining} byte(s) remain from the prefix"
                ),
                None => write!(
                    f,
                    "packet walk failed at {position}: {remaining} trailing \
                     byte(s) are too few for a size prefix"
                ),
            },
            ContainerError::BadEndian { marker } => {
                write!(f, "unknown endianness marker {marker:?}")
            }
            ContainerError::TerminalExcessTooLarge { excess } => {
                write!(f, "terminal overlap excess {excess} exceeds u16")
            }
        }
    }
}

impl std::error::Error for ContainerError {}
