//! wwise-wem: convert between a signed-16 PCM WAV and a Wwise Vorbis WEM.
//!
//! Usage:
//!   `wwise-wem <input.wav> [--output <path>] [--wwise-version <label>] [--time]`
//!   `wwise-wem <input.wem> --decode [--output <path>] [--time]`
//!
//! * `--output` defaults to stdout (use "-" explicitly for stdout).
//! * `--wwise-version` selects the Wwise generation (default: the installed
//!   one, `2013`). The PCM geometry always comes from the input WAV, so the
//!   two together are the structured profile selection; there is no profile
//!   name and no profile directory.
//! * `--decode` reverses the direction: the input is a WEM and the output is
//!   an uncompressed signed-16 PCM WAV. A WEM is self-describing, so the
//!   encoder's options are refused rather than ignored, and there is no
//!   selection to make. The WAV is signed-16 PCM because that is the form the
//!   encode direction reads, so the two directions meet: the Python command
//!   line writes the same bytes for the same WEM.
//! * `--time` prints per-stage timings to stderr (bench/diagnostics).
//!
//! A one-line summary (bytes, frames, audio packets — or bytes, frames,
//! channels, sample rate when decoding) is written to stderr so the output
//! bytes on stdout or in the output file stay pristine. The byte count is the
//! length of what was written; for a digest of it, run the platform's tool
//! over the output file.
//!
//! Decoding writes its WAV only once the session has finished: a session the
//! decoder refuses part way through has delivered a *prefix* of the declared
//! frame count (`include/wem.h` section 5), and a WAV built from that prefix
//! would present it as a whole file. A refusal writes nothing and reports how
//! much of the stream had been decoded.

use std::path::Path;
use std::time::Instant;

use wem_core::decoder::{DecodeSession, DecodeStep, DecodedHeader};
use wem_core::error::DecoderError;
use wem_core::usecases::wav::read_pcm16;
use wem_core::{Encoder, Pcm16, WwiseProfile, WwiseVersion};

/// WEM bytes handed to the decode session per step.
///
/// Chunk boundaries never move a sample (`include/wem.h` section 5), so this
/// is pacing and not a parameter: bounded pushes hold one chunk of input and
/// one chunk's PCM instead of materializing the whole stream in one step. The
/// package facade pushes the same size (Python `_PUSH_BYTES`).
const PUSH_BYTES: usize = 8192;

fn main() {
    if let Err(message) = run() {
        eprintln!("wwise-wem: {message}");
        std::process::exit(1);
    }
}

fn usage() -> &'static str {
    "usage: wwise-wem <input.wav> [--output <path>] [--wwise-version <label>] [--time]\n       \
     wwise-wem <input.wem> --decode [--output <path>] [--time]"
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut version: Option<String> = None;
    let mut time_stages = false;
    let mut decode = false;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                i += 1;
                output = Some(args.get(i).cloned().ok_or_else(|| usage().to_string())?);
            }
            "--wwise-version" | "-V" => {
                i += 1;
                version = Some(args.get(i).cloned().ok_or_else(|| usage().to_string())?);
            }
            "--decode" => decode = true,
            "--time" => time_stages = true,
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(());
            }
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(format!("unknown option {other:?}"));
            }
            other => {
                if input.is_some() {
                    return Err(format!("unexpected argument {other:?}"));
                }
                input = Some(other.to_string());
            }
        }
        i += 1;
    }
    let input = input.ok_or_else(|| usage().to_string())?;

    if decode {
        if version.is_some() {
            return Err(
                "--wwise-version selects an encoder configuration; a WEM is \
                 self-describing and --decode has no selection to make"
                    .to_string(),
            );
        }
        return run_decode(&input, output.as_deref(), time_stages);
    }
    run_encode(&input, output.as_deref(), version.as_deref(), time_stages)
}

/// Encode one PCM WAV into a WEM.
fn run_encode(
    input: &str,
    output: Option<&str>,
    version: Option<&str>,
    time_stages: bool,
) -> Result<(), String> {
    let stage_start = Instant::now();
    let wav = read_pcm16(Path::new(input)).map_err(|error| error.to_string())?;
    let wav_load_ms = stage_start.elapsed().as_secs_f64() * 1e3;

    let generation = match version {
        Some(label) => WwiseVersion::parse(label).map_err(|error| error.to_string())?,
        None => WwiseVersion::DEFAULT,
    };
    let selection = WwiseProfile::new(generation, wav.channels() as i64, wav.sample_rate() as i64)
        .map_err(|error| error.to_string())?;
    let encoder = Encoder::new(selection).map_err(|error| error.to_string())?;
    let profile_assembly_ms = stage_start.elapsed().as_secs_f64() * 1e3 - wav_load_ms;

    let stage_start = Instant::now();
    let pcm: Pcm16 = wav.to_pcm16().map_err(|error| error.to_string())?;
    let result = encoder
        .encode_pcm(&pcm)
        .map_err(|error| error.to_string())?;
    let encode_ms = stage_start.elapsed().as_secs_f64() * 1e3;

    let stage_start = Instant::now();
    write_output(output, &result.data)?;
    let output_write_ms = stage_start.elapsed().as_secs_f64() * 1e3;

    if time_stages {
        eprintln!(
            "stages: wav_load {:.2}ms | profile_assembly {:.2}ms | encode {:.2}ms | output_write {:.2}ms",
            wav_load_ms, profile_assembly_ms, encode_ms, output_write_ms
        );
    }
    eprintln!(
        "wem-core: {} bytes ({} frames, {} audio packets)",
        result.len(),
        result.stats.pcm_frames,
        result.stats.audio_packets,
    );
    Ok(())
}

/// Decode one WEM into a signed-16 PCM WAV.
fn run_decode(input: &str, output: Option<&str>, time_stages: bool) -> Result<(), String> {
    let stage_start = Instant::now();
    let wem = std::fs::read(input).map_err(|error| format!("{input}: {error}"))?;
    let wem_load_ms = stage_start.elapsed().as_secs_f64() * 1e3;

    let stage_start = Instant::now();
    let decoded = decode_wem(&wem)?;
    let decode_ms = stage_start.elapsed().as_secs_f64() * 1e3;

    let stage_start = Instant::now();
    let wav = wav_writer::pcm16_wav(decoded.channels, decoded.sample_rate, &decoded.samples)?;
    let wav_assembly_ms = stage_start.elapsed().as_secs_f64() * 1e3;

    let stage_start = Instant::now();
    write_output(output, &wav)?;
    let output_write_ms = stage_start.elapsed().as_secs_f64() * 1e3;

    if time_stages {
        eprintln!(
            "stages: wem_load {:.2}ms | decode {:.2}ms | wav_assembly {:.2}ms | output_write {:.2}ms",
            wem_load_ms, decode_ms, wav_assembly_ms, output_write_ms
        );
    }
    eprintln!(
        "wem-core: {} bytes ({} frames, {} channels, {} Hz)",
        wav.len(),
        decoded.total_frames,
        decoded.channels,
        decoded.sample_rate,
    );
    Ok(())
}

/// Write the finished bytes to `output`, or to stdout when none was asked for
/// (or `-` was).
fn write_output(output: Option<&str>, data: &[u8]) -> Result<(), String> {
    match output {
        Some(path) if path != "-" => {
            std::fs::write(path, data).map_err(|error| format!("{path}: {error}"))
        }
        _ => {
            use std::io::Write;
            let mut stdout = std::io::stdout().lock();
            stdout
                .write_all(data)
                .and_then(|()| stdout.flush())
                .map_err(|error| error.to_string())
        }
    }
}

/// One decoded WEM: what the header announced, and the whole stream it declared.
#[derive(Debug)]
struct DecodedWem {
    channels: u32,
    sample_rate: u32,
    /// `dw_total_pcm_frames`, as the decode session announced it.
    total_frames: u64,
    /// Interleaved signed-16 samples, exactly `total_frames * channels` of them.
    samples: Vec<i16>,
}

/// Decode a whole WEM and map it into the signed-16 domain the WAV carries.
///
/// `Init -> push* -> Finish` over the whole container, the lifecycle
/// `include/wem.h` section 5 defines. A refusal returns its diagnostic and
/// nothing else: the frames the session completed before it are a prefix of
/// the declared stream, and handing that prefix to the writer would present it
/// as a whole file. The diagnostic says how much had been decoded, so a
/// truncated stream cannot be mistaken for a complete one.
fn decode_wem(bytes: &[u8]) -> Result<DecodedWem, String> {
    let mut session = DecodeSession::new();
    let mut header: Option<DecodedHeader> = None;
    let mut samples: Vec<i16> = Vec::new();
    let mut delivered: u64 = 0;

    for chunk in bytes.chunks(PUSH_BYTES) {
        let step = session.push_bytes(chunk);
        absorb(&step, &mut header, &mut samples, &mut delivered)?;
        if let Err(error) = &step.outcome {
            return Err(refusal(error, delivered, header.as_ref()));
        }
    }
    let step = session.finish();
    absorb(&step, &mut header, &mut samples, &mut delivered)?;
    if let Err(error) = &step.outcome {
        return Err(refusal(error, delivered, header.as_ref()));
    }

    let header = header.ok_or_else(|| {
        "the decode session delivered the whole stream without announcing a geometry".to_string()
    })?;
    if delivered != header.total_frames {
        // Unreachable through the kernel's own contract (a success delivers
        // exactly the declared count); checked rather than assumed, because
        // the WAV is built from what was delivered.
        return Err(format!(
            "the decode session delivered {delivered} frames but the container declares {}",
            header.total_frames
        ));
    }
    Ok(DecodedWem {
        channels: header.channels,
        sample_rate: header.sample_rate,
        total_frames: header.total_frames,
        samples,
    })
}

/// Take one step's output: the header announcement, and its PCM converted into
/// the signed-16 domain.
fn absorb(
    step: &DecodeStep,
    header: &mut Option<DecodedHeader>,
    samples: &mut Vec<i16>,
    delivered: &mut u64,
) -> Result<(), String> {
    if let Some(announced) = &step.header {
        if header.is_some() {
            return Err("the decode session announced its header twice".to_string());
        }
        // The announcement carries the declared frame count, so the buffer is
        // sized once here instead of growing per block.
        let frames = announced.total_frames as usize;
        let channels = announced.channels as usize;
        samples.reserve(frames.saturating_mul(channels));
        *header = Some(announced.clone());
    }
    if step.pcm.is_empty() {
        return Ok(());
    }
    let channels = step.channels as usize;
    if channels == 0 || !step.pcm.len().is_multiple_of(channels) {
        return Err(format!(
            "the decode session delivered {} samples for {channels} channels, which is not \
             a whole number of frames",
            step.pcm.len()
        ));
    }
    for (index, sample) in step.pcm.iter().enumerate() {
        let Some(value) = wav_writer::float_to_int16(*sample) else {
            return Err(format!(
                "decoded sample {index} of this step is {sample}, which has no signed-16 value"
            ));
        };
        samples.push(value);
    }
    *delivered += (step.pcm.len() / channels) as u64;
    Ok(())
}

/// The diagnostic one refused step produces.
///
/// The kernel's own text comes first and unchanged; what the CLI adds is what
/// it did with the outcome — nothing was written — and how much of the
/// declared stream the refusal left behind.
fn refusal(error: &DecoderError, delivered: u64, header: Option<&DecodedHeader>) -> String {
    match header {
        Some(header) => format!(
            "{error}; nothing written: the session was refused after {delivered} of {} \
             declared frames",
            header.total_frames
        ),
        None => format!("{error}; nothing written: the refusal came before any frame"),
    }
}

/// The signed-16 PCM WAV writer, and the one sample conversion it needs.
///
/// The decode direction's adapter, and the mirror of
/// `wem_core::usecases::wav`'s reader. It lives in this binary because the
/// kernel has no writer and this is adapter work, not kernel numerics: no
/// stage of the decode chain is re-implemented here, only a container built
/// around samples the chain already produced.
///
/// The conversion rule is the one `wwise_wem/adapters/sample_conversion.py`
/// states for the encode direction (`float_to_int16`), read in reverse, so the
/// Rust and Python command lines write the same bytes for the same decode:
/// scale by `32768.0`, round to nearest with ties away from zero, then
/// saturate to `[-32768, 32767]`.
mod wav_writer {
    /// `WAVE_FORMAT_PCM`: uncompressed signed-integer samples.
    const FORMAT_PCM: u16 = 1;
    /// Bits per sample. Sixteen is the width the encode direction's reader
    /// accepts, so a decoded file can be handed straight back to it.
    const BITS_PER_SAMPLE: u16 = 16;
    /// Bytes per sample at that width.
    const BYTES_PER_SAMPLE: u64 = (BITS_PER_SAMPLE / 8) as u64;
    /// The RIFF/WAVE header before the sample payload: `RIFF` + size + `WAVE`,
    /// the 16-byte `fmt ` chunk, and `data` + size.
    const HEADER_BYTES: u64 = 44;
    /// The largest `data` payload a RIFF size field can describe: that field
    /// counts 36 bytes of framing plus the payload.
    const MAX_DATA_BYTES: u64 = u32::MAX as u64 - (HEADER_BYTES - 8);

    /// Convert one decoded f32 sample into the signed-16 domain.
    ///
    /// `None` for a non-finite sample: a NaN or an infinity has no signed-16
    /// value, and inventing one here would be the silent truncation the
    /// standards forbid. Scaling by `2^15` is exact in either float width, so
    /// this is the same value the Python rule computes.
    pub fn float_to_int16(sample: f32) -> Option<i16> {
        if !sample.is_finite() {
            return None;
        }
        let scaled = f64::from(sample) * 32768.0;
        let rounded = if scaled > 0.0 {
            (scaled + 0.5).trunc()
        } else if scaled < 0.0 {
            -((-scaled) + 0.5).trunc()
        } else {
            0.0
        };
        Some(rounded.clamp(-32768.0, 32767.0) as i16)
    }

    /// Build one uncompressed signed-16 PCM WAV around `samples`.
    ///
    /// Interleaved and little-endian, `channels` samples per frame, with the
    /// channel count and sample rate the decode header announced. The frame
    /// count is the payload's own, so the header cannot describe data the file
    /// does not hold.
    pub fn pcm16_wav(channels: u32, sample_rate: u32, samples: &[i16]) -> Result<Vec<u8>, String> {
        if channels == 0 || channels > u16::MAX as u32 {
            return Err(format!(
                "the decode header announced {channels} channels, which the WAV header's \
                 16-bit channel field cannot describe"
            ));
        }
        if sample_rate == 0 {
            return Err("the decode header announced a zero sample rate".to_string());
        }
        let byte_rate = u64::from(sample_rate) * u64::from(channels) * BYTES_PER_SAMPLE;
        if byte_rate > u32::MAX as u64 {
            return Err(format!(
                "a {sample_rate} Hz {channels}-channel stream needs a byte rate of \
                 {byte_rate}, past the WAV header's 32-bit field"
            ));
        }
        let data_bytes = samples.len() as u64 * BYTES_PER_SAMPLE;
        if data_bytes > MAX_DATA_BYTES {
            return Err(format!(
                "the decoded PCM is {data_bytes} bytes, past the {MAX_DATA_BYTES}-byte \
                 limit a RIFF container can describe"
            ));
        }

        let frame_bytes = u64::from(channels) * BYTES_PER_SAMPLE;
        let mut out = Vec::with_capacity((HEADER_BYTES + data_bytes) as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((HEADER_BYTES - 8 + data_bytes) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&FORMAT_PCM.to_le_bytes());
        out.extend_from_slice(&(channels as u16).to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&(byte_rate as u32).to_le_bytes());
        out.extend_from_slice(&(frame_bytes as u16).to_le_bytes());
        out.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_bytes as u32).to_le_bytes());
        for sample in samples {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A committed fixture, by the path convention the `wem-core` suites use
    /// (`CARGO_MANIFEST_DIR` is `crates/wem-core`).
    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    #[test]
    fn the_conversion_is_the_rule_the_python_adapter_states() {
        // The boundaries `sample_conversion.float_to_int16` documents, plus
        // the ties-away-from-zero rounding on both sides of zero.
        assert_eq!(wav_writer::float_to_int16(0.0), Some(0));
        assert_eq!(wav_writer::float_to_int16(1.0), Some(32767));
        assert_eq!(wav_writer::float_to_int16(-1.0), Some(-32768));
        assert_eq!(wav_writer::float_to_int16(2.0), Some(32767));
        assert_eq!(wav_writer::float_to_int16(-2.0), Some(-32768));
        assert_eq!(wav_writer::float_to_int16(0.5 / 32768.0), Some(1));
        assert_eq!(wav_writer::float_to_int16(-0.5 / 32768.0), Some(-1));
        assert_eq!(wav_writer::float_to_int16(0.4999 / 32768.0), Some(0));
    }

    #[test]
    fn a_non_finite_sample_has_no_signed16_value() {
        assert_eq!(wav_writer::float_to_int16(f32::NAN), None);
        assert_eq!(wav_writer::float_to_int16(f32::INFINITY), None);
        assert_eq!(wav_writer::float_to_int16(f32::NEG_INFINITY), None);
    }

    #[test]
    fn the_wav_reads_back_through_the_kernels_own_reader() {
        // The writer and the encode side's reader are two halves of one
        // contract, and the reader is the older half: what it accepts is what
        // the writer must produce, samples included.
        let samples = [0i16, 1, -1, i16::MIN, i16::MAX, 12345, -12345, 300];
        let bytes = wav_writer::pcm16_wav(2, 44_100, &samples).expect("the WAV builds");
        assert_eq!(bytes.len(), 44 + samples.len() * 2);
        let wav = wem_core::usecases::wav::parse_pcm16(&bytes).expect("the reader accepts the WAV");
        assert_eq!(wav.channels(), 2);
        assert_eq!(wav.sample_rate(), 44_100);
        assert_eq!(wav.frames(), 4);
        let expected: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        assert_eq!(wav.interleaved_le_bytes(), &expected[..]);
    }

    #[test]
    fn the_header_is_a_canonical_44_byte_riff_header() {
        let bytes = wav_writer::pcm16_wav(6, 44_100, &[0; 12]).expect("the WAV builds");
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 24);
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 16);
        assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 6);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            44_100
        );
        assert_eq!(
            u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
            44_100 * 6 * 2
        );
        assert_eq!(u16::from_le_bytes(bytes[32..34].try_into().unwrap()), 12);
        assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 16);
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 24);
    }

    #[test]
    fn an_impossible_geometry_is_refused_rather_than_written() {
        assert!(wav_writer::pcm16_wav(0, 44_100, &[0, 0]).is_err());
        assert!(wav_writer::pcm16_wav(6, 0, &[0, 0]).is_err());
        assert!(wav_writer::pcm16_wav(u32::from(u16::MAX) + 1, 44_100, &[0]).is_err());
    }

    #[test]
    fn the_committed_fixture_decodes_into_a_wav_the_reader_accepts() {
        let wem = std::fs::read(fixture("reference.wem")).expect("the fixture reads");
        let decoded = decode_wem(&wem).expect("the committed container decodes");
        assert_eq!(decoded.channels, 6);
        assert_eq!(decoded.sample_rate, 44_100);
        assert_eq!(decoded.total_frames, 139_398);
        assert_eq!(decoded.samples.len(), 139_398 * 6);

        let wav = wav_writer::pcm16_wav(decoded.channels, decoded.sample_rate, &decoded.samples)
            .expect("the WAV builds");
        let read_back =
            wem_core::usecases::wav::parse_pcm16(&wav).expect("the reader accepts the WAV");
        assert_eq!(read_back.channels(), 6);
        assert_eq!(read_back.sample_rate(), 44_100);
        assert_eq!(read_back.frames(), 139_398);
        assert_eq!(
            read_back.interleaved_le_bytes().len(),
            decoded.samples.len() * 2
        );
    }

    #[test]
    fn a_truncated_container_is_refused_and_yields_no_wav() {
        // The refusal lands part way through, so the session has delivered a
        // prefix; `decode_wem` hands back no samples at all for it.
        let mut wem = std::fs::read(fixture("reference.wem")).expect("the fixture reads");
        wem.truncate(wem.len() / 2);
        let error = decode_wem(&wem).expect_err("a truncated container is refused");
        assert!(error.contains("nothing written"), "{error}");
        assert!(error.contains("declared frames"), "{error}");
        assert!(
            !error.contains("after 0 of"),
            "the fixture's first half decodes to a non-empty prefix: {error}"
        );
    }

    #[test]
    fn bytes_that_are_not_a_container_are_refused() {
        let error = decode_wem(b"NOTARIFF").expect_err("junk is refused");
        assert!(error.contains("nothing written"), "{error}");
        assert!(error.contains("came before any frame"), "{error}");
    }
}
