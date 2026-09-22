//! Decode one WEM into interleaved f32 PCM through the Rust kernel directly.
//!
//! There is no selection argument, unlike the encode direction: a WEM is
//! self-describing, and `DecodeSession` resolves the geometry from the
//! container's own header region. The output is the decoder's own samples,
//! written exactly as they arrive — interleaved little-endian f32 at +-1.0
//! full scale, no header — so the file is never a conversion this example
//! could get subtly wrong.

use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Write};

use wem_core::decoder::{DecodeSession, DecodeStep, DecodedHeader};

/// WEM bytes handed to the session per push. The step's reply carries every
/// frame that push completed, so bounded pushes hold one chunk of input and
/// one chunk's PCM instead of the whole stream's; the samples do not depend on
/// the chunking (`DecodeSession::push_bytes`).
const PUSH_BYTES: usize = 8192;

fn usage(program: &str, exit_code: i32) -> ! {
    eprintln!(
        "usage: {program} INPUT.wem OUTPUT.f32\n\
         example: {program} reference.wem out.f32\n\
         \n\
         OUTPUT.f32 is raw interleaved little-endian f32 at +-1.0 full scale,\n\
         with no header; the geometry and frame count are printed."
    );
    std::process::exit(exit_code);
}

/// What a decode session reports that a caller cannot recompose from the
/// samples alone: the one-time header announcement, and what has been written.
struct Decoded {
    header: Option<DecodedHeader>,
    steps_with_pcm: u64,
    frames: u64,
}

impl Decoded {
    fn new() -> Self {
        Self {
            header: None,
            steps_with_pcm: 0,
            frames: 0,
        }
    }

    /// Consume one step: take the header announcement if this is the step that
    /// carried it, write its samples as little-endian f32, and only then
    /// report a refusal. The kernel hands over the samples the earlier packets
    /// completed *and then* the code (the C ABI's `pcm_cb` / return-code
    /// order), so a refusal never drops the prefix it completed.
    fn consume(
        &mut self,
        step: DecodeStep,
        output: &mut impl Write,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(announced) = step.header {
            assert!(
                self.header.is_none(),
                "the header is announced exactly once per session"
            );
            self.header = Some(announced);
        }
        if !step.pcm.is_empty() {
            let channels = usize::try_from(
                self.header
                    .as_ref()
                    .ok_or("a step delivered PCM before the header announcement")?
                    .channels,
            )?;
            if !step.pcm.len().is_multiple_of(channels) {
                return Err(format!(
                    "a step delivered {} samples, not a whole number of {channels}-channel frames",
                    step.pcm.len()
                )
                .into());
            }
            let mut bytes = Vec::with_capacity(step.pcm.len() * 4);
            for sample in &step.pcm {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
            output.write_all(&bytes)?;
            self.steps_with_pcm += 1;
            self.frames += (step.pcm.len() / channels) as u64;
        }
        if let Err(error) = step.outcome {
            return Err(format!("decode refused: {error}").into());
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args();
    let program = args.next().unwrap_or_else(|| "decode_wem".into());
    let args: Vec<String> = args.collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        usage(&program, 0);
    }
    let [input, output]: [String; 2] = args.try_into().unwrap_or_else(|_| usage(&program, 2));

    // The WEM is read whole here because the input *is* a file; the session
    // itself is fed bounded chunks below.
    let wem = fs::read(&input)?;
    let mut session = DecodeSession::new();
    let mut output = BufWriter::new(File::create(&output)?);

    let mut decoded = Decoded::new();
    for chunk in wem.chunks(PUSH_BYTES) {
        let step = session.push_bytes(chunk);
        decoded.consume(step, &mut output)?;
    }
    // Finish is terminal whatever it returns: the session is dropped, not
    // reused (include/wem.h section 5). On WEM_OK it has delivered exactly the
    // frame count the container declares.
    let step = session.finish();
    decoded.consume(step, &mut output)?;
    output.flush()?;

    let header = decoded
        .header
        .ok_or("the decode session resolved no geometry")?;
    println!(
        "wrote {output} ({frames} frames, {channels} channels, {rate} Hz, \
         container declares {declared} frames, setup packet {setup} bytes, \
         {steps} steps with PCM)",
        output = &input,
        frames = decoded.frames,
        channels = header.channels,
        rate = header.sample_rate,
        declared = header.total_frames,
        setup = header.setup_packet.len(),
        steps = decoded.steps_with_pcm,
    );
    if decoded.frames != header.total_frames {
        return Err(format!(
            "delivered {} frames, container declares {}",
            decoded.frames, header.total_frames
        )
        .into());
    }
    Ok(())
}
