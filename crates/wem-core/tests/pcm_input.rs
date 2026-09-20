use wem_core::{EncoderError, Pcm16};

#[test]
fn interleaved_pcm_rejects_unrepresentable_channel_count() {
    let error = Pcm16::from_interleaved_le(48_000, usize::MAX, &[0, 0])
        .expect_err("channel-byte multiplication must be checked");
    assert!(matches!(error, EncoderError::StateError { .. }));

    let error = Pcm16::from_interleaved_le(48_000, usize::MAX / 2, &[])
        .expect_err("empty PCM must be rejected before channel allocation");
    assert!(matches!(error, EncoderError::StateError { .. }));
}

#[test]
fn interleaved_pcm_rejects_zero_channels_and_partial_frames() {
    assert!(matches!(
        Pcm16::from_interleaved_le(48_000, 0, &[]),
        Err(EncoderError::StateError { .. })
    ));
    assert!(matches!(
        Pcm16::from_interleaved_le(48_000, 2, &[0, 0, 0]),
        Err(EncoderError::GeometryMismatch { .. })
    ));
}
