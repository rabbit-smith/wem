#!/usr/bin/env python3
"""End-to-end smoke test for the wwise.v1 mock gRPC server.

Starts scripts/mock_grpc_server.py on a local port, streams a synthetic
6-channel / 44100 Hz PCM buffer through the Encode RPC in several
chunks, and asserts that the WEM container reassembled from the received
Packet stream is byte-identical to the direct encode_wav output for the
same input.  Exits 0 on success, 1 otherwise.
"""

from __future__ import annotations

import hashlib
import math
import socket
import struct
import subprocess
import sys
import tempfile
import time
import wave
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
SRC_PATH = REPO_ROOT / "src"
SCRIPTS_PATH = REPO_ROOT / "scripts"
for path in (str(SRC_PATH), str(SCRIPTS_PATH)):
    if path not in sys.path:
        sys.path.insert(0, path)

try:
    import grpc
    from mock_grpc_server import generate_stubs, load_stubs
except ImportError as exc:  # grpcio or grpcio-tools missing from this interpreter
    print(
        "grpcio/grpcio-tools are required for the smoke test; "
        f"run: .venv/bin/python -m pip install grpcio grpcio-tools ({exc})",
        file=sys.stderr,
    )
    raise SystemExit(2)


CHANNELS = 6
SAMPLE_RATE = 44100
FRAMES = 8192
CHUNK_FRAME_SIZES = (3000, 3000, 2192)
READY_TIMEOUT_SECONDS = 60
STREAM_TIMEOUT_SECONDS = 180


def _free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def _synthetic_pcm_bytes() -> bytes:
    """Deterministic interleaved signed-16 PCM for the smoke geometry."""
    sample_rate = float(SAMPLE_RATE)
    out = bytearray()
    for frame in range(FRAMES):
        for channel in range(CHANNELS):
            value = 8000.0 * math.sin(
                2.0 * math.pi * 110.0 * frame / sample_rate
                + channel * math.pi / 3.0
            )
            value = max(-32767.0, min(32767.0, value))
            out += struct.pack("<h", int(round(value)))
    return bytes(out)


def _write_wav(path: Path, pcm_raw: bytes) -> None:
    with wave.open(str(path), "wb") as target:
        target.setnchannels(CHANNELS)
        target.setsampwidth(2)
        target.setframerate(SAMPLE_RATE)
        target.writeframes(pcm_raw)


def _start_server(port: int) -> subprocess.Popen:
    process = subprocess.Popen(
        [
            sys.executable,
            str(SCRIPTS_PATH / "mock_grpc_server.py"),
            "--host", "127.0.0.1",
            "--port", str(port),
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        cwd=REPO_ROOT,
    )
    deadline = time.monotonic() + READY_TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        if process.poll() is not None:
            output = process.communicate()[0]
            raise RuntimeError(f"server exited early:\n{output}")
        line = process.stdout.readline()
        if line.startswith(f"READY {port}"):
            return process
        if not line:
            continue
    raise RuntimeError("server did not become ready in time")


def _expect(condition: bool, detail: str) -> None:
    if not condition:
        raise AssertionError(detail)


def main() -> int:
    pcm_raw = _synthetic_pcm_bytes()
    _expect(len(pcm_raw) == FRAMES * CHANNELS * 2, "synthetic PCM size mismatch")

    with tempfile.TemporaryDirectory(prefix="wwise-v1-smoke-") as tmp:
        tmp_root = Path(tmp)
        wav_path = tmp_root / "input.wav"
        _write_wav(wav_path, pcm_raw)

        from wwise_wem import encode_wav

        reference = encode_wav(wav_path)
        reference_bytes = reference.data
        reference_sha = hashlib.sha256(reference_bytes).hexdigest()

        profile = None
        from wwise_wem.profiles.registry import PROFILE_REGISTRY

        for candidate in PROFILE_REGISTRY.list():
            if (candidate.channels, candidate.sample_rate) == (CHANNELS, SAMPLE_RATE):
                profile = candidate
        _expect(profile is not None, "no installed profile for the smoke geometry")

        port = _free_port()
        server = _start_server(port)
        try:
            with tempfile.TemporaryDirectory(prefix="wwise-v1-smoke-stubs-") as stubs:
                generate_stubs(Path(stubs))
                common, profile_mod, encode, grpc_mod = load_stubs(Path(stubs))

                channel = grpc.insecure_channel(f"127.0.0.1:{port}")
                grpc.channel_ready_future(channel).result(
                    timeout=STREAM_TIMEOUT_SECONDS
                )
                stub = grpc_mod.WemEncoderStub(channel)

                chunk_slices = []
                cursor = 0
                for size in CHUNK_FRAME_SIZES:
                    length = size * CHANNELS * 2
                    chunk_slices.append(pcm_raw[cursor : cursor + length])
                    cursor += length
                _expect(cursor == len(pcm_raw), "chunk split mismatch")

                def request_stream():
                    yield encode.EncodeRequest(
                        init=encode.Init(
                            profile=encode.ProfileRef(
                                setup_sha256=profile.setup_sha256,
                                name=profile.name,
                            )
                        )
                    )
                    for payload in chunk_slices:
                        yield encode.EncodeRequest(
                            chunk=encode.PcmChunk(
                                format=common.PcmFormat(
                                    channels=CHANNELS,
                                    sample_rate=SAMPLE_RATE,
                                    layout=common.PCM_SAMPLE_LAYOUT_SIGNED_16_INTERLEAVED,
                                ),
                                frames=common.PcmFrames(data=payload),
                            )
                        )
                    yield encode.EncodeRequest(finish=encode.Finish())

                responses = list(stub.Encode(request_stream()))

            packets: list[bytes] = []
            complete = None
            for index, response in enumerate(responses):
                which = response.WhichOneof("payload")
                if which == "packet":
                    _expect(
                        response.packet.seq == len(packets),
                        f"packet seq out of order at reply {index}",
                    )
                    packets.append(bytes(response.packet.data))
                elif which == "complete":
                    _expect(complete is None, "duplicate WemComplete")
                    complete = response.complete
                elif which == "error":
                    raise AssertionError(
                        f"server error in stream: "
                        f"{encode.EncoderErrorCode.Name(response.error.code)}: "
                        f"{response.error.message}"
                    )
                else:
                    raise AssertionError(f"empty EncodeResponse at reply {index}")

            _expect(complete is not None, "no WemComplete in stream")

            # Reassemble the WEM container from the packet stream the way
            # the Python encoder does at its container step, then compare
            # against the direct encode_wav output byte for byte.
            from wwise_wem.container.wem import build_vorbis_wem

            fmt = dict(
                profile.container_metadata.to_fmt_dict(
                    frame_count=profile.container_metadata.dwTotalPCMFrames
                )
            )
            fmt["dwTotalPCMFrames"] = FRAMES
            rebuilt = build_vorbis_wem(
                fmt,
                packets,
                seek_table=profile.seek_table,
                endian=profile.endian,
                extra_chunks=list(profile.extra_chunks),
                recompute_sizes=True,
            )
            _expect(
                len(packets) == reference.stats.audio_packets + 1,
                f"packet count {len(packets)} != setup + {reference.stats.audio_packets}",
            )
            _expect(
                rebuilt == reference_bytes,
                "reassembled WEM differs from encode_wav output",
            )
            rebuilt_sha = hashlib.sha256(rebuilt).hexdigest()
            _expect(
                rebuilt_sha == reference_sha,
                f"reassembled sha256 {rebuilt_sha} != encode_wav {reference_sha}",
            )
            _expect(
                complete.total_len == len(reference_bytes),
                f"total_len {complete.total_len} != {len(reference_bytes)}",
            )
            _expect(
                complete.sha256 == reference_sha,
                f"sha256 {complete.sha256} != {reference_sha}",
            )
            if complete.inline_bytes:
                _expect(
                    bytes(complete.inline_bytes) == reference_bytes,
                    "inline_bytes differ from encode_wav output",
                )

            # Negative path: an unknown setup digest is PROFILE_NOT_FOUND.
            bogus = encode.EncodeRequest(
                init=encode.Init(
                    profile=encode.ProfileRef(setup_sha256="0" * 64)
                )
            )
            bogus_replies = list(stub.Encode(iter([bogus])))
            _expect(len(bogus_replies) == 1, "expected exactly one error reply")
            bogus_reply = bogus_replies[0]
            _expect(
                bogus_reply.WhichOneof("payload") == "error",
                "expected an error reply for unknown setup digest",
            )
            _expect(
                bogus_reply.error.code
                == encode.ENCODER_ERROR_CODE_PROFILE_NOT_FOUND,
                f"unexpected error code {bogus_reply.error.code}",
            )

            print(f"SMOKE PASS port={port} wem_bytes={len(reference_bytes)}")
            print(f"SMOKE SHA256 {reference_sha}")
            print(
                f"SMOKE PACKETS setup+audio={len(packets)} "
                f"(audio_packets={reference.stats.audio_packets})"
            )
            return 0
        finally:
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()


if __name__ == "__main__":
    raise SystemExit(main())
