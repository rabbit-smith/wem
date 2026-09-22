//! The measurement worker `scripts/measure_concurrency.py` starts N copies of.
//!
//! One process, one encode, one line on stdout. The script starts N copies as
//! close together as the spawn syscall allows, reaps each with `os.wait4` for
//! its own `ru_utime`/`ru_stime`, and reads the encode stage printed here. It is
//! the matrix's **only** instrument, so its two arms are one program built
//! twice — `cargo test --release -p wem-core --test concurrency_worker --no-run`
//! with and without `--features parallel` — and no arm can differ from the other
//! by its entry point or its startup.
//!
//! The parallel arm used to be the CLI. This worker is the instrument instead
//! because the CLI is a bin target that *requires* the `parallel` feature
//! (`crates/wem-core/Cargo.toml`), so it does not exist in the scalar
//! configuration at all, and a matrix whose two arms cannot both be built is
//! not a comparison.
//!
//! Contract with the script (both sides move together):
//!
//! | variable | meaning |
//! |---|---|
//! | `WEM_CONCURRENCY_WAV` | the input WAV (required) |
//! | `WEM_CONCURRENCY_OUTPUT` | where the container goes (default `/dev/null`) |
//!
//! and one stdout line, `wem_concurrency_worker encode_ms=<ms> load_ms=<ms>
//! write_ms=<ms> bytes=<n>`.
//!
//! It asserts nothing about time — the caller owns the statistics and the
//! script has no thresholds — and the only check is that the encode succeeded
//! and produced a container.
//!
//! Almost always this never runs: `cargo test --workspace --all-targets`
//! reports the worker without executing it, and the script runs it with
//! `--ignored --exact concurrency_worker --nocapture`.
//!
//! Release only: a debug build is 10–30× slower and says nothing about the
//! shipped artifact, so the worker prints a skip notice and returns.

use std::path::PathBuf;
use std::time::Instant;

use wem_core::usecases::wav::read_pcm16;
use wem_core::{Encoder, Pcm16, WwiseProfile, WwiseVersion};

#[test]
#[ignore = "worker: scripts/measure_concurrency.py runs it with --ignored --exact"]
fn concurrency_worker() {
    if cfg!(debug_assertions) {
        eprintln!("concurrency_worker: release only (cargo test --release …)");
        return;
    }

    let wav_path = PathBuf::from(
        std::env::var("WEM_CONCURRENCY_WAV").expect("WEM_CONCURRENCY_WAV names the input WAV"),
    );
    let output = std::env::var("WEM_CONCURRENCY_OUTPUT").unwrap_or_else(|_| "/dev/null".into());

    // The same steps the CLI takes, in the same order: read the WAV, resolve the
    // one structured selection from its geometry, assemble the profile, encode,
    // write. Nothing here is instrumented; the two `Instant`s bracket the work
    // the script reports.
    let start = Instant::now();
    let wav = read_pcm16(&wav_path).expect("the measurement WAV reads");
    let selection = WwiseProfile::new(
        WwiseVersion::DEFAULT,
        wav.channels() as i64,
        wav.sample_rate() as i64,
    )
    .expect("the WAV geometry names an installed configuration");
    let encoder = Encoder::new(selection).expect("the selected profile assembles");
    let pcm: Pcm16 = wav
        .to_pcm16()
        .expect("the PCM converts to the signed-16 domain");
    let load_ms = start.elapsed().as_secs_f64() * 1e3;

    let start = Instant::now();
    let result = encoder.encode_pcm(&pcm).expect("the encode succeeds");
    let encode_ms = start.elapsed().as_secs_f64() * 1e3;

    let start = Instant::now();
    result.write_to(&output).expect("the container writes");
    let write_ms = start.elapsed().as_secs_f64() * 1e3;

    println!(
        "wem_concurrency_worker encode_ms={encode_ms:.3} load_ms={load_ms:.3} \
         write_ms={write_ms:.3} bytes={}",
        result.len()
    );
}
