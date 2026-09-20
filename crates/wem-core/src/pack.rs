//! Frame-level packet assembly for one analysis frame.
//!
//! Rust-side equivalent of Python `wwise_wem/vorbis/packet_encoder.py::
//! pack_analysis_frame`, relocated to the orchestration layer (wem-core)
//! because it bridges analysis output (`PsyFrame`) with Vorbis packing —
//! the per-channel post/raw assembly step of the Python application layer.
//! The `wem-vorbis` crate intentionally stays free of it.

use wem_analysis::model::PsyFrame;
use wem_vorbis::codebook::Codebook;
use wem_vorbis::floor_fit::floor1_fit_wwise_carriers;
use wem_vorbis::packet_encoder::{pack_block_packet, pack_block_packet_details, PacketError};
use wem_vorbis::setup::SetupInfo;

/// Packet bytes plus the unwrapped 10-bit floor posts used to create them
/// (Python `EncodedPacket`).
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedPacket {
    /// The analysis frame that produced the packet (Python `frame`).
    pub frame: PsyFrame,
    /// Per-channel 10-bit fit posts; `None` marks a floor-zero channel
    /// (Python `posts`).
    pub posts: Vec<Option<Vec<i64>>>,
    /// The encoded audio packet (Python `packet`).
    pub packet: Vec<u8>,
    /// The exact integer residue rows supplied to VQ (Python
    /// `quantized_residue`).
    pub quantized_residue: Vec<Vec<i64>>,
}

fn fit_floor_posts(
    setup: &SetupInfo,
    analysis: &PsyFrame,
    channels: u32,
) -> Result<Vec<Option<Vec<i64>>>, PacketError> {
    let ch = channels as usize;
    let row_counts = [
        analysis.post.len(),
        analysis.raw_mdct().len(),
        analysis.side.len(),
        analysis.coupling_peak.len(),
    ];
    if let Some(got) = row_counts.into_iter().find(|count| *count != ch) {
        return Err(PacketError::AnalysisChannelsMismatch { want: ch, got });
    }
    let mode = analysis.window().current() as u32;
    let mode_config = setup
        .modes
        .get(mode as usize)
        .ok_or(PacketError::ModeOutOfRange {
            mode,
            modes: setup.nmodes,
        })?;
    let mapping =
        setup
            .maps
            .get(mode_config.mapping as usize)
            .ok_or(PacketError::MappingOutOfRange {
                mapping: mode_config.mapping,
                maps: setup.nmaps,
            })?;
    let mut posts = Vec::with_capacity(ch);
    for (channel, (post_curve, raw_curve)) in analysis
        .post
        .iter()
        .zip(analysis.raw_mdct().iter())
        .enumerate()
    {
        let submap = if mapping.submaps > 1 {
            *mapping
                .chmux
                .get(channel)
                .ok_or(PacketError::ChannelMuxTooShort {
                    got: mapping.chmux.len(),
                    want: ch,
                })?
        } else {
            0
        };
        let floor_index =
            *mapping
                .floors
                .get(submap as usize)
                .ok_or(PacketError::FloorMapTooShort {
                    got: mapping.floors.len(),
                    want: mapping.submaps as usize,
                })?;
        let floor = setup
            .floors
            .get(floor_index as usize)
            .ok_or(PacketError::FloorIndexOutOfRange { index: floor_index })?;
        posts.push(
            floor1_fit_wwise_carriers(post_curve, raw_curve, floor, Some(raw_curve.len()))
                .map_err(PacketError::FloorFit)?,
        );
    }
    Ok(posts)
}

/// Fit floor1 curves and pack floor/residue for one analysis frame
/// (Python `pack_analysis_frame`).
///
/// The posts handed to the packet layer come from `floor1_fit_wwise` on the
/// analysis `post`/`raw_mdct` curves, exactly as in Python: `posts_are_10bit`
/// is set so the packet encoder applies the format's multiplier shift before
/// wrapping.
pub fn pack_analysis_frame(
    setup: &SetupInfo,
    books: &[Codebook],
    analysis: &PsyFrame,
    channels: u32,
) -> Result<EncodedPacket, PacketError> {
    let posts = fit_floor_posts(setup, analysis, channels)?;
    let mode = analysis.window().current() as u32;
    let packet_result = pack_block_packet_details(
        setup,
        books,
        channels,
        mode,
        &posts,
        &analysis.side,
        Some(&analysis.coupling_peak),
        true,
        true,
    )?;
    Ok(EncodedPacket {
        frame: analysis.clone(),
        posts,
        packet: packet_result.packet,
        quantized_residue: packet_result.quantized_residue,
    })
}

/// Pack one analysis frame for the production encoder without retaining the
/// diagnostic frame and residue snapshots returned by [`pack_analysis_frame`].
#[doc(hidden)]
pub fn pack_analysis_packet(
    setup: &SetupInfo,
    books: &[Codebook],
    analysis: &PsyFrame,
    channels: u32,
) -> Result<Vec<u8>, PacketError> {
    let posts = fit_floor_posts(setup, analysis, channels)?;
    pack_block_packet(
        setup,
        books,
        channels,
        analysis.window().current() as u32,
        &posts,
        &analysis.side,
        Some(&analysis.coupling_peak),
        true,
        true,
    )
}
