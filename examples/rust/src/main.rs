//! Encode interleaved signed-16 PCM through the Rust kernel directly.

use std::env;
use std::fs;
use std::path::PathBuf;

use wem_core::encoder::{DataDir, Encoder, Pcm16};

fn usage(program: &str) -> ! {
    eprintln!(
        "usage: {program} INPUT.pcm OUTPUT.wem PROFILE SAMPLE_RATE CHANNELS\n\
         example: {program} input.pcm output.wem wwise2013-2ch-48000 48000 2"
    );
    std::process::exit(2);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args();
    let program = args.next().unwrap_or_else(|| "wem-rust-example".into());
    let [input, output, profile, sample_rate, channels]: [String; 5] = args
        .collect::<Vec<_>>()
        .try_into()
        .unwrap_or_else(|_| usage(&program));

    let sample_rate = sample_rate.parse::<i64>()?;
    let channels = channels.parse::<usize>()?;
    let pcm = fs::read(input)?;
    let pcm = Pcm16::from_interleaved_le(sample_rate, channels, &pcm)?;

    let profiles =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/wwise_wem/data/profiles");
    let data_dir = DataDir::from_profiles_dir(profiles);
    let encoder = Encoder::from_profile_in(&data_dir, &profile)?;
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
