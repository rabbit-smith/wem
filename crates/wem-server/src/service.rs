//! The wwise.v1 `WemEncoder` gRPC service.
//!
//! Design (thin shell over the kernel):
//!
//! * `StreamSession` is the single source of truth for encode semantics;
//!   this file only translates between the proto lifecycle and it.
//! * `Encode` runs the whole session on one blocking thread (the kernel is
//!   synchronous CPU work and must not block a tokio worker). Requests are
//!   piped over a bounded channel (backpressure); replies over an
//!   unbounded one (each reply is bounded by one packet, the output
//!   itself).
//! * No buffering/reordering: each packet returned by `push_pcm_chunk` is
//!   sent immediately; the end-of-stream tail is flushed by the kernel at
//!   `Finish`, and the service simply forwards what it was given.

use sha2::{Digest, Sha256};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use wem_container::load_wem_parts_bytes;
use wem_core::error::EncoderError;
use wem_core::stream::{ProfileRef, StreamSession};
use wem_profiles::data::DataDir;
use wem_profiles::registry::installed_registry;

use crate::wwise::{
    encode_request, encode_response, init, wem_encoder_server::WemEncoder as WemEncoderTrait,
    EncodeRequest, EncodeResponse, EncoderError as EncoderErrorReply, EncoderErrorCode,
    ListProfilesRequest, ListProfilesResponse, Packet, PcmFormat, PcmSampleLayout, ProfileInfo,
    WemComplete,
};

/// Containers at most this size are inlined into `WemComplete`; larger
/// ones leave `inline_bytes` empty (the client reconstructs from the
/// packet stream).
const INLINE_BYTES_LIMIT: usize = 2 * 1024 * 1024;

/// Bounded request queue: a slow worker applies backpressure to the gRPC
/// stream instead of unbounded input buffering.
const REQUEST_QUEUE_CAPACITY: usize = 16;

/// The service holds no state of its own: the installed profile tree is
/// resolved per call from the environment (`WEM_DATA_DIR` or repository
/// layout), matching the kernel's own resolution.
#[derive(Debug, Default)]
pub struct WemEncoderService;

impl WemEncoderService {
    pub fn new() -> Self {
        Self
    }
}

/// One resolved profile's geometry (for per-chunk format checks).
#[derive(Clone)]
struct ProfileGeo {
    name: String,
    channels: i64,
    sample_rate: i64,
}

type Reply = Result<EncodeResponse, Status>;
type ReplyTx = tokio::sync::mpsc::UnboundedSender<Reply>;

#[tonic::async_trait]
impl WemEncoderTrait for WemEncoderService {
    type EncodeStream = UnboundedReceiverStream<Reply>;

    /// List the installed encoder profiles, optionally filtered by geometry.
    async fn list_profiles(
        &self,
        request: Request<ListProfilesRequest>,
    ) -> Result<Response<ListProfilesResponse>, Status> {
        let key = request.into_inner().key;
        // Small filesystem work, but keep it off the async worker anyway.
        let (channels, sample_rate) = match key {
            Some(key) => (key.channels as i64, key.sample_rate as i64),
            None => (0, 0),
        };
        let response =
            tokio::task::spawn_blocking(move || list_profiles_blocking(channels, sample_rate))
                .await
                .map_err(|error| Status::internal(format!("worker task failed: {error}")))?;
        response.map(Response::new)
    }

    /// Run one streaming encode (Init -> chunk* -> Finish).
    async fn encode(
        &self,
        request: Request<Streaming<EncodeRequest>>,
    ) -> Result<Response<UnboundedReceiverStream<Reply>>, Status> {
        let (req_tx, req_rx) = tokio::sync::mpsc::channel(REQUEST_QUEUE_CAPACITY);
        let (resp_tx, resp_rx) = tokio::sync::mpsc::unbounded_channel();

        // Feeder: pump client requests into the (bounded) worker queue.
        tokio::spawn(async move {
            let mut stream = request.into_inner();
            while let Ok(Some(request)) = stream.message().await {
                if req_tx.send(request).await.is_err() {
                    break; // worker gone; nothing left to feed
                }
            }
            // Dropping the sender marks end-of-requests for the worker.
        });

        // Worker: the whole session on one blocking thread.
        tokio::task::spawn_blocking(move || run_encode_call(req_rx, &resp_tx));

        Ok(Response::new(UnboundedReceiverStream::new(resp_rx)))
    }
}

/// ListProfiles body (runs on a blocking thread).
fn list_profiles_blocking(channels: i64, sample_rate: i64) -> Result<ListProfilesResponse, Status> {
    let data = DataDir::from_env()
        .map_err(|error| Status::internal(format!("profile data directory: {error}")))?;
    let registry = installed_registry(&data)
        .map_err(|error| Status::internal(format!("profile registry: {error}")))?;
    let index_bytes = std::fs::read(data.index_path())
        .map_err(|error| Status::internal(format!("profile index: {error}")))?;

    let profiles: Vec<ProfileInfo> = registry
        .list()
        .into_iter()
        .filter(|profile| {
            channels <= 0
                || sample_rate <= 0
                || (profile.channels(), profile.sample_rate()) == (channels, sample_rate)
        })
        .map(|profile| ProfileInfo {
            name: profile.name().to_string(),
            wwise_generation: profile
                .key()
                .generation()
                .map(str::to_string)
                .unwrap_or_default(),
            setup_sha256: profile.setup_sha256().to_string(),
            manifest_sha256: manifest_sha256_for(&index_bytes, profile.name()).unwrap_or_default(),
            supported_formats: vec![PcmFormat {
                channels: profile.channels() as u32,
                sample_rate: profile.sample_rate() as u32,
                layout: PcmSampleLayout::Signed16Interleaved as i32,
            }],
        })
        .collect();

    Ok(ListProfilesResponse { profiles })
}

/// The manifest digest recorded for one profile in the package index
/// (the version-handshake value clients pin; mirrors the reference
/// semantics of reading `index.json -> profiles[name].sha256`).
fn manifest_sha256_for(index_bytes: &[u8], name: &str) -> Option<String> {
    let index: serde_json::Value = serde_json::from_slice(index_bytes).ok()?;
    index
        .get("profiles")?
        .get(name)?
        .get("sha256")?
        .as_str()?
        .to_string()
        .into()
}

/// Resolve one installed profile's geometry by setup digest (lowercase,
/// exact — the same assertion the kernel performs in `init_profile`).
fn resolve_profile_geometry(setup_sha256: &str) -> Option<ProfileGeo> {
    let data = DataDir::from_env().ok()?;
    let registry = installed_registry(&data).ok()?;
    let wanted = setup_sha256.to_lowercase();
    registry
        .list()
        .into_iter()
        .find(|profile| profile.setup_sha256() == wanted)
        .map(|profile| ProfileGeo {
            name: profile.name().to_string(),
            channels: profile.channels(),
            sample_rate: profile.sample_rate(),
        })
}

/// The whole Encode call state machine (runs on one blocking thread).
///
/// Reply contract (wwise.v1): packets in emission order (seq 0 = setup),
/// then exactly one `WemComplete` — or a single terminal `EncoderError`,
/// after which the stream closes.
fn run_encode_call(mut req_rx: tokio::sync::mpsc::Receiver<EncodeRequest>, resp_tx: &ReplyTx) {
    let mut session = StreamSession::new();
    let mut profile: Option<ProfileGeo> = None;
    let mut seq: u32 = 0;
    // 0 = awaiting Init, 1 = initialized, 2 = stream terminated
    let mut phase: u8 = 0;

    loop {
        if phase == 2 {
            break;
        }
        let request = match req_rx.blocking_recv() {
            Some(request) => request,
            None => break, // request stream ended
        };

        match &request.payload {
            Some(encode_request::Payload::Init(init)) => match &init.r#ref {
                Some(init::Ref::Profile(ref_message)) => {
                    if phase == 1 {
                        terminal_reply(
                            resp_tx,
                            lifecycle_error(
                                EncoderErrorCode::StateError,
                                "Init must be the first request",
                            ),
                        );
                        phase = 2;
                        continue;
                    }
                    let ref_ = ProfileRef {
                        setup_sha256: ref_message.setup_sha256.clone(),
                        // proto3: empty name is the absence of a soft check.
                        name: (!ref_message.name.is_empty()).then(|| ref_message.name.clone()),
                    };
                    match session.init_profile(&ref_) {
                        Err(error) => {
                            terminal_reply(resp_tx, kernel_error(&error));
                            phase = 2;
                        }
                        Ok(()) => match resolve_profile_geometry(&ref_message.setup_sha256) {
                            Some(geo) => {
                                profile = Some(geo);
                                phase = 1;
                            }
                            None => {
                                // The kernel just accepted this digest; the
                                // geometry lookup must see the same tree.
                                terminal_reply(
                                    resp_tx,
                                    lifecycle_error(
                                        EncoderErrorCode::Internal,
                                        "installed profile tree changed mid-request",
                                    ),
                                );
                                phase = 2;
                            }
                        },
                    }
                }
                Some(init::Ref::Template(_)) => {
                    terminal_reply(
                        resp_tx,
                        lifecycle_error(
                            EncoderErrorCode::StateError,
                            "template selection is not supported in v1",
                        ),
                    );
                    phase = 2;
                }
                None => {
                    terminal_reply(
                        resp_tx,
                        lifecycle_error(
                            EncoderErrorCode::StateError,
                            "Init requires a profile or template ref",
                        ),
                    );
                    phase = 2;
                }
            },
            Some(encode_request::Payload::Chunk(chunk)) => {
                if phase == 0 {
                    terminal_reply(
                        resp_tx,
                        lifecycle_error(
                            EncoderErrorCode::StateError,
                            "Init is required before chunks",
                        ),
                    );
                    phase = 2;
                    continue;
                }
                // v1 accepts only SIGNED_16_INTERLEAVED, at the profile geometry.
                let layout_ok = chunk
                    .format
                    .as_ref()
                    .map(|format| format.layout == PcmSampleLayout::Signed16Interleaved as i32)
                    .unwrap_or(false);
                if !layout_ok {
                    terminal_reply(
                        resp_tx,
                        lifecycle_error(
                            EncoderErrorCode::FormatUnsupported,
                            "v1 only supports SIGNED_16_INTERLEAVED PCM",
                        ),
                    );
                    phase = 2;
                    continue;
                }
                let geo = profile
                    .as_ref()
                    .expect("phase 1 stores the profile geometry");
                let format = chunk.format.as_ref().expect("layout check passed");
                if (format.channels as i64, format.sample_rate as i64)
                    != (geo.channels, geo.sample_rate)
                {
                    terminal_reply(
                        resp_tx,
                        lifecycle_error(
                            EncoderErrorCode::GeometryMismatch,
                            &format!(
                                "PCM geometry {}ch/{}Hz does not match profile {} ({}ch/{}Hz)",
                                format.channels,
                                format.sample_rate,
                                geo.name,
                                geo.channels,
                                geo.sample_rate
                            ),
                        ),
                    );
                    phase = 2;
                    continue;
                }
                let frames = &chunk.frames;
                let data = frames
                    .as_ref()
                    .map(|f| f.data.as_slice())
                    .unwrap_or_default();
                if !data.len().is_multiple_of(2 * geo.channels as usize) {
                    terminal_reply(
                        resp_tx,
                        lifecycle_error(
                            EncoderErrorCode::GeometryMismatch,
                            "chunk carries a trailing partial PCM frame",
                        ),
                    );
                    phase = 2;
                    continue;
                }
                match session.push_pcm_chunk(data) {
                    Err(error) => {
                        terminal_reply(resp_tx, kernel_error(&error));
                        phase = 2;
                    }
                    Ok(packets) => {
                        for packet in packets {
                            let _ = resp_tx.send(Ok(EncodeResponse {
                                payload: Some(encode_response::Payload::Packet(Packet {
                                    seq,
                                    data: packet.data,
                                })),
                            }));
                            seq += 1;
                        }
                    }
                }
            }
            Some(encode_request::Payload::Finish(_)) => {
                if phase != 1 {
                    terminal_reply(
                        resp_tx,
                        lifecycle_error(
                            EncoderErrorCode::StateError,
                            "Finish must come exactly once, after Init",
                        ),
                    );
                    phase = 2;
                    continue;
                }
                match session.finish() {
                    Err(error) => {
                        terminal_reply(resp_tx, kernel_error(&error));
                        phase = 2;
                    }
                    Ok(result) => {
                        // Flush the kernel's end-of-stream tail: the full
                        // packet sequence is recoverable from the container
                        // itself (the kernel keeps it in emission order).
                        let tail = match load_wem_parts_bytes(&result.data) {
                            Ok(parts) => {
                                let mut all: Vec<Vec<u8>> = Vec::new();
                                if let Some(setup) = parts.setup_packet {
                                    all.push(setup);
                                }
                                all.extend(parts.audio_packets);
                                all
                            }
                            Err(error) => {
                                terminal_reply(
                                    resp_tx,
                                    lifecycle_error(
                                        EncoderErrorCode::Internal,
                                        &format!("container reparse failed: {error}"),
                                    ),
                                );
                                phase = 2;
                                continue;
                            }
                        };
                        for (index, data) in tail.iter().enumerate().skip(seq as usize) {
                            let _ = resp_tx.send(Ok(EncodeResponse {
                                payload: Some(encode_response::Payload::Packet(Packet {
                                    seq: index as u32,
                                    data: data.clone(),
                                })),
                            }));
                        }
                        seq = tail.len() as u32;
                        let digest = Sha256::digest(&result.data);
                        let sha256 = wem_profiles::resources::hex(digest);
                        let inline_bytes = if result.data.len() <= INLINE_BYTES_LIMIT {
                            result.data.clone()
                        } else {
                            Vec::new()
                        };
                        let _ = resp_tx.send(Ok(EncodeResponse {
                            payload: Some(encode_response::Payload::Complete(WemComplete {
                                total_len: result.data.len() as u64,
                                sha256,
                                inline_bytes,
                            })),
                        }));
                        phase = 2;
                    }
                }
            }
            None => {
                terminal_reply(
                    resp_tx,
                    lifecycle_error(EncoderErrorCode::StateError, "empty EncodeRequest payload"),
                );
                phase = 2;
            }
        }
    }

    if phase != 2 {
        terminal_reply(
            resp_tx,
            lifecycle_error(
                EncoderErrorCode::StateError,
                "request stream ended before Finish",
            ),
        );
    }
}

/// Map one kernel error onto the wwise.v1 terminal error reply (1:1).
fn kernel_error(error: &EncoderError) -> EncoderErrorReply {
    let code = match error {
        EncoderError::ProfileNotFound { .. } => EncoderErrorCode::ProfileNotFound,
        EncoderError::StateError { .. } => EncoderErrorCode::StateError,
        EncoderError::GeometryMismatch { .. } => EncoderErrorCode::GeometryMismatch,
        EncoderError::InputTooShort { .. } => EncoderErrorCode::InputTooShort,
        EncoderError::FormatUnsupported { .. } => EncoderErrorCode::FormatUnsupported,
        EncoderError::Internal(..) => EncoderErrorCode::Internal,
    };
    EncoderErrorReply {
        code: code as i32,
        message: error.to_string(),
    }
}

/// Build a terminal error reply from a code + message (lifecycle checks).
fn lifecycle_error(code: EncoderErrorCode, message: &str) -> EncoderErrorReply {
    EncoderErrorReply {
        code: code as i32,
        message: message.to_string(),
    }
}

/// Send one terminal reply. Fire-and-forget: a dead receiver only means
/// the client hung up; the stream is already over.
fn terminal_reply(resp_tx: &ReplyTx, reply: EncoderErrorReply) {
    let _ = resp_tx.send(Ok(EncodeResponse {
        payload: Some(encode_response::Payload::Error(reply)),
    }));
}
