//! Container contract: rebuild the Wwise WEM from the real captured packet
//! stream and verify the fmt/setup/data segment hashes and the full-file
//! SHA-256 against `tests/data/stage-golden/stages/index.json`.
//!
//! The packet stream (setup packet + 205 audio packets, seek table, fmt
//! fields, extra chunks) is captured from the reference-oracle encode path
//! via a subprocess: the stage-golden dumps only carry 28 representative
//! audio packets, so the full 206-packet stream is fetched on demand. The
//! capture drives the reference oracle directly (a test asset, not an
//! engine: the facade's single execution path is the native kernel and is
//! never on this path), observes the arguments of the reference-tree
//! `build_vorbis_wem` (`wwise_wem_reference.python_engine`). All
//! assertions are against the committed index.json hashes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use sha2::{Digest, Sha256};
use wem_container::{
    build_vorbis_wem, load_wem_parts_bytes, Endian, VorbisFmtFields,
};

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

fn stages_dir() -> PathBuf {
    repo_root().join("tests/data/stage-golden/stages")
}

fn sha256_hex(payload: &[u8]) -> String {
    let digest = Sha256::digest(payload);
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::new();
    for &b in digest.as_slice() {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0xF) as usize] as char);
    }
    out
}

fn load_index() -> Value {
    let raw = std::fs::read_to_string(stages_dir().join("index.json"))
        .expect("stage-golden index.json exists");
    serde_json::from_str(&raw).expect("index parses")
}

/// Python script: capture the reference-oracle encoder's build_vorbis_wem
/// arguments (packets / seek table / fmt fields / extra chunks) as JSON.
/// Mirrors tests/contract/stage_golden_support.py's observing wrapper.
const CAPTURE_SCRIPT: &str = r#"
import base64, json, sys
from pathlib import Path
repo = Path(sys.argv[1])
sys.path.insert(0, str(repo / "src"))
sys.path.insert(0, str(repo / "reference"))

import wwise_wem_reference.python_engine as python_engine
from wwise_wem_reference.container.model import ContainerPlan
from wwise_wem.adapters.wav import read_pcm16
from wwise_wem.profiles.registry import resolve_wem_profile

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
    return original_build(fmt_fields, packets, **kw)

python_engine.build_vorbis_wem = capture_build
try:
    pcm = read_pcm16(repo / "tests/fixtures/input.wav")
    profile = resolve_wem_profile(pcm.channel_count, pcm.sample_rate)
    python_engine.encode_pcm_python(
        profile=profile,
        container=ContainerPlan.from_profile(profile),
        pcm=pcm,
    )
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
}))
"#;

/// Run the Python capture helper and parse its JSON output.
fn capture_packet_stream() -> Value {
    let root = repo_root();
    let script = root.join("crates/wem-container/.capture_packets.py");
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

#[test]
fn container_contract_matches_stage_golden() {
    let index = load_index();
    let container = &index["container"];

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
    let i64f = |key: &str| -> u64 { ff[key].as_u64().unwrap_or_else(|| panic!("fmt field {key}")) };
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

    // Sanity: the captured setup packet is the golden setup packet.
    assert_eq!(
        sha256_hex(&packets[0]),
        container["setup_sha256"].as_str().unwrap(),
        "captured setup packet differs from index.json"
    );

    // Rebuild the container with the Rust kernel.
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

    // Segment hashes (fmt / data) and sizes against index.json.
    assert_eq!(
        sha256_hex(&built.fmt_raw),
        container["fmt_sha256"].as_str().unwrap(),
        "fmt segment hash differs"
    );
    assert_eq!(
        built.fmt_raw.len(),
        container["fmt_size"].as_u64().unwrap() as usize,
        "fmt segment size differs"
    );
    assert_eq!(
        sha256_hex(&built.data_raw),
        container["data_sha256"].as_str().unwrap(),
        "data segment hash differs"
    );
    assert_eq!(
        built.data_raw.len(),
        container["data_size"].as_u64().unwrap() as usize,
        "data segment size differs"
    );

    // Whole-file hash.
    assert_eq!(
        sha256_hex(&built.wem_bytes),
        container["wem_sha256"].as_str().unwrap(),
        "full WEM SHA-256 differs"
    );
    assert_eq!(
        built.wem_bytes.len(),
        container["wem_size"].as_u64().unwrap() as usize,
        "full WEM size differs"
    );

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
    assert_eq!(
        parts.fmt.dw_data_payload_size,
        built.data_raw.len() as u32
    );
    assert_eq!(
        parts.fmt.u_max_packet_size,
        packets.iter().map(|p| p.len()).max().unwrap() as u16
    );
}
