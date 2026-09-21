//! Encode interleaved signed-16 PCM through the Rust kernel directly.

use std::env;
use std::fs;

use wem_core::encoder::Pcm16;
use wem_core::{Encoder, WwiseProfile, WwiseVersion};

fn usage(program: &str, exit_code: i32) -> ! {
    eprintln!(
        "usage: {program} INPUT.pcm OUTPUT.wem VERSION SAMPLE_RATE CHANNELS\n\
         example: {program} input.pcm output.wem 2013 48000 2"
    );
    std::process::exit(exit_code);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args();
    let program = args.next().unwrap_or_else(|| "wem-rust-example".into());
    let args: Vec<String> = args.collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        usage(&program, 0);
    }
    let [input, output, version, sample_rate, channels]: [String; 5] =
        args.try_into().unwrap_or_else(|_| usage(&program, 2));

    // One structured selection: the Wwise generation plus the PCM geometry
    // (the input is header-less PCM, so the geometry is explicit). A
    // selection no compiled configuration satisfies is a hard error, never
    // a silently substituted default.
    let version = WwiseVersion::parse(&version)?;
    let sample_rate = sample_rate.parse::<i64>()?;
    let channels = channels.parse::<i64>()?;
    let selection = WwiseProfile::new(version, channels, sample_rate)?;

    let pcm = fs::read(input)?;
    let pcm = Pcm16::from_interleaved_le(sample_rate, usize::try_from(channels)?, &pcm)?;

    let encoder = Encoder::new(selection)?;
    let result = encoder.encode_pcm(&pcm)?;
    fs::write(&output, &result.data)?;

    println!(
        "wrote {} bytes to {output} ({} audio packets, sha256={})",
        result.data.len(),
        result.stats.audio_packets,
        result.sha256()
    );
    Ok(())
}
