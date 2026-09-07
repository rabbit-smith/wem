//! wwise-wem: encode a signed-16 PCM WAV into a Wwise Vorbis WEM.
//!
//! Usage:
//!   wwise-wem <input.wav> [--output <path>] [--profile <name>] [--time]
//!
//! * `--output` defaults to stdout (use "-" explicitly for stdout).
//! * `--profile` selects an installed profile by name; without it, the
//!   profile is resolved from the WAV's channel count / sample rate
//!   (Python `api.py::encode_wav` default path).
//! * `--time` prints per-stage timings to stderr (bench/diagnostics).
//!
//! A one-line summary (bytes, sha256, stats) is written to stderr so the
//! encoded bytes on stdout or in the output file stay pristine.

use std::path::Path;
use std::time::Instant;

use wem_core::usecases::wav::read_pcm16;
use wem_core::{resolve_profile_by_geometry, Encoder, Pcm16};

fn main() {
    if let Err(message) = run() {
        eprintln!("wwise-wem: {message}");
        std::process::exit(1);
    }
}

fn usage() -> &'static str {
    "usage: wwise-wem <input.wav> [--output <path>] [--profile <name>] [--time]"
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut profile: Option<String> = None;
    let mut time_stages = false;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                i += 1;
                output = Some(args.get(i).cloned().ok_or_else(|| usage().to_string())?);
            }
            "--profile" | "-p" => {
                i += 1;
                profile = Some(args.get(i).cloned().ok_or_else(|| usage().to_string())?);
            }
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

    let stage_start = Instant::now();
    let wav = read_pcm16(Path::new(&input)).map_err(|error| error.to_string())?;
    let wav_load_ms = stage_start.elapsed().as_secs_f64() * 1e3;

    let encoder = match profile {
        Some(name) => Encoder::from_profile(&name).map_err(|error| error.to_string())?,
        None => resolve_profile_by_geometry(wav.channels() as i64, wav.sample_rate())
            .map_err(|error| error.to_string())?,
    };
    let profile_assembly_ms = stage_start.elapsed().as_secs_f64() * 1e3 - wav_load_ms;

    let stage_start = Instant::now();
    let pcm: Pcm16 = wav.to_pcm16().map_err(|error| error.to_string())?;
    let result = encoder
        .encode_pcm(&pcm)
        .map_err(|error| error.to_string())?;
    let encode_ms = stage_start.elapsed().as_secs_f64() * 1e3;

    let stage_start = Instant::now();
    match output {
        Some(path) if path != "-" => {
            std::fs::write(&path, &result.data).map_err(|error| format!("{path}: {error}"))?;
        }
        _ => {
            use std::io::Write;
            let mut stdout = std::io::stdout().lock();
            stdout
                .write_all(&result.data)
                .and_then(|()| stdout.flush())
                .map_err(|error| error.to_string())?;
        }
    }
    let output_write_ms = stage_start.elapsed().as_secs_f64() * 1e3;

    if time_stages {
        eprintln!(
            "stages: wav_load {:.2}ms | profile_assembly {:.2}ms | encode {:.2}ms | output_write {:.2}ms",
            wav_load_ms, profile_assembly_ms, encode_ms, output_write_ms
        );
    }
    eprintln!(
        "wem-core: {} bytes, sha256 {} ({} frames, {} audio packets)",
        result.data.len(),
        result.sha256(),
        result.stats.pcm_frames,
        result.stats.audio_packets,
    );
    Ok(())
}
