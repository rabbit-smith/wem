//! Degenerate inputs to the public container surface: a payload that cannot
//! be interpreted comes back as a `ContainerError`, never as a panic.

use wem_container::{
    build_riff, build_vorbis_wem, load_wem_parts_bytes, recompute_vorbis_fmt_sizes, ContainerError,
    Endian, VorbisFmtFields,
};

/// A `fmt ` payload without a format tag cannot be classified. `parse_chunks`
/// accepts any `fmt ` chunk size, so this is the first read of the payload:
/// it reports the short size instead of indexing past it.
#[test]
fn fmt_payload_without_format_tag_reports_too_short() {
    for fmt_len in [0usize, 1] {
        let chunks: Vec<([u8; 4], Vec<u8>)> =
            vec![(*b"fmt ", vec![0u8; fmt_len]), (*b"data", Vec::new())];
        let raw = build_riff(&chunks, Endian::Little, false).expect("riff builds");
        // 12-byte RIFF/WAVE header + 8-byte fmt header + payload (+ pad) +
        // 8-byte empty data header: 28 bytes, or the 30-byte shape when the
        // odd fmt payload is word-padded.
        assert_eq!(raw.len(), if fmt_len == 0 { 28 } else { 30 });

        let err = load_wem_parts_bytes(&raw).expect_err("a short fmt payload is rejected");
        assert_eq!(err, ContainerError::FmtTooShort { got: fmt_len });
    }
}

/// Long-mode audio packets render `(2048 + 2048) / 4` frames per window.
fn long_mode_packets(audio_packets: usize) -> Vec<Vec<u8>> {
    let mut packets = vec![b"setup".to_vec()];
    packets.extend(std::iter::repeat_n(vec![1u8], audio_packets));
    packets
}

fn long_mode_fields() -> VorbisFmtFields {
    VorbisFmtFields {
        dw_total_pcm_frames: 0,
        u_blocksize0_pow: 8,
        u_blocksize1_pow: 11,
        ..VorbisFmtFields::DEFAULTS
    }
}

/// The terminal overlap excess is stored in a u16 twice; an excess that does
/// not fit is reported instead of leaving both derived fields stale.
#[test]
fn terminal_excess_beyond_u16_is_reported() {
    let packets = long_mode_packets(200);
    let refs: Vec<&[u8]> = packets.iter().map(|p| p.as_slice()).collect();
    // 199 windows * (2048 + 2048) / 4 = 203776 rendered frames, no PCM total.
    assert_eq!(
        recompute_vorbis_fmt_sizes(&mut long_mode_fields(), &refs, b""),
        Err(ContainerError::TerminalExcessTooLarge { excess: 203_776 })
    );
}

/// The same derivation still succeeds when the excess fits u16.
#[test]
fn terminal_excess_within_u16_is_written() {
    let packets = long_mode_packets(4);
    let refs: Vec<&[u8]> = packets.iter().map(|p| p.as_slice()).collect();
    let mut fields = VorbisFmtFields {
        dw_total_pcm_frames: 2304,
        ..long_mode_fields()
    };
    recompute_vorbis_fmt_sizes(&mut fields, &refs, b"").expect("excess fits u16");
    // 3 windows * 1024 - 2304 PCM frames.
    assert_eq!(fields.u_unknown_0x32, 768);
    assert_eq!(fields.dw_unknown_0x24, 768 << 16);
}

/// The public builder surfaces the same error rather than emitting a WEM whose
/// derived fields were never written.
#[test]
fn build_vorbis_wem_reports_terminal_excess_beyond_u16() {
    let packets = long_mode_packets(200);
    let err = build_vorbis_wem(
        long_mode_fields(),
        &packets,
        b"",
        Endian::Little,
        &[],
        true,
        None,
    )
    .expect_err("an unrepresentable overlap excess is rejected");
    assert_eq!(
        err,
        ContainerError::TerminalExcessTooLarge { excess: 203_776 }
    );
}
