#![no_main]

use libfuzzer_sys::fuzz_target;
use wem_core::decoder::{DecodeSession, DecodeStep};
use wem_core::error::DecoderError;

fn accept(step: DecodeStep, samples: &mut Vec<u32>) -> bool {
    assert!(!matches!(step.outcome, Err(DecoderError::Internal(_))));
    if !step.pcm.is_empty() {
        assert_ne!(step.channels, 0);
        assert_eq!(step.pcm.len() % step.channels as usize, 0);
        assert!(step.pcm.iter().all(|sample| sample.is_finite()));
        samples.extend(step.pcm.iter().map(|sample| sample.to_bits()));
    }
    step.outcome.is_ok()
}

fn decode(data: &[u8], chunk: usize) -> Option<Vec<u32>> {
    let mut session = DecodeSession::new();
    let mut samples = Vec::new();
    for bytes in data.chunks(chunk) {
        if !accept(session.push_bytes(bytes), &mut samples) {
            return None;
        }
    }
    accept(session.finish(), &mut samples).then_some(samples)
}

fuzz_target!(|data: &[u8]| {
    // Both the batch container parser and incremental decoder see the same
    // untrusted bytes. Parse refusals are expected; panics and INTERNAL are not.
    drop(wem_container::load_wem_parts_bytes(data));
    let whole = decode(data, data.len().max(1));
    let chunked = decode(data, 1 + data.len() % 257);
    assert_eq!(
        whole, chunked,
        "chunk boundaries changed an accepted decode"
    );
});
