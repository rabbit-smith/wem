#![no_main]

use libfuzzer_sys::fuzz_target;
use wem_core::error::EncoderError;
use wem_core::usecases::wav::parse_pcm16;

fuzz_target!(|data: &[u8]| {
    match parse_pcm16(data) {
        Ok(wav) => {
            assert!(wav.channels() > 0);
            assert_eq!(wav.interleaved_le_bytes().len() % (wav.channels() * 2), 0);
            assert!(wav.to_pcm16().is_ok());
        }
        Err(error) => assert!(!matches!(error, EncoderError::Internal(_))),
    }
});
