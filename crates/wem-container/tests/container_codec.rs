//! The container builder: its parity with the oracle, and its refusal shapes.
//!
//! `parity` captures one live packet stream and compares three containers byte
//! for byte — the kernel's, the Python oracle's and the committed reference —
//! naming the first differing byte of the segment a divergence falls in.
//! `degenerate` hands the same builder payloads and geometries it cannot
//! represent, and requires a `ContainerError` rather than a panic. Each module
//! keeps its own test names and assertion messages.

mod parity {
    //! Container bytes: the native kernel's WEM against the pure-Python oracle's,
    //! built at test time from the same captured packet stream, and both against
    //! the committed reference container.
    //!
    //! The packet stream (setup packet + 205 audio packets, seek table, fmt
    //! fields, extra chunks) is captured live from the reference-oracle encode
    //! path via a subprocess, which observes the arguments of the reference-tree
    //! `build_vorbis_wem` (`wwise_wem_reference.python_engine`). The capture drives
    //! the reference oracle directly (a test asset, not an engine: the facade's
    //! single execution path is the native kernel and is never on this path).
    //!
    //! Both container builders then run at test time on those arguments — the
    //! Python one inside the capture subprocess, the Rust `wem_container::
    //! build_vorbis_wem` here — and the two whole containers are compared byte for
    //! byte. The same bytes are then compared against the committed
    //! `tests/fixtures/reference.wem`, which is what makes the claim about the
    //! shipped artifact rather than only about two implementations agreeing.
    //! Nothing is recorded and no side is compared against a stored digest: a
    //! divergence is a divergence, and it names the first differing byte of the
    //! segment it falls in.

    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use serde_json::Value;
    use wem_container::{build_vorbis_wem, load_wem_parts_bytes, Endian, VorbisFmtFields};

    fn repo_root() -> PathBuf {
        // crates/wem-container -> repo root (two levels up from the manifest
        // dir).
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates dir")
            .parent()
            .expect("repo root")
            .to_path_buf()
    }

    /// The committed reference container for `tests/fixtures/input.wav`.
    fn reference_wem() -> Vec<u8> {
        std::fs::read(repo_root().join("tests/fixtures/reference.wem"))
            .expect("committed reference container reads")
    }

    /// Python script: capture the reference-oracle encoder's build_vorbis_wem
    /// arguments (packets / seek table / fmt fields / extra chunks) as JSON, plus
    /// the container bytes the oracle's own builder returns for them.
    const CAPTURE_SCRIPT: &str = r#"
import base64, json, sys
from pathlib import Path
repo = Path(sys.argv[1])
sys.path.insert(0, str(repo / "src"))
sys.path.insert(0, str(repo / "reference"))

import wwise_wem
import wwise_wem_reference.python_engine as python_engine
from wwise_wem_reference.container.model import ContainerPlan
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem_reference.profiles.artifact import resolve_selection

captured = {}
original_build = python_engine.build_vorbis_wem

def capture_build(fmt_fields, packets, **kw):
    captured["fmt_fields"] = {k: int(v) for k, v in dict(fmt_fields).items()}
    captured["packets"] = [bytes(p) for p in packets]
    captured["seek_table"] = bytes(kw.get("seek_table", b""))
    captured["endian"] = kw.get("endian", "le")
    captured["extra_chunks"] = [
        (bytes(c[0]), bytes(c[1])) for c in (kw.get("extra_chunks") or [])
    ]
    captured["recompute_sizes"] = kw.get("recompute_sizes", True)
    captured["fmt_raw"] = kw.get("fmt_raw")
    wem_bytes = original_build(fmt_fields, packets, **kw)
    # The oracle builder's own output for exactly these arguments: the
    # comparison target, produced at test time rather than recorded.
    captured["wem_bytes"] = bytes(wem_bytes)
    return wem_bytes

python_engine.build_vorbis_wem = capture_build
try:
    pcm = read_pcm_wav(repo / "tests/fixtures/input.wav")
    profile = resolve_selection(
        wwise_wem.WwiseProfile(
            wwise_wem.WwiseVersion.WWISE2013, pcm.channel_count, pcm.sample_rate
        )
    )
    result = python_engine.encode_pcm_python(
        profile=profile,
        container=ContainerPlan.from_profile(profile),
        pcm=pcm,
    )
    captured["encode_data"] = bytes(result.data)
finally:
    python_engine.build_vorbis_wem = original_build

def b64(b):
    return base64.b64encode(b).decode("ascii") if b is not None else None

print(json.dumps({
    "fmt_fields": captured["fmt_fields"],
    "packets": [b64(p) for p in captured["packets"]],
    "seek_table": b64(captured["seek_table"]),
    "endian": captured["endian"],
    "extra_chunks": [[b64(c), b64(p)] for c, p in captured["extra_chunks"]],
    "recompute_sizes": captured["recompute_sizes"],
    "fmt_raw": b64(captured["fmt_raw"]),
    "wem_bytes": b64(captured["wem_bytes"]),
    "encode_data": b64(captured["encode_data"]),
}))
"#;

    /// Run the Python capture helper and parse its JSON output.
    fn capture_packet_stream() -> Value {
        let root = repo_root();
        // The helper is a run-time artifact, so it lives in the system temp
        // directory: a crashed test must not leave it inside the checkout. The
        // name is unique per process and per call, so parallel tests cannot
        // overwrite each other's script.
        static SEQUENCE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let script = std::env::temp_dir().join(format!(
            "wem-capture-packets-{}-{sequence}.py",
            std::process::id()
        ));
        std::fs::write(&script, CAPTURE_SCRIPT).expect("write capture script");
        #[cfg(unix)]
        let list_separator = ":";
        #[cfg(not(unix))]
        let list_separator = ";";
        let pythonpath = format!(
            "{}{}{}",
            root.join("src").display(),
            list_separator,
            root.join("reference").display()
        );
        let output = Command::new("python3")
            .current_dir(&root)
            .env("PYTHONPATH", pythonpath)
            .arg(script.as_os_str())
            .arg(root.as_os_str())
            .output();
        let _ = std::fs::remove_file(&script);
        let output: Output = output.expect("python3 available");
        if !output.status.success() {
            panic!(
                "python capture failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        serde_json::from_slice(&output.stdout).expect("capture JSON parses")
    }

    fn b64_decode(value: &Value) -> Vec<u8> {
        base64_decode(value.as_str().expect("base64 string"))
    }

    fn base64_decode(text: &str) -> Vec<u8> {
        // Compact base64 decoder (the only dependency-free way to read the
        // capture output; mirrors the alphabet of Python's standard encoder).
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let mut buf: u32 = 0;
        let mut nbits = 0u32;
        for b in text.bytes() {
            if b == b'=' {
                break;
            }
            let idx = ALPHABET
                .iter()
                .position(|&a| a == b)
                .expect("valid base64 character");
            buf = (buf << 6) | idx as u32;
            nbits += 6;
            if nbits >= 8 {
                nbits -= 8;
                out.push((buf >> nbits) as u8);
                buf &= (1 << nbits) - 1;
            }
        }
        out
    }

    /// Assert two byte strings are equal, naming the first differing offset.
    fn assert_bytes_equal(what: &str, kernel: &[u8], other: &[u8]) {
        if kernel == other {
            return;
        }
        let at = kernel
            .iter()
            .zip(other)
            .position(|(kernel_byte, other_byte)| kernel_byte != other_byte)
            .unwrap_or_else(|| kernel.len().min(other.len()));
        let byte = |payload: &[u8]| match payload.get(at) {
            Some(value) => format!("0x{value:02x}"),
            None => "-- (past the end)".to_string(),
        };
        panic!(
            "{what}: kernel {at} bytes in, other {}; first difference at byte {at}: \
         kernel {} != other {}",
            other.len(),
            byte(kernel),
            byte(other),
        );
    }

    #[test]
    fn container_matches_the_oracle_and_the_reference_bytes() {
        let reference = reference_wem();
        let reference_parts =
            load_wem_parts_bytes(&reference).expect("the committed reference container parses");

        let captured = capture_packet_stream();
        let endian = if captured["endian"].as_str() == Some("be") {
            Endian::Big
        } else {
            Endian::Little
        };
        let seek_table = b64_decode(&captured["seek_table"]);
        let packets: Vec<Vec<u8>> = captured["packets"]
            .as_array()
            .unwrap()
            .iter()
            .map(b64_decode)
            .collect();
        assert_eq!(packets.len(), 206, "setup + 205 audio packets");
        let extra_chunks: Vec<([u8; 4], Vec<u8>)> = captured["extra_chunks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pair| {
                let id_bytes = b64_decode(&pair[0]);
                let mut id = [0u8; 4];
                id.copy_from_slice(&id_bytes);
                (id, b64_decode(&pair[1]))
            })
            .collect();
        let recompute_sizes = captured["recompute_sizes"].as_bool().unwrap();
        let fmt_raw = captured["fmt_raw"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| b64_decode(&Value::String(s.to_string())));

        // Build VorbisFmtFields from the captured (pre-recompute) fields.
        let ff = &captured["fmt_fields"];
        let i64f = |key: &str| -> u64 {
            ff[key]
                .as_u64()
                .unwrap_or_else(|| panic!("fmt field {key}"))
        };
        let fields = VorbisFmtFields {
            w_format_tag: i64f("wFormatTag") as u16,
            n_channels: i64f("nChannels") as u16,
            n_samples_per_sec: i64f("nSamplesPerSec") as u32,
            n_avg_bytes_per_sec: i64f("nAvgBytesPerSec") as u32,
            n_block_align: i64f("nBlockAlign") as u16,
            w_bits_per_sample: i64f("wBitsPerSample") as u16,
            cb_size: i64f("cbSize") as u16,
            w_reserved0: i64f("wReserved0") as u16,
            dw_channel_mask: i64f("dwChannelMask") as u32,
            dw_total_pcm_frames: i64f("dwTotalPCMFrames") as u32,
            dw_first_audio_packet_offset: i64f("dwFirstAudioPacketOffset") as u32,
            dw_data_payload_size: i64f("dwDataPayloadSize") as u32,
            dw_unknown_0x24: i64f("dwUnknown_0x24") as u32,
            dw_seek_table_size: i64f("dwSeekTableSize") as u32,
            dw_vorbis_data_offset: i64f("dwVorbisDataOffset") as u32,
            u_max_packet_size: i64f("uMaxPacketSize") as u16,
            u_unknown_0x32: i64f("uUnknown_0x32") as u16,
            dw_unknown_0x34: i64f("dwUnknown_0x34") as u32,
            dw_unknown_0x38: i64f("dwUnknown_0x38") as u32,
            dw_unknown_0x3c: i64f("dwUnknown_0x3C") as u32,
            u_blocksize0_pow: i64f("uBlocksize0Pow") as u8,
            u_blocksize1_pow: i64f("uBlocksize1Pow") as u8,
        };

        // The oracle's own container for exactly these arguments, produced by the
        // capture subprocess at test time.
        let oracle_wem = b64_decode(&captured["wem_bytes"]);
        let oracle_whole_file = b64_decode(&captured["encode_data"]);
        assert_bytes_equal(
            "the oracle's capture agrees with its own whole-file encode",
            &oracle_wem,
            &oracle_whole_file,
        );
        let oracle_parts = load_wem_parts_bytes(&oracle_wem).expect("the oracle container parses");
        assert_eq!(
            oracle_parts.setup_packet.as_deref(),
            Some(packets[0].as_slice()),
            "the captured setup packet is the oracle container's"
        );
        // Sanity: the captured stream is the committed container's own — its setup
        // packet and every audio packet, byte for byte.
        assert_eq!(
            reference_parts.setup_packet.as_deref(),
            Some(packets[0].as_slice()),
            "the captured setup packet must be the reference container's"
        );
        assert_eq!(
            reference_parts.audio_packets,
            packets[1..].to_vec(),
            "the captured audio packets must be the reference container's"
        );

        // Build the same container with the Rust kernel, from the same arguments.
        let built = build_vorbis_wem(
            fields,
            &packets,
            &seek_table,
            endian,
            &extra_chunks,
            recompute_sizes,
            fmt_raw.as_deref(),
        )
        .expect("build_vorbis_wem succeeds");

        // The whole claim: one container, built twice, byte for byte.
        assert_bytes_equal("the built container", &built.wem_bytes, &oracle_wem);
        assert_bytes_equal("the fmt segment", &built.fmt_raw, &oracle_parts.fmt_raw);
        assert_bytes_equal("the data segment", &built.data_raw, &oracle_parts.data_raw);

        // And the same bytes are the committed reference container's: the segment
        // comparisons name a divergence more precisely, the whole-file comparison
        // is the shipped-artifact claim.
        assert_bytes_equal("the fmt segment", &built.fmt_raw, &reference_parts.fmt_raw);
        assert_bytes_equal(
            "the data segment",
            &built.data_raw,
            &reference_parts.data_raw,
        );
        assert_bytes_equal("the built container", &built.wem_bytes, &reference);

        // Round-trip: the structural parser sees the expected layout.
        let parts = load_wem_parts_bytes(&built.wem_bytes).expect("parts parse");
        assert!(parts.is_wwise_vorbis);
        assert_eq!(parts.fmt_raw, built.fmt_raw);
        assert_eq!(parts.data_raw, built.data_raw);
        assert_eq!(parts.setup_packet, Some(packets[0].clone()));
        assert_eq!(parts.audio_packets, packets[1..].to_vec());
        assert_eq!(
            parts.fmt.dw_first_audio_packet_offset,
            seek_table.len() as u32 + 2 + packets[0].len() as u32
        );
        assert_eq!(parts.fmt.dw_data_payload_size, built.data_raw.len() as u32);
        assert_eq!(
            parts.fmt.u_max_packet_size,
            packets.iter().map(|p| p.len()).max().unwrap() as u16
        );
    }
}

mod degenerate {
    //! Degenerate inputs to the public container surface: a payload that cannot
    //! be interpreted comes back as a `ContainerError`, never as a panic.

    use wem_container::{
        build_riff, build_vorbis_wem, load_wem_parts_bytes, recompute_vorbis_fmt_sizes,
        ContainerError, Endian, VorbisFmtFields,
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
}
