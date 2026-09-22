//! Streaming decode session: WEM bytes in, PCM out.
//!
//! The structural mirror of [`crate::stream::StreamSession`] with the data
//! direction reversed. Exactly one `Init` (the constructor), zero or more
//! `push`ed WEM byte chunks, then exactly one `Finish`. Every step reports what
//! it produced — the one-time header announcement and interleaved f32 PCM —
//! and how it ended; nothing panics on input, and no rejection silently
//! advances the stream.
//!
//! The lifecycle, the reply framing and the error codes are pinned for the
//! cross-language shells in `include/wem.h` (crates/AGENTS.md, "C ABI
//! surface"). This session is what `wem-capi`'s decoder entries wrap: the
//! callbacks live in the shell, so the kernel stays free of `extern "C"`.
//!
//! # The chain this assembles
//!
//! ```text
//! container framing (incremental)
//!   -> setup packet (parsed, then checked against the compiled carrier)
//!   -> per audio packet: mode and mapping -> floor1 posts -> residue
//!      coefficients (every residue type) -> mapping0 coupling inverse
//!      -> floor envelope
//!   -> inverse MDCT + hybrid synthesis window + overlap-add (wem-analysis)
//!   -> interleaved f32 PCM, exactly `dw_total_pcm_frames` frames
//! ```
//!
//! Every codec stage is wave 1's (`wem-vorbis`'s decode-direction segments and
//! `wem-analysis::dsp`'s inverse transform); this module is the orchestration
//! that sequences them and owns the container framing.
//!
//! # Input scope
//!
//! A WEM is self-describing, so the session takes no profile selection: the
//! geometry comes from the container's `fmt ` chunk, and the setup packet must
//! be the one the compiled carrier holds for that geometry. A WEM from another
//! Wwise generation is rejected because its setup packet is not that one — a
//! consequence of the carrier's authority, not a version test. There is no
//! version field in the container to test: `w_format_tag` is `0xFFFF` on every
//! sample examined.
//!
//! # Output length and alignment
//!
//! Exactly `dw_total_pcm_frames` frames are emitted, and output sample *i* is
//! the encoder's input sample *i*. The origin offset is one long half-block
//! (`blocksizes[1] / 2`, 1024 for both registered profiles); the scheduler
//! timeline's leading span is lead-in, and [`SynthesisOla`] already applies
//! the offset, so nothing here re-derives or fits it. The samples past
//! `dw_total_pcm_frames` are the encoder's own tail look-ahead and are
//! dropped, never delivered.
//!
//! # `Codebook::decode`: how the early end and the defect are told apart
//!
//! `Codebook::decode` reports a failed bit read and an unassigned tree branch
//! as the same error, and the decode-direction residue stage folds both into
//! [`ResidueStatus::EndOfPacket`], mirroring the reference decoder and
//! libvorbis. A session must not read that as "the packet ended" without
//! looking, or a real bitstream defect becomes a silent truncation.
//!
//! The policy here is:
//!
//! * An early end is **not** a decode failure. The format allows a packet to
//!   end inside its residue, so a session that refused every
//!   [`ResidueStatus::EndOfPacket`] would reject material the reference
//!   decoder decodes — strictly worse than the decoder it is measured
//!   against.
//! * It is **classified** instead, from the one piece of information the
//!   conflation does not destroy: the reader's residual bit count at the stop.
//!   A `Codebook::decode` that runs out of bits consumes every remaining bit
//!   before it fails; one that meets an unassigned branch has already consumed
//!   the offending bit while more remain. So `bits_left > 0` at the stop can
//!   only be an unassigned codeword, and it is reported as
//!   [`DecoderError::ResidueBitstreamDefect`] — the truncation stops being
//!   silent.
//! * The test is one-directional, and that asymmetry is stated rather than
//!   papered over: an unassigned branch taken on the packet's *very last* bit
//!   also leaves zero bits and is accepted as an early end. Closing that last
//!   bit would mean distinguishing the two inside `Codebook::decode`, which
//!   lives in wave 1's crate and is not this lane's to change.
//! * `CodebookError::EmptyCodebook` — a book with no usable codeword at all —
//!   is also folded into an early end, and it is a property of the *carrier's*
//!   codebook rather than of the bitstream, so the rule above would misreport
//!   it as malformed input. It cannot arise for a decodable container: the
//!   setup packet must be the carrier's, and no codebook either installed
//!   profile references is empty. That is pinned by a test over both
//!   profiles in `crates/wem-core/tests/decode_reference_wem.rs`, not assumed.
//!
//! # Incrementality and memory
//!
//! Only the container's *header region* — the RIFF framing, the chunk headers
//! and the `fmt ` payload — is buffered whole, because the data payload's
//! shape (its seek-table length) is only known once `fmt ` has been parsed.
//! Data-payload bytes that arrive after that are consumed as they arrive: a
//! packet is parsed and fed to the overlap-add as soon as its bytes are
//! complete, and the session retains one block awaiting its follower's mode
//! (bounded by the profile's long block size) plus the bytes of the packet it
//! is currently reading. Nothing proportional to the stream length is held —
//! with one ordering exception, stated rather than glossed: a container that
//! put its `data` chunk *before* its `fmt ` chunk cannot have its payload
//! interpreted until the `fmt ` arrives after it, so that payload is held until
//! then. Every container this repository's writer and the paired build produce
//! puts `fmt ` first, and the exception is a memory bound, never a wrong
//! sample.
//!
//! Three pieces of state are what a live session costs, and none of them grows
//! with the stream: the framing's unconsumed bytes (one packet, plus the
//! consumed prefix `ContainerStream::consume` has not compacted away yet), one
//! block awaiting its follower's mode, and one [`SynthesisOla`] window per
//! channel — which releases the finalized front of its timeline on every push,
//! so it holds at most one long block's worth of samples. The *delivered* PCM
//! is the caller's: each step's `pcm` is the reply, and nothing here keeps a
//! copy. `crates/wem-core/tests/decode_memory.rs` measures the whole of it as
//! the peak RSS of a child process decoding a stream 64x the fixture's length;
//! the encode side's equivalent check is
//! `crates/wem-core/tests/streaming.rs`. A bound stated in a comment and
//! measured in no test is how this one drifted, so both are named here.
//!
//! # Failures, and what they leave behind
//!
//! A rejection never advances the stream: the packet (or container region) a
//! step could not read stays pending and the overlap-add state is untouched,
//! so the session is still usable and a later step reports the same rejection
//! again. That keeps the encoder's rule for a rejection (include/wem.h,
//! "Panics": a rejection carrying any code but `WEM_ERR_INTERNAL` refuses the
//! call and leaves the handle usable) literally true. The step's own output is
//! still reported, so PCM completed before the refusal is delivered rather
//! than dropped. Only a defect ([`DecoderError::Internal`]) and `Finish` are
//! terminal.

use std::collections::HashMap;

use wem_analysis::config::MdctLook;
use wem_analysis::dsp::transform::SynthesisOla;
use wem_container::error::ContainerError;
use wem_container::fmt::{VorbisFmtFields, WWISE_VORBIS_FORMAT_TAG};
use wem_container::riff::{parse_chunks, Endian};
use wem_profiles::carrier::compiled_profile_for_selection;
use wem_profiles::codebooks::load_setup_codebooks;
use wem_profiles::error::ProfileError;
use wem_profiles::selection::{WwiseProfile, WwiseVersion};
use wem_vorbis::bitio::BitReader;
use wem_vorbis::codebook::Codebook;
use wem_vorbis::floor::{
    floor1_curve_from_posts, floor1_unwrap, postlist_from_floor, Floor1Error, FLOOR1_RANGES,
};
use wem_vorbis::packet_decoder::{
    coupling_dirty_nonzero, decode_floors_for_packet, parse_audio_header, PacketDecodeError,
};
use wem_vorbis::packet_encoder::apply_mapping_coupling_inverse;
use wem_vorbis::residue::{decode_residue_coeffs, ResidueStatus};
use wem_vorbis::setup::{parse_setup, Mapping0Setup, SetupInfo};

use crate::error::{DecoderError, InternalError};

/// The mode of the block after the last one: the encoder plans its own tail
/// with a long block (`SynthesisOla`'s `terminal_following`, and
/// `wem-scheduling`'s terminal transition). A decoder reads no window flags
/// from a packet in this format revision, so this is the format's fixed rule.
const TERMINAL_FOLLOWING: i64 = 1;

/// The one-time geometry and setup announcement of a decode session.
///
/// Mirrors the encoder's seq-0 setup delivery with the data direction
/// reversed: the geometry is available nowhere else in the output, which is
/// why the C ABI's `header_cb` is required rather than optional.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedHeader {
    /// PCM channel count the container declares.
    pub channels: u32,
    /// PCM sample rate the container declares.
    pub sample_rate: u32,
    /// `dw_total_pcm_frames`: the frame count the container declares, and so
    /// the exact number this session will deliver if it finishes with
    /// `WEM_OK`. It is announced here because the session has already read it
    /// to reach this point, and because the alternative is that every shell
    /// locates the container's `fmt` chunk and reads one field itself —
    /// container layout knowledge duplicated per shell, which the integration
    /// topology puts in the kernel.
    pub total_frames: u64,
    /// The setup packet this revision parsed, exactly as the container
    /// carried it.
    pub setup_packet: Vec<u8>,
}

/// What one decode step produced, and how it ended.
///
/// The step's output is reported whether or not it was refused: a rejection
/// stops the step at the packet it could not read, and the blocks earlier
/// packets completed are still real, so they are handed back rather than
/// dropped. Nothing was advanced past the failure — the stream position and
/// the overlap-add state are the ones the last accepted packet left.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodeStep {
    /// The header announcement, on the one step that resolved it.
    pub header: Option<DecodedHeader>,
    /// Interleaved f32 PCM completed by this step, at ±1.0 full scale.
    pub pcm: Vec<f32>,
    /// Channel count `pcm` is interleaved over (`0` before the header is
    /// known).
    pub channels: u32,
    /// `Ok(())` when the step consumed every byte it was given; the rejection
    /// that stopped it early otherwise.
    pub outcome: Result<(), DecoderError>,
}

impl DecodeStep {
    /// An empty step: nothing produced, nothing refused.
    fn idle() -> Self {
        Self {
            header: None,
            pcm: Vec::new(),
            channels: 0,
            outcome: Ok(()),
        }
    }

    /// PCM frames in [`DecodeStep::pcm`].
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.pcm.len() / self.channels as usize
        }
    }
}

/// The container's framing facts, once the header region is complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContainerHeader {
    fmt: VorbisFmtFields,
    /// Declared extent of the data payload, in bytes.
    data_size: usize,
}

/// The container framing state: bytes accumulate until the header region is
/// complete, then the data payload is consumed as it arrives.
#[derive(Debug, Default)]
struct ContainerStream {
    /// Bytes not yet consumed, preceded by the consumed prefix that has not
    /// been compacted away yet: `pending[cursor..]` is what is left to read.
    ///
    /// The prefix is dropped only when it is at least as large as what remains
    /// ([`ContainerStream::consume`]), which is what keeps the walk linear: a
    /// front `Vec::drain` per packet moves every unconsumed byte once per
    /// packet, so one push of a whole WEM costs O(packets x buffer) in
    /// `memmove` — 845 ms (49.9%) more than the same bytes pushed in 64 KiB
    /// chunks on a 7 MB WEM, and growing with the square of the push size
    /// (`docs/findings/decode-performance.md`, finding 3).
    pending: Vec<u8>,
    /// Bytes of `pending` already consumed, not yet compacted away.
    cursor: usize,
    endian: Endian,
    header: Option<ContainerHeader>,
    /// Data payload bytes not yet consumed.
    remaining: usize,
    /// Seek-table bytes still to skip.
    seek_left: usize,
    /// Whether the setup packet (packet 0) has been accepted.
    setup_taken: bool,
    /// Audio packets accepted so far (the setup packet is not one of them).
    audio_packets: u64,
}

/// One complete packet, peeked but not yet consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Packet {
    is_setup: bool,
    /// Index within the audio packet stream (meaningless for the setup).
    index: u64,
    payload: Vec<u8>,
}

impl ContainerStream {
    /// The bytes pushed but not consumed yet. The packet walk reads only this.
    fn unread(&self) -> &[u8] {
        &self.pending[self.cursor..]
    }

    /// Consume `count` bytes from the front of [`ContainerStream::unread`],
    /// releasing the consumed prefix once it is worth the move.
    ///
    /// The release threshold is the whole point: the prefix is dropped when it
    /// is at least as long as the bytes it would move, so the `memmove` cost of
    /// a consumed byte is at most one byte, whatever the push size. Pushing a
    /// whole WEM therefore costs O(bytes) rather than O(packets x bytes).
    fn consume(&mut self, count: usize) {
        self.cursor = (self.cursor + count).min(self.pending.len());
        if self.cursor >= self.pending.len() - self.cursor {
            self.pending.drain(..self.cursor);
            self.cursor = 0;
        }
    }

    /// Resolve the header region if the accumulated bytes complete it.
    ///
    /// Returns whether the region is resolved. `finishing` turns an incomplete
    /// region into a typed rejection instead of a wait.
    fn resolve_header(&mut self, finishing: bool) -> Result<bool, DecoderError> {
        if self.header.is_some() {
            return Ok(true);
        }
        if self.unread().len() < 12 {
            return self.shortfall(finishing, "the RIFF/WAVE header");
        }
        // The walk is deliberately the permissive one the container crate
        // exposes: a prefix of the container yields the chunks whose headers
        // are complete, with the last one's payload possibly partial.
        let (endian, chunks) = parse_chunks(self.unread())?;
        self.endian = endian;
        let fmt_chunk = chunks
            .iter()
            .find(|chunk| chunk.id == *b"fmt " && chunk.payload.len() == chunk.size as usize);
        let Some(fmt_chunk) = fmt_chunk else {
            return self.shortfall(finishing, "the fmt chunk");
        };
        let fmt = VorbisFmtFields::parse(&fmt_chunk.payload)?;
        if fmt.w_format_tag != WWISE_VORBIS_FORMAT_TAG {
            return Err(DecoderError::NotWwiseVorbis {
                format_tag: fmt.w_format_tag,
            });
        }
        let data_chunk = chunks.iter().find(|chunk| chunk.id == *b"data");
        let Some(data_chunk) = data_chunk else {
            return self.shortfall(finishing, "the data chunk");
        };
        let data_size = data_chunk.size as usize;
        let seek_left = fmt.dw_seek_table_size as usize;
        if seek_left > data_size {
            // The same shortfall the packet walk reports when the seek table
            // does not fit the payload it belongs to.
            return Err(ContainerError::BadSeekTableSize {
                seek_table_size: seek_left,
                data_size,
            }
            .into());
        }
        // Everything before the data payload is framing; only the payload is
        // streamed, and the walk's own accounting is what makes a short or
        // surplus payload visible.
        let drop = (data_chunk.off + 8).min(self.unread().len());
        self.consume(drop);
        self.remaining = data_size;
        self.seek_left = seek_left;
        self.header = Some(ContainerHeader { fmt, data_size });
        Ok(true)
    }

    /// The rejection an unresolved header region produces at `Finish`, or a
    /// plain "wait for more bytes" while pushing.
    fn shortfall(&self, finishing: bool, need: &'static str) -> Result<bool, DecoderError> {
        if finishing {
            return Err(DecoderError::Truncated {
                stream_offset: 0,
                need,
            });
        }
        Ok(false)
    }

    /// Bytes of the data payload consumed so far.
    fn consumed(&self) -> u64 {
        self.header
            .map(|header| (header.data_size - self.remaining) as u64)
            .unwrap_or(0)
    }

    /// The next complete packet, or `None` while more bytes are needed.
    ///
    /// The packet is *not* consumed: [`ContainerStream::commit_packet`] does
    /// that once the codec has accepted it, so a rejected packet leaves the
    /// stream exactly where it was.
    fn peek_packet(&mut self, finishing: bool) -> Result<Option<Packet>, DecoderError> {
        if self.header.is_none() {
            return Ok(None);
        }
        if self.seek_left > 0 {
            let skip = self.seek_left.min(self.unread().len());
            self.consume(skip);
            self.seek_left -= skip;
            self.remaining -= skip;
            if self.seek_left > 0 {
                return self.packet_shortfall(finishing, "the data payload's seek table");
            }
        }
        if self.remaining == 0 {
            return Ok(None);
        }
        if self.unread().len() < 2 {
            return self.packet_shortfall(finishing, "a packet size prefix");
        }
        let size = match self.endian {
            Endian::Little => u16::from_le_bytes([self.unread()[0], self.unread()[1]]),
            Endian::Big => u16::from_be_bytes([self.unread()[0], self.unread()[1]]),
        } as usize;
        if size + 2 > self.remaining {
            return self.packet_shortfall(finishing, "a packet payload inside the data payload");
        }
        if self.unread().len() < 2 + size {
            return self.packet_shortfall(finishing, "a packet payload");
        }
        Ok(Some(Packet {
            is_setup: !self.setup_taken,
            index: self.audio_packets,
            payload: self.unread()[2..2 + size].to_vec(),
        }))
    }

    /// Release a peeked packet, after the codec accepted it.
    fn commit_packet(&mut self, packet: &Packet) {
        self.consume(2 + packet.payload.len());
        self.remaining -= 2 + packet.payload.len();
        if packet.is_setup {
            self.setup_taken = true;
        } else {
            self.audio_packets += 1;
        }
    }

    /// A packet the payload promised but the pushed bytes never completed.
    fn packet_shortfall(
        &self,
        finishing: bool,
        need: &'static str,
    ) -> Result<Option<Packet>, DecoderError> {
        if finishing {
            return Err(DecoderError::Truncated {
                stream_offset: self.consumed(),
                need,
            });
        }
        Ok(None)
    }
}

/// One decoded block waiting for the *following* block's mode.
///
/// A block's hybrid synthesis window reads the size of the block after it, so
/// block `k` is pushed once packet `k + 1` has been accepted, and the last
/// block at [`DecodeSession::finish`].
struct PendingBlock {
    /// Mode of this block (0 = short, 1 = long).
    mode: i64,
    /// One `blocksizes[mode] / 2`-long coefficient row per channel.
    spectra: Vec<Vec<f64>>,
}

/// The resolved decode configuration: the compiled carrier's tables for one
/// container geometry.
struct Codec {
    channels: usize,
    sample_rate: u32,
    blocksizes: [i64; 2],
    setup: SetupInfo,
    books: Vec<Codebook>,
    /// The frozen MDCT trig bank per mode (0 = short, 1 = long).
    looks: [MdctLook; 2],
    /// The frozen Vorbis window halves, by block size.
    windows: HashMap<i64, Vec<f32>>,
    /// One overlap-add state per channel.
    ola: Vec<SynthesisOla>,
    /// The block awaiting its follower's mode.
    pending: Option<PendingBlock>,
    /// `dw_total_pcm_frames`: the exact number of frames to deliver.
    total_frames: u64,
    /// Frames delivered so far.
    emitted: u64,
}

impl Codec {
    /// Push one block into every channel's overlap-add and interleave the
    /// frames it completed, clamped to the container's declared count.
    fn push_block(
        &mut self,
        block: &PendingBlock,
        following: i64,
        pcm: &mut Vec<f32>,
    ) -> Result<(), DecoderError> {
        let Codec {
            channels,
            looks,
            windows,
            ola,
            total_frames,
            emitted,
            ..
        } = self;
        let look = &looks[block.mode as usize];
        // The overlap-add hands back exactly the samples this push finalized,
        // in order; those are what gets interleaved. Nothing here indexes a
        // timeline the state has already released, which is what keeps this
        // session's own footprint independent of the stream's length.
        let mut released = Vec::with_capacity(ola.len());
        for (channel, state) in ola.iter_mut().enumerate() {
            released.push(state.push(
                look,
                windows,
                block.mode,
                following,
                &block.spectra[channel],
            )?);
        }
        append_completed(&released, *channels, *total_frames, emitted, pcm)
    }

    /// Release the last block's tail (`Finish`).
    fn finish_blocks(&mut self, pcm: &mut Vec<f32>) -> Result<(), DecoderError> {
        if let Some(block) = self.pending.take() {
            self.push_block(&block, TERMINAL_FOLLOWING, pcm)?;
        }
        let Codec {
            channels,
            ola,
            total_frames,
            emitted,
            ..
        } = self;
        let mut released = Vec::with_capacity(ola.len());
        for state in ola.iter_mut() {
            released.push(state.finish());
        }
        append_completed(&released, *channels, *total_frames, emitted, pcm)
    }
}

/// Interleave what the overlap-add states released in one call, clamped to the
/// frames the container declares.
///
/// `released` is one slice per channel, each holding the samples that channel
/// finalized in the call that produced it (`SynthesisOla::push`'s or
/// `finish`'s return value). The slices are the samples: a released window is
/// not re-read from the state, because the state's own window is bounded and
/// its front is gone by the next push.
fn append_completed(
    released: &[&[f64]],
    channels: usize,
    total_frames: u64,
    emitted: &mut u64,
    pcm: &mut Vec<f32>,
) -> Result<(), DecoderError> {
    if released.len() != channels {
        return Err(DecoderError::Internal(InternalError::Invariant {
            message: "overlap-add state count differs from the container's channels",
        }));
    }
    let completed = released.first().map_or(0, |first| first.len());
    for slice in released {
        if slice.len() != completed {
            // Every channel is driven by the same mode sequence, so the
            // overlap-add advances in lockstep or an invariant is broken.
            return Err(DecoderError::Internal(InternalError::Invariant {
                message: "channel overlap-add states diverged",
            }));
        }
    }
    let room = total_frames.saturating_sub(*emitted);
    let allowed = room.min(completed as u64) as usize;
    for frame in 0..allowed {
        for slice in released {
            pcm.push(slice[frame] as f32);
        }
    }
    *emitted += allowed as u64;
    Ok(())
}

/// The whole decode session.
pub struct DecodeSession {
    finished: bool,
    stream: ContainerStream,
    codec: Option<Codec>,
}

impl Default for DecodeSession {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeSession {
    /// Open a decode session (`Init`).
    ///
    /// There is no selection argument: the WEM is self-describing, and the
    /// configuration is resolved from the container's own geometry once its
    /// `fmt ` chunk arrives.
    pub fn new() -> Self {
        Self {
            finished: false,
            stream: ContainerStream::default(),
            codec: None,
        }
    }

    /// Push one chunk of WEM bytes (`chunk*`).
    ///
    /// Chunk boundaries never affect the emitted samples: any chunking of the
    /// same WEM bytes emits the same PCM. An empty chunk is a no-op.
    ///
    /// **Chunking is not a time/memory trade on this surface — bounded pushes
    /// are both.** The framing holds what it is given until it is consumed, and
    /// this step's own reply carries every frame the call completed, so one
    /// push of a whole stream materializes that stream's whole PCM in the reply
    /// buffer before the caller sees any of it. A caller that pushes bounded
    /// chunks holds one chunk of input and one chunk's PCM instead. The walk
    /// itself adds nothing either way: it compacts the consumed prefix only
    /// when releasing it moves no more bytes than it frees, so its cost is
    /// linear in the bytes pushed whatever the push size
    /// (`docs/findings/decode-performance.md`, finding 3, for the measurement
    /// that used to be the other way round).
    ///
    /// `STATE_ERROR` after `Finish`. A rejection with any other code is not
    /// terminal: the session keeps the bytes it could not read and reports the
    /// same rejection again on the next step.
    pub fn push_bytes(&mut self, data: &[u8]) -> DecodeStep {
        if self.finished {
            return DecodeStep {
                outcome: Err(DecoderError::StateError {
                    message: "chunks are not allowed after Finish".into(),
                }),
                ..DecodeStep::idle()
            };
        }
        if data.is_empty() {
            return DecodeStep::idle();
        }
        self.stream.pending.extend_from_slice(data);
        let mut step = DecodeStep::idle();
        step.outcome = self.run(false, &mut step);
        step
    }

    /// Mark the end of the WEM bytes and complete the decode (`Finish`).
    ///
    /// `STATE_ERROR` when `Finish` already ran. Terminal whatever it returns:
    /// after this call the session must be dropped, not reused.
    pub fn finish(&mut self) -> DecodeStep {
        if self.finished {
            return DecodeStep {
                outcome: Err(DecoderError::StateError {
                    message: "Finish must come exactly once".into(),
                }),
                ..DecodeStep::idle()
            };
        }
        self.finished = true;
        let mut step = DecodeStep::idle();
        step.outcome = self.run(true, &mut step);
        step
    }

    /// Drive the container framing and the codec over the bytes available.
    fn run(&mut self, finishing: bool, step: &mut DecodeStep) -> Result<(), DecoderError> {
        loop {
            if !self.stream.resolve_header(finishing)? {
                return Ok(());
            }
            let Some(packet) = self.stream.peek_packet(finishing)? else {
                break;
            };
            if packet.is_setup {
                let codec = self.open_codec(&packet.payload)?;
                step.header = Some(DecodedHeader {
                    channels: codec.channels as u32,
                    sample_rate: codec.sample_rate,
                    total_frames: codec.total_frames,
                    setup_packet: packet.payload.clone(),
                });
                step.channels = codec.channels as u32;
                self.codec = Some(codec);
            } else {
                let index = packet.index;
                let codec = self.codec.as_mut().ok_or(DecoderError::MissingSetup {
                    data_size: self.stream.header.map(|h| h.data_size).unwrap_or(0),
                })?;
                let block = decode_audio_packet(&*codec, &packet.payload, index)?;
                // Commit: the previous block's following mode is now known.
                let mut pcm = Vec::new();
                if let Some(previous) = codec.pending.take() {
                    codec.push_block(&previous, block.mode, &mut pcm)?;
                }
                codec.pending = Some(block);
                step.pcm.extend_from_slice(&pcm);
                step.channels = codec.channels as u32;
            }
            self.stream.commit_packet(&packet);
        }
        if !finishing {
            return Ok(());
        }
        let Some(codec) = self.codec.as_mut() else {
            // The data payload ended without a setup packet: there is no
            // configuration to decode, whatever the frame count says.
            return Err(DecoderError::MissingSetup {
                data_size: self.stream.header.map(|h| h.data_size).unwrap_or(0),
            });
        };
        step.channels = codec.channels as u32;
        let mut pcm = std::mem::take(&mut step.pcm);
        // The tail is released first: the frames it holds are part of the
        // count the container declares, so the check below reads the count
        // *after* the last block has landed. (Written as a match rather than
        // `and`/`and_then` on purpose — `Result::and` would evaluate the check
        // eagerly, before `finish_blocks` ran, and report a short decode for a
        // complete one.)
        let tail = codec.finish_blocks(&mut pcm);
        let outcome = match tail {
            Ok(()) if codec.emitted == codec.total_frames => Ok(()),
            // The frame count the container declares is the contract: a stream
            // that synthesized fewer frames than it promised is malformed
            // input, never a short decode handed back as if it were complete.
            Ok(()) => Err(DecoderError::FrameCountMismatch {
                declared: codec.total_frames,
                synthesized: codec.emitted,
            }),
            Err(error) => Err(error),
        };

        step.pcm = pcm;
        outcome
    }

    /// Resolve the container's geometry and setup packet against the compiled
    /// carrier (the setup packet it holds for this geometry).
    fn open_codec(&self, setup_packet: &[u8]) -> Result<Codec, DecoderError> {
        let header = self.stream.header.ok_or(DecoderError::Truncated {
            stream_offset: 0,
            need: "the container header region",
        })?;
        let fmt = &header.fmt;
        // A setup packet that does not parse is malformed input, before any
        // question of which configuration it names.
        let channels = i64::from(fmt.n_channels);
        let sample_rate = i64::from(fmt.n_samples_per_sec);
        let setup =
            parse_setup(setup_packet, channels).map_err(|source| DecoderError::Setup { source })?;
        if !setup.parse_complete {
            return Err(DecoderError::SetupPadding {
                end_bit: setup.bit_positions.end,
                total_bits: setup.bits_total,
                pad_bits: setup.trailing_pad_bits,
                pad_value: setup.trailing_pad_value,
            });
        }
        let selection =
            WwiseProfile::new(WwiseVersion::DEFAULT, channels, sample_rate).map_err(|source| {
                DecoderError::ConfigurationUnsupported {
                    channels,
                    sample_rate,
                    source,
                }
            })?;
        let compiled =
            compiled_profile_for_selection(selection).map_err(|source| match source {
                // "Parses cleanly but names a configuration the carrier does not
                // hold": the unsupported-configuration class, never malformed
                // input and never a substituted default.
                ProfileError::Selection { .. } => DecoderError::ConfigurationUnsupported {
                    channels,
                    sample_rate,
                    source,
                },
                other => DecoderError::Internal(InternalError::Profile(other)),
            })?;
        let profile = compiled
            .encoder_profile()
            .map_err(|error| DecoderError::Internal(InternalError::Profile(error)))?;

        // The container's block-size fields and the resolved profile's must
        // agree: the pair is the profile's geometry, so a disagreement is
        // malformed input rather than an attempt to decode with the wrong
        // block sizes.
        let container = [
            block_size_from_power(fmt.u_blocksize0_pow)?,
            block_size_from_power(fmt.u_blocksize1_pow)?,
        ];
        let profile_sizes = profile.block_sizes();
        if container != profile_sizes {
            return Err(DecoderError::BlockSizeMismatch {
                container,
                profile: profile_sizes,
            });
        }

        // The setup packet must be the carrier's for this geometry. This is
        // the whole of the version question (see the module docs).
        let carried = profile
            .setup_packet()
            .map_err(|error| DecoderError::Internal(InternalError::Profile(error)))?;
        if carried != setup_packet {
            return Err(DecoderError::SetupNotCarried {
                container_len: setup_packet.len(),
                carried_len: carried.len(),
                first_difference: first_difference(setup_packet, &carried),
            });
        }

        let tables = compiled
            .book_tables()
            .map_err(|error| DecoderError::Internal(InternalError::Profile(error)))?;
        let books = load_setup_codebooks(&setup.book_ids, &tables)
            .map_err(|error| DecoderError::Internal(InternalError::Profile(error)))?;

        let looks = compiled
            .mdct_looks()
            .map_err(|error| DecoderError::Internal(InternalError::Profile(error)))?;
        let mut mode_looks = Vec::with_capacity(2);
        for size in profile_sizes {
            mode_looks.push(looks.get(&size).cloned().ok_or(DecoderError::Internal(
                InternalError::Invariant {
                    message: "the carrier holds no MDCT look for its own block size",
                },
            ))?);
        }
        let looks: [MdctLook; 2] = mode_looks.try_into().map_err(|_| {
            DecoderError::Internal(InternalError::Invariant {
                message: "the carrier's block geometry is not a short/long pair",
            })
        })?;
        let windows = compiled
            .frozen_tables()
            .map_err(|error| DecoderError::Internal(InternalError::Profile(error)))?
            .ok_or(DecoderError::Internal(InternalError::Invariant {
                message: "the carrier holds no frozen window tables",
            }))?
            .window_halves;
        for size in profile_sizes {
            if !windows.contains_key(&size) {
                return Err(DecoderError::Internal(InternalError::Invariant {
                    message: "the carrier holds no frozen window half for its own block size",
                }));
            }
        }
        let mut ola = Vec::with_capacity(fmt.n_channels as usize);
        for _ in 0..fmt.n_channels {
            ola.push(SynthesisOla::new(&profile_sizes)?);
        }
        Ok(Codec {
            channels: fmt.n_channels as usize,
            sample_rate: fmt.n_samples_per_sec,
            blocksizes: profile_sizes,
            setup,
            books,
            looks,
            windows,
            ola,
            pending: None,
            total_frames: u64::from(fmt.dw_total_pcm_frames),
            emitted: 0,
        })
    }
}

/// `1 << power` as a block size, rejecting a power this target cannot shift.
fn block_size_from_power(power: u8) -> Result<i64, DecoderError> {
    if power >= 63 {
        return Err(DecoderError::Internal(InternalError::Invariant {
            message: "container declares an unrepresentable block size",
        }));
    }
    Ok(1i64 << power)
}

/// First differing byte offset, when both packets are long enough to compare
/// that far.
fn first_difference(left: &[u8], right: &[u8]) -> Option<usize> {
    left.iter().zip(right.iter()).position(|(a, b)| a != b)
}

/// The submap one channel belongs to: the mapping's mux selects it when there
/// is more than one submap, and a single-submap mapping puts every channel in
/// submap 0 (the rule `wem-vorbis`'s floor reader applies too).
fn submap_of(mapping: &Mapping0Setup, channel: usize) -> usize {
    if mapping.submaps > 1 {
        mapping.chmux.get(channel).copied().unwrap_or(0) as usize
    } else {
        0
    }
}

/// Decode one audio packet into a block: header, floors, residue, coupling
/// inverse and floor envelope — the composite order libvorbis's
/// `mapping0_inverse` and this repository's reference decoder share.
fn decode_audio_packet(
    codec: &Codec,
    payload: &[u8],
    index: u64,
) -> Result<PendingBlock, DecoderError> {
    let mut br = BitReader::new(payload);
    let header = parse_audio_header(&mut br, &codec.setup)
        .map_err(|source| DecoderError::Packet { index, source })?;
    let mapping = codec
        .setup
        .maps
        .get(header.mapping as usize)
        .ok_or(DecoderError::Packet {
            index,
            source: PacketDecodeError::MappingOutOfRange {
                mapping: header.mapping,
                maps: codec.setup.maps.len(),
            },
        })?;
    let floors = decode_floors_for_packet(
        &mut br,
        &codec.setup,
        &codec.books,
        codec.channels as u32,
        mapping,
    )
    .map_err(|source| DecoderError::Packet { index, source })?;
    // Coupling makes both channels of a step live as soon as either floor is:
    // the pair is coded jointly, so the residue stage must see the propagated
    // flags (the encoder writes them that way).
    let dirty = coupling_dirty_nonzero(&floors.nonzero, &mapping.coupling)
        .map_err(|source| DecoderError::Packet { index, source })?;

    let mode = (header.blockflag & 1) as usize;
    let n_spectrum = (codec.blocksizes[mode] / 2) as usize;
    let packet_bits = (payload.len() as u64) * 8;
    let mut rows: Vec<Vec<f64>> = vec![vec![0.0f64; n_spectrum]; codec.channels];
    for submap in 0..mapping.submaps as usize {
        // Each submap codes the channels its mux selects, in the flat
        // `bin * bundle + position` domain; a single-submap mapping (both
        // registered profiles) is the whole channel set.
        let bundle: Vec<usize> = (0..codec.channels)
            .filter(|&channel| submap_of(mapping, channel) == submap)
            .collect();
        if bundle.is_empty() {
            continue;
        }
        let bundle_used: Vec<bool> = bundle.iter().map(|&channel| dirty[channel]).collect();
        let residue_id = *mapping
            .residues
            .get(submap)
            .ok_or_else(|| DecoderError::Packet {
                index,
                source: PacketDecodeError::FloorMapTooShort {
                    got: mapping.residues.len(),
                    want: submap + 1,
                },
            })?;
        let residue = if residue_id < codec.setup.residues.len() as u64 {
            &codec.setup.residues[residue_id as usize]
        } else {
            return Err(DecoderError::Packet {
                index,
                source: PacketDecodeError::ResidueIndexOutOfRange {
                    index: residue_id,
                    residues: codec.setup.residues.len(),
                },
            });
        };
        let (bundle_rows, status) =
            decode_residue_coeffs(&mut br, residue, &codec.books, &bundle_used, n_spectrum)
                .map_err(|source| DecoderError::Packet {
                    index,
                    source: PacketDecodeError::Residue {
                        submap,
                        residue_id,
                        source,
                    },
                })?;
        if status == ResidueStatus::EndOfPacket {
            // The early end is kept (the format allows it), but not without
            // looking: bits left at the stop can only be an unassigned
            // codeword (see the module docs).
            let bit_position = br.tell_bits();
            if packet_bits.saturating_sub(bit_position) > 0 {
                return Err(DecoderError::ResidueBitstreamDefect {
                    index,
                    submap,
                    bit_position,
                    packet_bits,
                });
            }
        }
        for (position, &channel) in bundle.iter().enumerate() {
            rows[channel] = bundle_rows[position].clone();
        }
    }

    // The four-branch libvorbis mapping0 inverse — the exact pre-image of the
    // encoder's `apply_mapping_coupling` — applied to the residue domain
    // before the floor envelope. Which inverse is right is decided by the
    // mapping, never by the name.
    apply_mapping_coupling_inverse(&mut rows, &mapping.coupling)
        .map_err(|error| DecoderError::Internal(InternalError::Packet(error)))?;

    let mut spectra = Vec::with_capacity(codec.channels);
    // The row is taken per channel and the floor is read from the setup, so
    // the loop walks channels rather than rows: `rows` is indexed by the
    // channel the mapping's mux selected, which is not the iteration order of
    // any single collection here.
    #[allow(clippy::needless_range_loop)]
    for channel in 0..codec.channels {
        let submap = submap_of(mapping, channel);
        let floor_index = *mapping.floors.get(submap).ok_or(DecoderError::Packet {
            index,
            source: PacketDecodeError::FloorMapTooShort {
                got: mapping.floors.len(),
                want: submap + 1,
            },
        })?;
        let floor = codec
            .setup
            .floors
            .get(floor_index as usize)
            .ok_or(DecoderError::Packet {
                index,
                source: PacketDecodeError::FloorIndexOutOfRange {
                    index: floor_index,
                    floors: codec.setup.floors.len(),
                },
            })?;
        let mut row = std::mem::take(&mut rows[channel]);
        if let Some(posts) = floors.curves[channel].as_ref() {
            let postlist = postlist_from_floor(floor);
            let range =
                *FLOOR1_RANGES
                    .get(floor.multiplier as usize)
                    .ok_or(DecoderError::Floor1 {
                        channel,
                        source: Floor1Error::InvalidMultiplier {
                            multiplier: floor.multiplier,
                        },
                    })?;
            let absolute = floor1_unwrap(posts, &postlist, range as i64)
                .map_err(|source| DecoderError::Floor1 { channel, source })?;
            let curve = floor1_curve_from_posts(&absolute, &postlist, n_spectrum, floor.multiplier)
                .map_err(|source| DecoderError::Floor1 { channel, source })?;
            for (value, envelope) in row.iter_mut().zip(curve.iter()) {
                *value *= envelope;
            }
        }
        spectra.push(row);
    }
    Ok(PendingBlock {
        mode: mode as i64,
        spectra,
    })
}
