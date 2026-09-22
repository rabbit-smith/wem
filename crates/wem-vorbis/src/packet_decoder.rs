//! Parse Wwise Vorbis *audio* packets (post-setup) into typed values
//! (Python: `wwise_wem_reference/vorbis/packet_decoder.py`).
//!
//! Wwise 2013.2 against raw Ogg Vorbis I:
//!
//! - there is no packet-type bit (every packet after the setup is audio); the
//!   first field is the mode number, using `ilog(nmodes - 1)` bits,
//! - a long packet does *not* serialize Vorbis-I's previous/next-window flags,
//!   so interpreting the two floor flags as window bits misaligns every long
//!   packet,
//! - floor1 and residue follow the Vorbis-I body layout,
//! - a one-byte `0x01` packet is a complete long-mode all-zero-floor packet.
//!
//! Ownership: this module reads bits and returns typed values. It never
//! selects a mode, a mapping or a geometry — the setup and the block sizes
//! arrive as arguments, as do the codebooks.

use crate::bitio::{BitError, BitReader};
use crate::codebook::Codebook;
use crate::floor::{decode_floor1_body, Floor1DecodeError};
use crate::residue::{decode_residue_coeffs, ResidueDecodeError, ResidueStatus};
use crate::setup::{ilog, CouplingStep, Mapping0Setup, SetupInfo};

/// Audio-packet parse errors.
///
/// Every variant carries the values that were observed (channel, submap, bit
/// position, the table that was too short) and keeps the stage error reachable
/// through [`std::error::Error::source`]. A packet that ends early is reported
/// here; a *residue* that ends early is not an error at all — the format
/// allows it, and it leaves through [`ResidueStatus::EndOfPacket`].
#[derive(Debug, Clone, PartialEq)]
pub enum PacketDecodeError {
    /// The packet ended inside the mode field.
    ModeBits { position: u64, source: BitError },
    /// The decoded mode number is outside the setup's mode list.
    ModeOutOfRange { mode: u64, modes: u64 },
    /// The mode names a mapping the setup does not hold.
    MappingOutOfRange { mapping: u64, maps: usize },
    /// The mapping's channel mux has no entry for a channel.
    ChannelMuxTooShort { got: usize, want: usize },
    /// The mapping's submap → floor list has no entry for a submap.
    FloorMapTooShort { got: usize, want: usize },
    /// The submap's floor index is outside the setup's floor list.
    FloorIndexOutOfRange { index: u64, floors: usize },
    /// The packet ended inside a channel's floor nonzero flag.
    FloorNonzero {
        channel: usize,
        position: u64,
        source: BitError,
    },
    /// A channel's floor1 body failed to decode.
    FloorBody {
        channel: usize,
        position: u64,
        source: Floor1DecodeError,
    },
    /// The mapping declares no residue submap at all.
    MissingResidueSubmap,
    /// The submap's residue index is outside the setup's residue list.
    ResidueIndexOutOfRange { index: u64, residues: usize },
    /// A residue submap failed to decode.
    Residue {
        submap: usize,
        residue_id: u64,
        source: ResidueDecodeError,
    },
    /// A coupling step names a channel the packet does not carry.
    CouplingChannelOutOfRange { channel: usize, channels: usize },
}

impl std::fmt::Display for PacketDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PacketDecodeError::ModeBits { position, source } => {
                write!(f, "audio packet mode bits at {position}: {source}")
            }
            PacketDecodeError::ModeOutOfRange { mode, modes } => {
                write!(f, "mode {mode} out of range 0..{modes}")
            }
            PacketDecodeError::MappingOutOfRange { mapping, maps } => {
                write!(f, "mapping {mapping} out of range 0..{maps}")
            }
            PacketDecodeError::ChannelMuxTooShort { got, want } => {
                write!(f, "mapping channel mux has {got} entries, expected {want}")
            }
            PacketDecodeError::FloorMapTooShort { got, want } => {
                write!(f, "mapping floor list has {got} entries, expected {want}")
            }
            PacketDecodeError::FloorIndexOutOfRange { index, floors } => {
                write!(f, "floor index {index} out of range 0..{floors}")
            }
            PacketDecodeError::FloorNonzero {
                channel,
                position,
                source,
            } => write!(
                f,
                "floor nonzero for channel {channel} at {position}: {source}"
            ),
            PacketDecodeError::FloorBody {
                channel,
                position,
                source,
            } => write!(
                f,
                "floor body for channel {channel} at {position}: {source}"
            ),
            PacketDecodeError::MissingResidueSubmap => {
                write!(f, "mapping has no residue submap")
            }
            PacketDecodeError::ResidueIndexOutOfRange { index, residues } => {
                write!(f, "residue index {index} out of range 0..{residues}")
            }
            PacketDecodeError::Residue {
                submap,
                residue_id,
                source,
            } => write!(f, "residue submap {submap} (id {residue_id}): {source}"),
            PacketDecodeError::CouplingChannelOutOfRange { channel, channels } => {
                write!(f, "coupling channel {channel} out of range 0..{channels}")
            }
        }
    }
}

impl std::error::Error for PacketDecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PacketDecodeError::ModeBits { source, .. } => Some(source),
            PacketDecodeError::FloorNonzero { source, .. } => Some(source),
            PacketDecodeError::FloorBody { source, .. } => Some(source),
            PacketDecodeError::Residue { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The mode and mapping an audio packet selects (Python
/// `parse_audio_header`'s dict).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioHeader {
    /// Mode number as read from the packet.
    pub mode: u64,
    /// The mode's block size selector: 0 = short, 1 = long.
    pub blockflag: u64,
    /// The mode's mapping index.
    pub mapping: u64,
    /// Previous-window flag. Always `None`: this format revision serializes
    /// only the mode number, so there is nothing to read and nothing to
    /// reinterpret as Vorbis-I window geometry.
    pub prev_window: Option<u64>,
    /// Following-window flag; always `None`, as [`AudioHeader::prev_window`].
    pub next_window: Option<u64>,
    /// Bit position where the header starts.
    pub bit_start: u64,
    /// Bit position just past the mode field.
    pub bit_end: u64,
    /// Submaps of the selected mapping.
    pub submaps: u64,
    /// Per-submap floor indices of the selected mapping.
    pub floor_ids: Vec<u64>,
    /// Per-submap residue indices of the selected mapping.
    pub residue_ids: Vec<u64>,
    /// Per-channel submap selection of the selected mapping.
    pub chmux: Vec<u64>,
}

/// Read the mode field of an audio packet (Python `parse_audio_header`).
///
/// This Wwise revision writes the mode number only: the two Vorbis-I window
/// flags are absent, which is why [`AudioHeader::prev_window`] and
/// [`AudioHeader::next_window`] are always `None`.
pub fn parse_audio_header(
    br: &mut BitReader<'_>,
    setup: &SetupInfo,
) -> Result<AudioHeader, PacketDecodeError> {
    let nmodes = setup.nmodes;
    let mode_bits = if nmodes > 1 { ilog(nmodes - 1) } else { 0 };
    let start = br.tell_bits();
    let mode_num = if mode_bits != 0 {
        br.read(mode_bits)
            .map_err(|source| PacketDecodeError::ModeBits {
                position: br.tell_bits(),
                source,
            })?
    } else {
        0
    };
    if mode_num >= nmodes {
        return Err(PacketDecodeError::ModeOutOfRange {
            mode: mode_num,
            modes: nmodes,
        });
    }
    // mode_num < nmodes <= 2^6, so the conversion is exact.
    let mode = *setup
        .modes
        .get(mode_num as usize)
        .ok_or(PacketDecodeError::ModeOutOfRange {
            mode: mode_num,
            modes: nmodes,
        })?;
    let mapping =
        setup
            .maps
            .get(mode.mapping as usize)
            .ok_or(PacketDecodeError::MappingOutOfRange {
                mapping: mode.mapping,
                maps: setup.maps.len(),
            })?;
    Ok(AudioHeader {
        mode: mode_num,
        blockflag: mode.blockflag,
        mapping: mode.mapping,
        prev_window: None,
        next_window: None,
        bit_start: start,
        bit_end: br.tell_bits(),
        submaps: mapping.submaps,
        floor_ids: mapping.floors.clone(),
        residue_ids: mapping.residues.clone(),
        chmux: mapping.chmux.clone(),
    })
}

/// Per-channel floor state of one audio packet (Python
/// `decode_floors_for_packet`'s dict).
#[derive(Debug, Clone, PartialEq)]
pub struct FloorsForPacket {
    /// One nonzero flag per channel, in channel order.
    pub nonzero: Vec<bool>,
    /// One entry per channel: the floor1 Y posts (wrapped residuals), or
    /// `None` when the channel's floor is inactive.
    pub curves: Vec<Option<Vec<i64>>>,
    /// Bit position just past the last floor.
    pub bit_pos: u64,
}

/// Read the per-channel floor flags and floor1 bodies (Python
/// `decode_floors_for_packet`).
///
/// A channel whose nonzero flag is 0 has no body and no curve; the channels
/// are read in order, so a failure names the channel and the bit position it
/// happened at.
pub fn decode_floors_for_packet(
    br: &mut BitReader<'_>,
    setup: &SetupInfo,
    books: &[Codebook],
    channels: u32,
    mapping: &Mapping0Setup,
) -> Result<FloorsForPacket, PacketDecodeError> {
    let channels = channels as usize;
    let mut nonzero = Vec::with_capacity(channels);
    let mut curves = Vec::with_capacity(channels);
    for ch in 0..channels {
        let sub = if mapping.submaps > 1 {
            *mapping
                .chmux
                .get(ch)
                .ok_or(PacketDecodeError::ChannelMuxTooShort {
                    got: mapping.chmux.len(),
                    want: channels,
                })?
        } else {
            0
        };
        let floor_index =
            *mapping
                .floors
                .get(sub as usize)
                .ok_or(PacketDecodeError::FloorMapTooShort {
                    got: mapping.floors.len(),
                    want: sub as usize + 1,
                })?;
        // Compare in the index's own width first: an index that does not fit
        // the platform's `usize` is out of range, not a wrapped entry.
        let floor = if floor_index < setup.floors.len() as u64 {
            &setup.floors[floor_index as usize]
        } else {
            return Err(PacketDecodeError::FloorIndexOutOfRange {
                index: floor_index,
                floors: setup.floors.len(),
            });
        };
        let nonzero_flag = br
            .read(1)
            .map_err(|source| PacketDecodeError::FloorNonzero {
                channel: ch,
                position: br.tell_bits(),
                source,
            })?;
        nonzero.push(nonzero_flag != 0);
        if nonzero_flag != 0 {
            let position = br.tell_bits();
            let y = decode_floor1_body(br, floor, books).map_err(|source| {
                PacketDecodeError::FloorBody {
                    channel: ch,
                    position,
                    source,
                }
            })?;
            curves.push(Some(y));
        } else {
            curves.push(None);
        }
    }
    Ok(FloorsForPacket {
        nonzero,
        curves,
        bit_pos: br.tell_bits(),
    })
}

/// One submap's decoded residue (the reference's `consume_residue` status
/// dict, carrying the coefficients it discarded).
#[derive(Debug, Clone, PartialEq)]
pub struct ResidueDecode {
    /// The residue configuration the submap selected.
    pub residue_id: u64,
    /// Per-channel coefficient rows (`coeffs[channel][bin]`).
    pub coeffs: Vec<Vec<f64>>,
    /// Whether the residue was read in full, was empty, or hit the end of the
    /// packet.
    pub status: ResidueStatus,
    /// Bit position where this residue started.
    pub bit_start: u64,
    /// Bit position just past this residue.
    pub bit_end: u64,
}

/// One parsed audio packet (Python `parse_audio_packet`'s dict).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioPacket {
    /// Payload length in bytes.
    pub packet_size: usize,
    /// The mode and mapping the packet selected.
    pub header: AudioHeader,
    /// Per-channel floor nonzero flags.
    pub nonzero: Vec<bool>,
    /// Per-channel floor1 Y posts (wrapped residuals).
    pub curves: Vec<Option<Vec<i64>>>,
    /// Bit position where the residue section starts.
    pub bits_after_floor: u64,
    /// One entry per residue submap of the mapping, in submap order.
    pub residues: Vec<ResidueDecode>,
    /// Bit position just past the last residue.
    pub bits_after_residue: u64,
    /// Bits left in the payload after the last residue (byte padding, or an
    /// early residue stop).
    pub bits_left: u64,
}

/// Parse one audio packet into header, floor posts and residue coefficients
/// (Python `parse_audio_packet`).
///
/// `block_sizes` is `[short, long]` in samples, indexed by the mode's
/// `blockflag`; the spectrum width is half the selected block size. The
/// reference hardcodes `[256, 2048]` here; this crate takes the geometry as an
/// argument, since only the caller knows the profile it resolved.
///
/// The reference returns an incomplete-packet dict instead of raising, and it
/// passes the *raw* floor nonzero flags to the residue stage. Both are
/// preserved in shape: a packet that ends before the floor section is a typed
/// error, and the residue stage sees the flags as read. A mapping with
/// coupling steps needs [`coupling_dirty_nonzero`] applied first, exactly as
/// the reference's composite decode does.
pub fn parse_audio_packet(
    payload: &[u8],
    setup: &SetupInfo,
    books: &[Codebook],
    channels: u32,
    block_sizes: [usize; 2],
) -> Result<AudioPacket, PacketDecodeError> {
    let mut br = BitReader::new(payload);
    let header = parse_audio_header(&mut br, setup)?;
    let mapping =
        setup
            .maps
            .get(header.mapping as usize)
            .ok_or(PacketDecodeError::MappingOutOfRange {
                mapping: header.mapping,
                maps: setup.maps.len(),
            })?;
    let floors = decode_floors_for_packet(&mut br, setup, books, channels, mapping)?;
    // blockflag comes from a one-bit read, so the index is always valid.
    let n_spec = block_sizes[(header.blockflag & 1) as usize] / 2;
    let ch_used: Vec<bool> = floors.nonzero.clone();
    let bits_after_floor = br.tell_bits();

    let mut residues = Vec::with_capacity(header.residue_ids.len());
    for (submap, &residue_id) in header.residue_ids.iter().enumerate() {
        let residue = if residue_id < setup.residues.len() as u64 {
            &setup.residues[residue_id as usize]
        } else {
            return Err(PacketDecodeError::ResidueIndexOutOfRange {
                index: residue_id,
                residues: setup.residues.len(),
            });
        };
        let bit_start = br.tell_bits();
        let (coeffs, status) = decode_residue_coeffs(&mut br, residue, books, &ch_used, n_spec)
            .map_err(|source| PacketDecodeError::Residue {
                submap,
                residue_id,
                source,
            })?;
        residues.push(ResidueDecode {
            residue_id,
            coeffs,
            status,
            bit_start,
            bit_end: br.tell_bits(),
        });
    }
    if header.residue_ids.is_empty() && floors.nonzero.iter().any(|&used| used) {
        return Err(PacketDecodeError::MissingResidueSubmap);
    }
    Ok(AudioPacket {
        packet_size: payload.len(),
        header,
        nonzero: floors.nonzero,
        curves: floors.curves,
        bits_after_floor,
        residues,
        bits_after_residue: br.tell_bits(),
        bits_left: br.bits_left(),
    })
}

/// Propagate floor use across coupling steps before the residue stage
/// (Python `decode_packet_2ch`'s `nonzero_dirty`).
///
/// A coupling step makes both of its channels' residues live as soon as
/// either floor is: the pair is coded jointly, so a decoder that keeps the
/// raw flags decodes the wrong bit stream. The reference's composite decode
/// applies this before calling the residue inverse.
pub fn coupling_dirty_nonzero(
    nonzero: &[bool],
    coupling: &[CouplingStep],
) -> Result<Vec<bool>, PacketDecodeError> {
    let mut dirty = nonzero.to_vec();
    for step in coupling {
        let channels = dirty.len();
        let mag = usize::try_from(step.mag)
            .ok()
            .filter(|&channel| channel < channels)
            .ok_or(PacketDecodeError::CouplingChannelOutOfRange {
                channel: if step.mag > usize::MAX as u64 {
                    usize::MAX
                } else {
                    step.mag as usize
                },
                channels,
            })?;
        let ang = usize::try_from(step.ang)
            .ok()
            .filter(|&channel| channel < channels)
            .ok_or(PacketDecodeError::CouplingChannelOutOfRange {
                channel: if step.ang > usize::MAX as u64 {
                    usize::MAX
                } else {
                    step.ang as usize
                },
                channels,
            })?;
        if dirty[mag] || dirty[ang] {
            dirty[mag] = true;
            dirty[ang] = true;
        }
    }
    Ok(dirty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitio::OggPack;
    use crate::codebook::StaticCodebook;
    use crate::floor::{floor1_wrap, postlist_from_floor};
    use crate::packet_encoder::{pack_audio_header, pack_floor1_body};
    use crate::residue::pack_residue_silent;
    use crate::setup::{BitPositions, Floor1Setup, ModeSetup, ResidueSetup};

    /// The mode field plus one zero floor flag per channel: the packet a
    /// silent block produces. The encoder-side helper that used to write this
    /// schedule had no caller in the kernel and was deleted, so the test
    /// writes it from the primitives that remain.
    fn silence_payload(setup: &SetupInfo, channels: u32, mode: u32) -> Vec<u8> {
        let mut op = OggPack::new(16);
        pack_audio_header(&mut op, setup, mode).expect("mode must pack");
        for _ in 0..channels {
            op.write(0, 1).expect("floor flag must pack");
        }
        op.into_buffer()
    }

    /// The mode field, one floor1 body per nonzero channel and an optional
    /// silent residue: the same schedule the deleted floor-only helper wrote,
    /// rebuilt from the primitives that remain.
    fn floor_only_payload(
        setup: &SetupInfo,
        books: &[Codebook],
        channels: u32,
        mode: u32,
        curves: &[Option<Vec<i64>>],
        silent_residue: bool,
        n_spectrum: Option<usize>,
    ) -> Vec<u8> {
        let mut op = OggPack::new(512);
        pack_audio_header(&mut op, setup, mode).expect("mode must pack");
        let mapping = &setup.maps[setup.modes[mode as usize].mapping as usize];
        let mut used = 0u32;
        for (ch, curve) in curves.iter().take(channels as usize).enumerate() {
            let sub = if mapping.submaps > 1 {
                mapping.chmux[ch]
            } else {
                0
            };
            let floor_index = mapping.floors[sub as usize];
            match curve.as_ref() {
                None => op.write(0, 1).expect("floor flag must pack"),
                Some(y) => {
                    op.write(1, 1).expect("floor flag must pack");
                    pack_floor1_body(&mut op, setup, floor_index, books, y)
                        .expect("floor body must pack");
                    used += 1;
                }
            }
        }
        if used != 0 && silent_residue {
            let residue = &setup.residues[mapping.residues[0] as usize];
            pack_residue_silent(&mut op, residue, books, used, n_spectrum)
                .expect("silent residue must pack");
        }
        op.into_buffer()
    }

    /// One maptype-0 book used both as the floor's subclass book and as the
    /// residue classbook (dim 1 ⇒ one class per classword).
    fn shared_book() -> Codebook {
        Codebook::from_static(
            StaticCodebook {
                dim: 1,
                entries: 256,
                lengthlist: vec![8; 256],
                maptype: 0,
                q_min: 0,
                q_delta: 0,
                q_quant: 0,
                q_sequencep: 0,
                quantlist: None,
            },
            None,
            None,
            None,
        )
        .expect("synthetic book must build")
    }

    fn test_floor() -> Floor1Setup {
        Floor1Setup {
            partitions: 2,
            partition_classes: vec![0, 0],
            max_class: 0,
            class_dims: vec![2],
            class_subs: vec![0],
            class_masterbooks: vec![None],
            subclass_books: vec![vec![0]],
            multiplier: 1,
            rangebits: 7,
            x_list: vec![32, 64, 96, 16],
            bit_start: 0,
            bit_end: 0,
        }
    }

    fn test_residue() -> ResidueSetup {
        ResidueSetup {
            residue_type: 0,
            begin: 0,
            end: 112,
            partition_size: 16,
            classifications: 8,
            classbook: 0,
            // A silent residue requires the class-0 cascade to be empty.
            cascades: vec![0; 8],
            books: vec![[-1; 8]; 8],
            bit_start: 0,
            bit_end: 0,
        }
    }

    fn test_setup(coupling: Vec<CouplingStep>) -> SetupInfo {
        SetupInfo {
            setup_size: 0,
            bits_total: 0,
            channels: 2,
            nbooks: 1,
            book_ids: vec![0],
            unique_book_ids: vec![0],
            nfloors: 1,
            floors: vec![test_floor()],
            nresidues: 1,
            residues: vec![test_residue()],
            nmaps: 1,
            maps: vec![Mapping0Setup {
                submaps: 1,
                coupling,
                reserved: 0,
                chmux: vec![0, 0],
                floors: vec![0],
                residues: vec![0],
                bit_start: 0,
                bit_end: 0,
            }],
            nmodes: 2,
            modes: vec![
                ModeSetup {
                    blockflag: 0,
                    mapping: 0,
                    bit_start: 0,
                    bit_end: 0,
                },
                ModeSetup {
                    blockflag: 1,
                    mapping: 0,
                    bit_start: 0,
                    bit_end: 0,
                },
            ],
            bit_positions: BitPositions {
                after_books: 0,
                after_floor_count: 0,
                after_floors: 0,
                after_residues: 0,
                after_maps: 0,
                after_modes: 0,
                end: 0,
            },
            trailing_pad_bits: 0,
            trailing_pad_value: 0,
            parse_complete: true,
            book_id_assignment: "",
        }
    }

    #[test]
    fn silence_packet_parses_as_an_all_zero_floor_packet() {
        let setup = test_setup(Vec::new());
        let books = vec![shared_book()];
        let payload = silence_payload(&setup, 2, 0);
        let parsed = parse_audio_packet(&payload, &setup, &books, 2, [256, 2048]).unwrap();

        assert_eq!(parsed.packet_size, payload.len());
        assert_eq!(parsed.header.mode, 0);
        assert_eq!(parsed.header.blockflag, 0);
        assert_eq!(parsed.header.mapping, 0);
        assert_eq!(parsed.header.submaps, 1);
        assert_eq!(parsed.header.floor_ids, vec![0]);
        assert_eq!(parsed.header.residue_ids, vec![0]);
        assert_eq!(parsed.header.chmux, vec![0, 0]);
        // This revision carries no window flags.
        assert_eq!(parsed.header.prev_window, None);
        assert_eq!(parsed.header.next_window, None);
        assert_eq!(parsed.header.bit_start, 0);
        assert_eq!(parsed.header.bit_end, 1); // one mode bit
        assert_eq!(parsed.nonzero, vec![false, false]);
        assert_eq!(parsed.curves, vec![None, None]);
        // No channel uses its floor, so the residue is empty and unconsumed.
        assert_eq!(parsed.residues.len(), 1);
        assert_eq!(parsed.residues[0].status, ResidueStatus::Empty);
        assert_eq!(parsed.residues[0].coeffs, vec![vec![0.0; 128]; 2]);
        assert_eq!(parsed.bits_after_residue, parsed.bits_after_floor);
        assert!(parsed.bits_left < 8);
    }

    #[test]
    fn long_mode_packet_selects_the_long_block_geometry() {
        let setup = test_setup(Vec::new());
        let books = vec![shared_book()];
        let payload = silence_payload(&setup, 2, 1);
        let short = parse_audio_packet(&payload, &setup, &books, 2, [256, 2048]).unwrap();
        let long = parse_audio_packet(&payload, &setup, &books, 2, [256, 2048]).unwrap();
        assert_eq!(short.header.blockflag, 1);
        assert_eq!(short.residues[0].coeffs[0].len(), 1024);
        assert_eq!(long, short);
    }

    #[test]
    fn floor_posts_round_trip_through_the_packet_parser() {
        let setup = test_setup(Vec::new());
        let books = vec![shared_book()];
        let floor = &setup.floors[0];
        let postlist = postlist_from_floor(floor);
        // Absolute posts → packet residuals, as the packet assembler expects.
        let posts = vec![7, 200, 31, 64, 96, 250];
        let y = floor1_wrap(&posts, &postlist, 256).unwrap();
        let payload = floor_only_payload(
            &setup,
            &books,
            2,
            0,
            &[Some(y.clone()), None],
            true,
            Some(128),
        );

        let parsed = parse_audio_packet(&payload, &setup, &books, 2, [256, 2048]).unwrap();
        assert_eq!(parsed.nonzero, vec![true, false]);
        assert_eq!(parsed.curves[0].as_ref(), Some(&y));
        assert_eq!(parsed.curves[1], None);
        // The silent residue codes class 0 for the one used channel and no VQ
        // vectors, so every coefficient stays zero and the packet closes.
        assert_eq!(parsed.residues[0].status, ResidueStatus::Complete);
        assert_eq!(parsed.residues[0].coeffs[0][0], 0.0);
        assert_eq!(parsed.residues[0].coeffs[0].len(), 128);
        assert!(parsed.bits_after_residue <= (payload.len() as u64) * 8);
        assert!(parsed.bits_left < 8, "packet must close to byte padding");
    }

    #[test]
    fn coupling_dirty_nonzero_propagates_one_sided_floor_use() {
        let coupling = vec![CouplingStep { mag: 0, ang: 1 }];
        // One live floor makes both channels live: the pair is coded jointly.
        assert_eq!(
            coupling_dirty_nonzero(&[true, false], &coupling).unwrap(),
            vec![true, true]
        );
        assert_eq!(
            coupling_dirty_nonzero(&[false, true], &coupling).unwrap(),
            vec![true, true]
        );
        // Neither live leaves both alone.
        assert_eq!(
            coupling_dirty_nonzero(&[false, false], &coupling).unwrap(),
            vec![false, false]
        );
        // No coupling steps is the identity.
        assert_eq!(
            coupling_dirty_nonzero(&[true, false], &[]).unwrap(),
            vec![true, false]
        );
        // A step naming a channel the packet does not carry is reported.
        assert_eq!(
            coupling_dirty_nonzero(&[true, false], &[CouplingStep { mag: 0, ang: 2 }]).unwrap_err(),
            PacketDecodeError::CouplingChannelOutOfRange {
                channel: 2,
                channels: 2
            }
        );
    }

    #[test]
    fn malformed_packets_return_typed_errors() {
        let setup = test_setup(Vec::new());
        let books = vec![shared_book()];

        // An empty payload cannot even carry the mode field.
        assert!(matches!(
            parse_audio_packet(&[], &setup, &books, 2, [256, 2048]).unwrap_err(),
            PacketDecodeError::ModeBits { .. }
        ));

        // One byte with the mode bit set and both floor flags set promises
        // floor bodies the payload does not carry.
        let err = parse_audio_packet(&[0xFF], &setup, &books, 2, [256, 2048]).unwrap_err();
        assert!(matches!(
            err,
            PacketDecodeError::FloorBody { channel: 0, .. }
        ));
        assert!(std::error::Error::source(&err).is_some());

        // A mode number outside the setup's mode list: three modes need two
        // mode bits, and `0b11` names the fourth, which does not exist.
        let mut bad = setup.clone();
        bad.nmodes = 3;
        bad.modes.push(bad.modes[0]);
        let err = parse_audio_packet(&[0x03], &bad, &books, 2, [256, 2048]).unwrap_err();
        assert_eq!(err, PacketDecodeError::ModeOutOfRange { mode: 3, modes: 3 });

        // A mode whose mapping the setup does not hold.
        let mut bad = setup.clone();
        bad.maps.clear();
        let err = parse_audio_packet(&[0x01], &bad, &books, 2, [256, 2048]).unwrap_err();
        assert_eq!(
            err,
            PacketDecodeError::MappingOutOfRange {
                mapping: 0,
                maps: 0
            }
        );

        // A mapping whose floor list cannot serve the submap it selects.
        let mut bad = setup.clone();
        bad.maps[0].floors.clear();
        let err = parse_audio_packet(&[0x01], &bad, &books, 2, [256, 2048]).unwrap_err();
        assert_eq!(err, PacketDecodeError::FloorMapTooShort { got: 0, want: 1 });

        // A submap naming a floor the setup does not hold.
        let mut bad = setup.clone();
        bad.maps[0].floors = vec![3];
        let err = parse_audio_packet(&[0x01], &bad, &books, 2, [256, 2048]).unwrap_err();
        assert_eq!(
            err,
            PacketDecodeError::FloorIndexOutOfRange {
                index: 3,
                floors: 1
            }
        );

        // A mapping with no residue submap at all, on a packet that needs one.
        let mut bad = setup.clone();
        bad.maps[0].residues.clear();
        let payload = floor_only_payload(
            &setup,
            &books,
            2,
            0,
            &[Some(vec![0; 6]), None],
            false,
            Some(128),
        );
        let err = parse_audio_packet(&payload, &bad, &books, 2, [256, 2048]).unwrap_err();
        assert_eq!(err, PacketDecodeError::MissingResidueSubmap);

        // A submap naming a residue the setup does not hold.
        let mut bad = setup.clone();
        bad.maps[0].residues = vec![7];
        let err = parse_audio_packet(&payload, &bad, &books, 2, [256, 2048]).unwrap_err();
        assert_eq!(
            err,
            PacketDecodeError::ResidueIndexOutOfRange {
                index: 7,
                residues: 1
            }
        );

        // Every prefix of a well-formed packet must parse or fail cleanly —
        // never panic.
        for payload in [silence_payload(&setup, 2, 0), silence_payload(&setup, 2, 1)] {
            for cut in 0..payload.len() {
                let _ = parse_audio_packet(&payload[..cut], &setup, &books, 2, [256, 2048]);
            }
        }
    }

    #[test]
    fn floors_for_packet_checks_the_channel_mux() {
        let mut setup = test_setup(Vec::new());
        setup.maps[0].submaps = 2;
        setup.maps[0].chmux = vec![0];
        let books = vec![shared_book()];
        // Enough bits for channel 0's flag and full floor body, so the
        // failure is the missing mux entry rather than a short payload.
        let mut br = BitReader::new(&[0xFF; 8]);
        let mapping = setup.maps[0].clone();
        let err = decode_floors_for_packet(&mut br, &setup, &books, 2, &mapping).unwrap_err();
        assert_eq!(
            err,
            PacketDecodeError::ChannelMuxTooShort { got: 1, want: 2 }
        );
    }

    #[test]
    fn packet_errors_expose_their_cause() {
        use std::error::Error;
        let err = PacketDecodeError::ModeBits {
            position: 0,
            source: BitError::OutOfBits,
        };
        assert!(err.source().is_some());
        assert!(PacketDecodeError::MissingResidueSubmap.source().is_none());
    }
}
