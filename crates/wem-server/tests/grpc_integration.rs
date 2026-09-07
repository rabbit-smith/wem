//! wwise.v1 gRPC integration tests: a real tonic client talks to the real
//! `WemEncoderService` on an ephemeral loopback port, over the unmodified
//! wem-core kernel.
//!
//! Semantic correspondence with `scripts/interop_grpc_smoke.py` (the
//! cross-language smoke driving this same server binary):
//! * the interop smoke drives this *tonic* server through a Python gRPC
//!   client with the same lifecycle (Init -> chunk* -> Finish) and asserts
//!   that the WEM container reassembled from the received `Packet` stream
//!   is byte-identical to the reference golden;
//! * these tests run the same assertions from within Rust with the same
//!   golden fixture: the reassembled container bytes must equal
//!   `tests/fixtures/reference.wem` byte-for-byte (SHA-256
//!   `17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247`),
//!   with packets consumed in strict seq order and exactly one
//!   `WemComplete` at the end;
//! * negative paths shared by both: a wrong `setup_sha256` must be
//!   `PROFILE_NOT_FOUND`; every lifecycle violation (this suite) must be
//!   a single terminal `STATE_ERROR` reply — the kernel never panics.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio_stream::StreamExt;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};
use tonic::Request;
use wem_container::{build_vorbis_wem, load_wem_parts_bytes};
use wem_core::usecases::wav::read_pcm16;
use wem_server::service::WemEncoderService;
use wem_server::wwise::{
    encode_request, encode_response, init, wem_encoder_client::WemEncoderClient,
    wem_encoder_server::WemEncoderServer, EncodeRequest, EncodeResponse, EncoderErrorCode, Finish,
    Init, ListProfilesRequest, PcmChunk, PcmFormat, PcmFrames, PcmSampleLayout, ProfileKey,
    ProfileRef, TemplateRef,
};

/// Golden WEM container for tests/fixtures/input.wav (kernel contract).
const GOLDEN_WEM_SHA256: &str = "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247";
const PROFILE_NAME: &str = "wwise2013-6ch-44100";
const SETUP_SHA256: &str = "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3";
const MANIFEST_SHA256: &str = "f36981acf01807117015d2cac64fa995a3352710e9c4a56ab72b05d9b05dfc44";
const WWISE_GENERATION: &str = "2013.2";

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .canonicalize()
        .expect("fixtures directory resolves")
}

async fn start_server() -> WemEncoderClient<Channel> {
    let incoming =
        TcpIncoming::bind("127.0.0.1:0".parse().expect("loopback addr")).expect("loopback bind");
    let addr = incoming.local_addr().expect("listener address");
    tokio::spawn(async move {
        Server::builder()
            .add_service(WemEncoderServer::new(WemEncoderService::new()))
            .serve_with_incoming(incoming)
            .await
            .expect("server runs");
    });
    let channel = Channel::from_shared(format!("http://{addr}"))
        .expect("channel scheme")
        .connect()
        .await
        .expect("channel connects");
    WemEncoderClient::new(channel)
}

fn init_request(setup_sha256: &str, name: &str) -> EncodeRequest {
    EncodeRequest {
        payload: Some(encode_request::Payload::Init(Init {
            r#ref: Some(init::Ref::Profile(ProfileRef {
                setup_sha256: setup_sha256.to_string(),
                name: name.to_string(),
            })),
        })),
    }
}

fn chunk_request(channels: u32, sample_rate: u32, layout: i32, data: Vec<u8>) -> EncodeRequest {
    EncodeRequest {
        payload: Some(encode_request::Payload::Chunk(PcmChunk {
            format: Some(PcmFormat {
                channels,
                sample_rate,
                layout,
            }),
            frames: Some(PcmFrames { data }),
        })),
    }
}

fn finish_request() -> EncodeRequest {
    EncodeRequest {
        payload: Some(encode_request::Payload::Finish(Finish {})),
    }
}

/// Send the request stream and drain every reply (a terminal reply is
/// always the last one; the stream then closes).
async fn collect(
    client: &mut WemEncoderClient<Channel>,
    requests: Vec<EncodeRequest>,
) -> Vec<EncodeResponse> {
    let mut stream = client
        .encode(tokio_stream::iter(requests))
        .await
        .expect("encode stream opens")
        .into_inner();
    let mut replies = Vec::new();
    while let Some(response) = stream.next().await {
        replies.push(response.expect("reply ok"));
    }
    replies
}

fn expect_error(replies: &[EncodeResponse], code: i32, what: &str) {
    let last = replies
        .last()
        .unwrap_or_else(|| panic!("{what}: no reply at all"));
    match &last.payload {
        Some(encode_response::Payload::Error(error)) => {
            assert_eq!(
                error.code,
                code,
                "{what}: wrong error code: {} ('{}')",
                EncoderErrorCode::try_from(error.code)
                    .map(|c| c.as_str_name().to_string())
                    .unwrap_or_else(|_| format!("{code}")),
                error.message
            );
        }
        other => panic!("{what}: expected terminal EncoderError, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// ListProfiles — the version handshake
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_profiles_reports_profile_identity_and_handshake_digests() {
    let mut client = start_server().await;

    let unfiltered = client
        .list_profiles(Request::new(ListProfilesRequest::default()))
        .await
        .expect("list profiles (unfiltered)")
        .into_inner();
    assert_eq!(
        unfiltered.profiles.len(),
        1,
        "exactly one installed profile"
    );
    let profile = &unfiltered.profiles[0];
    assert_eq!(profile.name, PROFILE_NAME);
    assert_eq!(profile.setup_sha256, SETUP_SHA256);
    assert_eq!(profile.manifest_sha256, MANIFEST_SHA256);
    assert_eq!(profile.wwise_generation, WWISE_GENERATION);
    assert_eq!(profile.supported_formats.len(), 1);
    let format = &profile.supported_formats[0];
    assert_eq!(format.channels, 6);
    assert_eq!(format.sample_rate, 44100);
    assert_eq!(format.layout, PcmSampleLayout::Signed16Interleaved as i32);

    let hit = client
        .list_profiles(Request::new(ListProfilesRequest {
            key: Some(ProfileKey {
                channels: 6,
                sample_rate: 44100,
            }),
        }))
        .await
        .expect("list profiles (geometry hit)")
        .into_inner();
    assert_eq!(hit.profiles.len(), 1);

    let miss = client
        .list_profiles(Request::new(ListProfilesRequest {
            key: Some(ProfileKey {
                channels: 2,
                sample_rate: 8000,
            }),
        }))
        .await
        .expect("list profiles (geometry miss)")
        .into_inner();
    assert!(miss.profiles.is_empty(), "geometry filter must exclude");
}

// ---------------------------------------------------------------------------
// Encode — happy path: 7 uneven chunks, byte identity with reference.wem
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stream_encode_rebuilds_reference_wem_byte_for_byte() {
    let fixtures = fixtures_dir();
    let reference = std::fs::read(fixtures.join("reference.wem")).expect("reference.wem reads");
    let wav = read_pcm16(&fixtures.join("input.wav")).expect("input.wav reads");
    let pcm = wav.interleaved_le_bytes();
    let channels = wav.channels() as u32;
    let sample_rate = wav.sample_rate() as u32;
    let layout = PcmSampleLayout::Signed16Interleaved as i32;

    // Seven uneven, frame-aligned chunk sizes (the last absorbs the
    // remainder so the split tiles the stream exactly).
    let frame_counts: Vec<usize> = {
        let fixed = [1000usize, 25000, 5, 30000, 1, 4096];
        let sum: usize = fixed.iter().sum();
        let remainder = wav.frames() - sum;
        assert!(
            remainder > 4096 && remainder != 1000 && remainder != 25000,
            "fixture must outgrow the fixed chunks (remainder {remainder})"
        );
        let mut counts: Vec<usize> = fixed.to_vec();
        counts.push(remainder);
        counts
    };
    assert_eq!(frame_counts.len(), 7);

    let mut requests: Vec<EncodeRequest> = Vec::new();
    requests.push(init_request(SETUP_SHA256, PROFILE_NAME));
    let mut offset = 0usize;
    for &frames in &frame_counts {
        let bytes = frames * channels as usize * 2;
        requests.push(chunk_request(
            channels,
            sample_rate,
            layout,
            pcm[offset..offset + bytes].to_vec(),
        ));
        offset += bytes;
    }
    assert_eq!(offset, pcm.len(), "chunk split must tile the stream");
    requests.push(finish_request());

    let mut client = start_server().await;

    let replies = collect(&mut client, requests).await;

    // 1) Strict reply shape: packets in seq order, then one WemComplete.
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut complete: Option<(usize, wem_server::wwise::WemComplete)> = None;
    for (index, reply) in replies.iter().enumerate() {
        match &reply.payload {
            Some(encode_response::Payload::Packet(packet)) => {
                assert_eq!(
                    packet.seq as usize,
                    packets.len(),
                    "reply {index}: packet seq out of order"
                );
                packets.push(packet.data.clone());
            }
            Some(encode_response::Payload::Complete(summary)) => {
                assert!(complete.is_none(), "duplicate WemComplete");
                complete = Some((index, summary.clone()));
            }
            Some(encode_response::Payload::Error(error)) => {
                panic!("unexpected terminal error in happy path: {error:?}");
            }
            None => panic!("reply {index}: empty EncodeResponse"),
        }
    }
    let (complete_index, complete) = complete.expect("WemComplete present");
    assert_eq!(
        complete_index,
        replies.len() - 1,
        "WemComplete must be the last reply"
    );

    // 2) Container summary matches the golden reference exactly.
    assert_eq!(
        complete.total_len as usize,
        reference.len(),
        "total_len must equal the reference container size"
    );
    assert_eq!(
        complete.sha256, GOLDEN_WEM_SHA256,
        "server-reported sha256 must equal the golden WEM hash"
    );
    assert_eq!(
        complete.inline_bytes, reference,
        "inline_bytes must equal reference.wem byte-for-byte (<= 2 MiB)"
    );

    // 3) Reassemble the container from the *received packet stream* the
    // way the reference semantics demand: profile fmt + packet sequence +
    // seek table -> WEM bytes == reference.wem.
    let parts = load_wem_parts_bytes(&reference).expect("reference WEM parses");
    let rebuilt = build_vorbis_wem(
        parts.fmt,
        &packets,
        &parts.seek_table,
        parts.endian,
        &[],
        true,
        None,
    )
    .expect("rebuild runs");
    assert_eq!(
        rebuilt.wem_bytes, reference,
        "WEM rebuilt from the gRPC packet stream must equal reference.wem"
    );
    assert_eq!(
        sha256_hex(&rebuilt.wem_bytes),
        sha256_hex(&reference),
        "rebuilt sha256 must match the reference sha256"
    );

    // 4) The packet stream itself must replay the container's packet
    // layout (seq 0 = setup, then audio in encoding order).
    let mut expected_packets: Vec<Vec<u8>> = Vec::new();
    if let Some(setup) = parts.setup_packet {
        expected_packets.push(setup);
    }
    expected_packets.extend(parts.audio_packets);
    assert_eq!(
        packets, expected_packets,
        "received packet sequence must equal the reference packet sequence"
    );
    assert!(packets.len() > 1, "setup + at least one audio packet");
}

/// Compute the lowercase hex SHA-256 of a byte string.
fn sha256_hex(bytes: &[u8]) -> String {
    wem_profiles::resources::hex(Sha256::digest(bytes))
}

// ---------------------------------------------------------------------------
// Encode — error paths (contract: terminal single-frame, never a panic)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn init_with_unknown_setup_digest_is_profile_not_found() {
    let mut client = start_server().await;
    let replies = collect(&mut client, vec![init_request(&"0".repeat(64), "")]).await;
    assert_eq!(replies.len(), 1, "exactly one terminal error reply");
    expect_error(
        &replies,
        EncoderErrorCode::ProfileNotFound as i32,
        "unknown setup digest",
    );
}

#[tokio::test]
async fn chunk_before_init_is_state_error() {
    let mut client = start_server().await;
    let layout = PcmSampleLayout::Signed16Interleaved as i32;
    let replies = collect(
        &mut client,
        vec![chunk_request(6, 44100, layout, vec![0u8; 12])],
    )
    .await;
    assert_eq!(replies.len(), 1, "exactly one terminal error reply");
    expect_error(
        &replies,
        EncoderErrorCode::StateError as i32,
        "chunk before Init",
    );
}

#[tokio::test]
async fn duplicate_init_is_state_error() {
    let mut client = start_server().await;
    let replies = collect(
        &mut client,
        vec![
            init_request(SETUP_SHA256, PROFILE_NAME),
            init_request(SETUP_SHA256, PROFILE_NAME),
        ],
    )
    .await;
    assert_eq!(replies.len(), 1, "exactly one terminal error reply");
    expect_error(
        &replies,
        EncoderErrorCode::StateError as i32,
        "duplicate Init",
    );
}

#[tokio::test]
async fn template_ref_is_state_error() {
    let mut client = start_server().await;
    let requests = vec![EncodeRequest {
        payload: Some(encode_request::Payload::Init(Init {
            r#ref: Some(init::Ref::Template(TemplateRef {})),
        })),
    }];
    let replies = collect(&mut client, requests).await;
    assert_eq!(replies.len(), 1, "exactly one terminal error reply");
    expect_error(
        &replies,
        EncoderErrorCode::StateError as i32,
        "template ref",
    );
}

#[tokio::test]
async fn finish_without_init_is_state_error() {
    let mut client = start_server().await;
    let replies = collect(&mut client, vec![finish_request()]).await;
    assert_eq!(replies.len(), 1, "exactly one terminal error reply");
    expect_error(
        &replies,
        EncoderErrorCode::StateError as i32,
        "Finish without Init",
    );
}

#[tokio::test]
async fn stream_end_before_finish_is_state_error() {
    let mut client = start_server().await;
    let replies = collect(&mut client, vec![init_request(SETUP_SHA256, PROFILE_NAME)]).await;
    assert_eq!(replies.len(), 1, "exactly one terminal error reply");
    expect_error(
        &replies,
        EncoderErrorCode::StateError as i32,
        "stream end before Finish",
    );
}

#[tokio::test]
async fn unsupported_layout_is_format_error() {
    let mut client = start_server().await;
    let layout = PcmSampleLayout::Unspecified as i32;
    let replies = collect(
        &mut client,
        vec![
            init_request(SETUP_SHA256, PROFILE_NAME),
            chunk_request(6, 44100, layout, vec![0u8; 12]),
        ],
    )
    .await;
    assert_eq!(replies.len(), 1, "exactly one terminal error reply");
    expect_error(
        &replies,
        EncoderErrorCode::FormatUnsupported as i32,
        "unsupported layout",
    );
}

#[tokio::test]
async fn geometry_mismatch_is_rejected() {
    let mut client = start_server().await;
    let layout = PcmSampleLayout::Signed16Interleaved as i32;
    let replies = collect(
        &mut client,
        vec![
            init_request(SETUP_SHA256, PROFILE_NAME),
            chunk_request(2, 48000, layout, vec![0u8; 4]),
        ],
    )
    .await;
    assert_eq!(replies.len(), 1, "exactly one terminal error reply");
    expect_error(
        &replies,
        EncoderErrorCode::GeometryMismatch as i32,
        "geometry mismatch",
    );
}
