//! Frame-level packet assembly for one analysis frame.
//!
//! Rust-side equivalent of Python `wwise_wem/vorbis/packet_encoder.py::
//! pack_analysis_frame`, relocated to the orchestration layer (wem-core)
//! because it bridges analysis output (`PsyFrame`) with Vorbis packing —
//! the per-channel post/raw assembly step of the Python application layer.
//! The `wem-vorbis` crate intentionally stays free of it.

use wem_analysis::model::PsyFrame;
use wem_vorbis::codebook::Codebook;
use wem_vorbis::floor_fit::floor1_fit_wwise;
use wem_vorbis::packet_encoder::{pack_block_packet_details, PacketError};
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
    let ch = channels as usize;
    if analysis.post.len() != ch || analysis.side.len() != ch {
        return Err(PacketError::AnalysisChannelsMismatch {
            want: ch,
            got: analysis.post.len(),
        });
    }
    let mode = analysis.window().current() as u32;
    let mapping = &setup.maps[setup.modes[mode as usize].mapping as usize];
    let mut posts: Vec<Option<Vec<i64>>> = Vec::with_capacity(ch);
    for (channel, (post_curve, raw_curve)) in analysis
        .post
        .iter()
        .zip(analysis.raw_mdct().iter())
        .enumerate()
    {
        let submap = if mapping.submaps > 1 {
            mapping.chmux[channel]
        } else {
            0
        };
        let floor = &setup.floors[mapping.floors[submap as usize] as usize];
        // The analysis rows carry f32 values in f64 carriers; recover the
        // exact f32 bit patterns for the fit (Python float boundary).
        let post_f32: Vec<f32> = post_curve.iter().map(|value| *value as f32).collect();
        let raw_f32: Vec<f32> = raw_curve.iter().map(|value| *value as f32).collect();
        posts.push(
            floor1_fit_wwise(&post_f32, &raw_f32, floor, Some(raw_f32.len()))
                .map_err(PacketError::FloorFit)?,
        );
    }
    let mdct: Vec<Vec<f32>> = analysis
        .side
        .iter()
        .map(|row| row.iter().map(|value| *value as f32).collect())
        .collect();
    let packet_result =
        pack_block_packet_details(setup, books, channels, mode, &posts, &mdct, true, true)?;
    Ok(EncodedPacket {
        frame: analysis.clone(),
        posts,
        packet: packet_result.packet,
        quantized_residue: packet_result.quantized_residue,
    })
}
