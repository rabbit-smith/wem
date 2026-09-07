#!/usr/bin/env python3
"""Cross-implementation smoke: Python gRPC client vs the real wem-server.

Supersedes the retired mock smoke's CI role now that the real Rust
wem-server owns the wwise.v1 contract: v1 semantics are pinned by the
wem-server integration tests plus this interop smoke.

Starts the real wem-server binary on an ephemeral loopback port (port 0;
the bound port is read from the server's READY line, so there is no
fixed-delay race), discovers the installed profile through the
ListProfiles handshake, streams tests/fixtures/input.wav's PCM through
the Encode RPC in three uneven frame-aligned chunks, and verifies:

  * the WemComplete's inline container bytes are byte-identical to
    tests/fixtures/reference.wem (the fixture is under the server's
    inline_bytes size limit, so the container travels in-band and is
    compared directly — no second container implementation needed);
  * the WemComplete summary (total_len / sha256) matches and the packet
    stream carries the golden packet count (setup + 205 audio) in
    strict sequence order;
  * an Init with a wrong setup_sha256 is a terminal PROFILE_NOT_FOUND.

Exit codes: 0 pass (or skip with a hint when no server binary is found via
default probing), 1 verification failure, 2 usage/environment error
(explicit binary missing, grpcio/grpcio-tools absent, fixtures missing).
Generated stubs and all intermediate artifacts live only in temp
directories; the repository is never written.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import subprocess
import sys
import tempfile
import time
import wave
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
SRC_PATH = REPO_ROOT / "src"
if str(SRC_PATH) not in sys.path:
    sys.path.insert(0, str(SRC_PATH))

FIXTURE_WAV = REPO_ROOT / "tests" / "fixtures" / "input.wav"
REFERENCE_WEM = REPO_ROOT / "tests" / "fixtures" / "reference.wem"
PROTO_ROOT = REPO_ROOT / "proto"
PROTO_FILES = (
    "wwise/v1/common.proto",
    "wwise/v1/profile.proto",
    "wwise/v1/encode.proto",
)
DEFAULT_SERVER_BIN = REPO_ROOT / "crates" / "target" / "release" / "wem-server"
# Two fixed head chunks in PCM frames; the third absorbs the remainder, so
# the split is always uneven, frame-aligned, and tiles the stream exactly.
CHUNK_HEAD_FRAMES = (3000, 5000)
READY_TIMEOUT_SECONDS = 60
STREAM_TIMEOUT_SECONDS = 180

try:
    import grpc
    import grpc_tools.protoc
except ImportError as exc:
    print(
        "grpcio/grpcio-tools are required for the interop smoke; "
        f"run: .venv/bin/python -m pip install grpcio grpcio-tools ({exc})",
        file=sys.stderr,
    )
    raise SystemExit(2)


def resolve_server_bin(explicit: str | None) -> tuple[Path | None, str | None]:
    """Resolve the wem-server binary.

    Returns ``(path, None)`` on success. An explicitly requested binary
    (``--server-bin`` or ``WEM_SERVER_BIN``) that is missing is a hard
    error ``(None, message)``; a default probe miss is a soft skip signal
    ``(None, "")`` so callers can exit 0 with a build hint.
    """
    for value in (explicit, os.environ.get("WEM_SERVER_BIN")):
        if value:
            path = Path(value).expanduser()
            if path.is_file():
                return path, None
            return None, f"server binary not found: {path}"
    if DEFAULT_SERVER_BIN.is_file():
        return DEFAULT_SERVER_BIN, None
    return None, ""


def generate_stubs(dest: Path) -> None:
    """Compile the wwise.v1 contract protos into ``dest`` (temp only)."""
    previous_cwd = os.getcwd()
    try:
        os.chdir(PROTO_ROOT)
        try:
            grpc_tools.protoc.main(
                [
                    "-I.",
                    f"--python_out={dest}",
                    f"--grpc_python_out={dest}",
                    *PROTO_FILES,
                ]
            )
        except SystemExit as exc:
            raise SystemExit(
                f"interop: protoc failed with exit code {exc.code}"
            ) from exc
    finally:
        os.chdir(previous_cwd)


def load_stubs(dest: Path):
    """Import the generated wwise.v1 modules from ``dest``."""
    import importlib

    sys.path.insert(0, str(dest))
    try:
        common = importlib.import_module("wwise.v1.common_pb2")
        encode = importlib.import_module("wwise.v1.encode_pb2")
        grpc_mod = importlib.import_module("wwise.v1.encode_pb2_grpc")
    except Exception:
        sys.path.remove(str(dest))
        raise
    return common, encode, grpc_mod


def start_server(server_bin: Path) -> tuple[subprocess.Popen, int]:
    """Start the server on an ephemeral port; wait for its READY line."""
    process = subprocess.Popen(
        [str(server_bin), "--addr", "127.0.0.1:0"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        cwd=REPO_ROOT,
    )
    deadline = time.monotonic() + READY_TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        if process.poll() is not None:
            output = process.communicate()[0]
            raise RuntimeError(
                f"server exited early (rc={process.returncode}):\n{output}"
            )
        line = process.stdout.readline()
        if line.startswith("READY "):
            host, sep, port = line.split(None, 1)[1].strip().rpartition(":")
            if not sep or not port.isdigit():
                raise RuntimeError(f"unexpected READY line: {line!r}")
            return process, int(port)
        if not line:
            continue
    raise RuntimeError(
        f"server did not become ready within {READY_TIMEOUT_SECONDS}s"
    )


def read_fixture_pcm() -> tuple[bytes, int, int, int]:
    """Return (interleaved s16le PCM, channels, sample_rate, frames)."""
    with wave.open(str(FIXTURE_WAV), "rb") as source:
        channels = source.getnchannels()
        sample_rate = source.getframerate()
        frames = source.getnframes()
        if source.getsampwidth() != 2:
            raise RuntimeError(f"unexpected sample width in {FIXTURE_WAV}")
        pcm_raw = source.readframes(frames)
    return pcm_raw, channels, sample_rate, frames


def chunk_split(frames: int) -> list[int]:
    """Three uneven, frame-aligned chunk sizes that tile the stream."""
    head = list(CHUNK_HEAD_FRAMES)
    tail = frames - sum(head)
    if tail <= 0 or tail in head:
        raise RuntimeError(
            f"cannot split {frames} frames into three uneven chunks "
            f"({CHUNK_HEAD_FRAMES} + remainder)"
        )
    return [*head, tail]


def discover_profile(stub, encode):
    """ListProfiles handshake: the single installed profile's identity."""
    reply = stub.ListProfiles(
        encode.ListProfilesRequest(), timeout=STREAM_TIMEOUT_SECONDS
    )
    if len(reply.profiles) != 1:
        raise RuntimeError(
            f"expected exactly one installed profile, got {len(reply.profiles)}"
        )
    profile = reply.profiles[0]
    if not profile.setup_sha256 or not profile.name:
        raise RuntimeError("profile identity incomplete in ListProfiles reply")
    if not profile.supported_formats:
        raise RuntimeError("no supported formats in ListProfiles reply")
    return profile


def run_encode_stream(stub, encode, common, profile, pcm_raw, channels, sample_rate):
    """Drive one Init -> 3 uneven chunks -> Finish stream; all replies."""
    frames = len(pcm_raw) // (channels * 2)
    chunk_frames = chunk_split(frames)
    assert sum(chunk_frames) == frames, "chunk split must tile the stream"

    def request_stream():
        yield encode.EncodeRequest(
            init=encode.Init(
                profile=encode.ProfileRef(
                    setup_sha256=profile.setup_sha256,
                    name=profile.name,
                )
            )
        )
        offset = 0
        for count in chunk_frames:
            length = count * channels * 2
            yield encode.EncodeRequest(
                chunk=encode.PcmChunk(
                    format=common.PcmFormat(
                        channels=channels,
                        sample_rate=sample_rate,
                        layout=common.PCM_SAMPLE_LAYOUT_SIGNED_16_INTERLEAVED,
                    ),
                    frames=common.PcmFrames(data=pcm_raw[offset : offset + length]),
                )
            )
            offset += length
        yield encode.EncodeRequest(finish=encode.Finish())

    stream = stub.Encode(request_stream(), timeout=STREAM_TIMEOUT_SECONDS)
    return list(stream)


def split_replies(replies, encode) -> tuple[list[bytes], object]:
    """Replies must be strict-seq Packets followed by one WemComplete."""
    packets: list[bytes] = []
    complete = None
    for index, response in enumerate(replies):
        which = response.WhichOneof("payload")
        if which == "packet":
            if response.packet.seq != len(packets):
                raise RuntimeError(f"packet seq out of order at reply {index}")
            packets.append(bytes(response.packet.data))
        elif which == "complete":
            if complete is not None:
                raise RuntimeError("duplicate WemComplete")
            complete = response.complete
        elif which == "error":
            name = encode.EncoderErrorCode.Name(response.error.code)
            raise RuntimeError(
                f"server error in stream: {name}: {response.error.message}"
            )
        else:
            raise RuntimeError(f"empty EncodeResponse at reply {index}")
    if complete is None:
        raise RuntimeError("no WemComplete in stream")
    return packets, complete


# Golden stream identity (tests/fixtures/reference.wem contract):
# the setup packet (seq 0) plus 205 audio packets; the golden container
# (108,771 bytes) is below the server's inline_bytes limit, so the full
# container is guaranteed to travel inline for this fixture.
GOLDEN_PACKET_COUNT = 206
GOLDEN_CONTAINER_BYTES = 108771


def verify_reference_identity(
    reference: bytes, reference_sha: str, packets: list[bytes], complete
) -> None:
    """WemComplete container bytes + stream shape vs reference.wem."""
    if complete.total_len != len(reference):
        raise RuntimeError(
            f"total_len {complete.total_len} != {len(reference)}"
        )
    if complete.sha256 != reference_sha:
        raise RuntimeError(
            f"sha256 {complete.sha256} != reference {reference_sha}"
        )
    if not complete.inline_bytes:
        raise RuntimeError(
            "inline_bytes empty for a container below the server's "
            "inline size limit; the v1 contract must carry it in-band"
        )
    if bytes(complete.inline_bytes) != reference:
        raise RuntimeError("inline_bytes differ from reference.wem")
    if len(packets) != GOLDEN_PACKET_COUNT:
        raise RuntimeError(
            f"packet count {len(packets)} != golden {GOLDEN_PACKET_COUNT} "
            "(setup + 205 audio packets)"
        )
    # The golden container size re-asserts the whole-file identity:
    # any drift in the packet stream would surface as an inline_bytes
    # mismatch above; the size check keeps the digest contract visible
    # even if the reference fixture is ever re-addressed.
    if len(reference) != GOLDEN_CONTAINER_BYTES:
        raise RuntimeError(
            f"reference container size {len(reference)} != "
            f"{GOLDEN_CONTAINER_BYTES}; golden identity drifted"
        )


def verify_profile_not_found(stub, encode) -> None:
    """Error path: an unknown setup digest is PROFILE_NOT_FOUND."""
    bogus = encode.EncodeRequest(
        init=encode.Init(profile=encode.ProfileRef(setup_sha256="0" * 64))
    )
    replies = list(stub.Encode(iter([bogus]), timeout=STREAM_TIMEOUT_SECONDS))
    if len(replies) != 1:
        raise RuntimeError(
            "expected exactly one error reply for unknown setup digest"
        )
    reply = replies[0]
    if reply.WhichOneof("payload") != "error":
        raise RuntimeError("expected an error reply for unknown setup digest")
    if reply.error.code != encode.ENCODER_ERROR_CODE_PROFILE_NOT_FOUND:
        raise RuntimeError(f"unexpected error code {reply.error.code}")


def run_interop(server_bin: Path) -> tuple[int, int, str]:
    """Full interop run; returns (port, wem_bytes, reference_sha)."""
    pcm_raw, channels, sample_rate, frames = read_fixture_pcm()
    reference = REFERENCE_WEM.read_bytes()
    reference_sha = hashlib.sha256(reference).hexdigest()

    with tempfile.TemporaryDirectory(prefix="wwise-v1-interop-stubs-") as stubs:
        stubs_path = Path(stubs)
        generate_stubs(stubs_path)
        common, encode, grpc_mod = load_stubs(stubs_path)
        server, port = start_server(server_bin)
        channel = None
        try:
            channel = grpc.insecure_channel(f"127.0.0.1:{port}")
            grpc.channel_ready_future(channel).result(
                timeout=STREAM_TIMEOUT_SECONDS
            )
            stub = grpc_mod.WemEncoderStub(channel)

            profile = discover_profile(stub, encode)
            fmt = profile.supported_formats[0]
            if (fmt.channels, fmt.sample_rate) != (channels, sample_rate):
                raise RuntimeError(
                    f"profile geometry {fmt.channels}ch/{fmt.sample_rate}Hz "
                    f"does not match fixture {channels}ch/{sample_rate}Hz"
                )
            if fmt.layout != common.PCM_SAMPLE_LAYOUT_SIGNED_16_INTERLEAVED:
                raise RuntimeError(
                    "profile layout is not SIGNED_16_INTERLEAVED"
                )

            replies = run_encode_stream(
                stub, encode, common, profile, pcm_raw, channels, sample_rate
            )
            packets, complete = split_replies(replies, encode)
            verify_reference_identity(
                reference, reference_sha, packets, complete
            )
            verify_profile_not_found(stub, encode)
            return port, len(reference), reference_sha
        finally:
            if channel is not None:
                channel.close()
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
        sys.path.remove(str(stubs_path))


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--server-bin",
        help="path to the wem-server binary (default: $WEM_SERVER_BIN, or "
        f"{DEFAULT_SERVER_BIN}; skip with a hint if none is found)",
    )
    args = parser.parse_args()

    server_bin, error = resolve_server_bin(args.server_bin)
    if error:
        print(f"interop: {error}", file=sys.stderr)
        return 2
    if server_bin is None:
        print(
            "interop SKIP: no wem-server binary found (probed "
            f"{DEFAULT_SERVER_BIN}); build it with: cargo build --release "
            "-p wem-server, or set WEM_SERVER_BIN / pass --server-bin.",
            file=sys.stderr,
        )
        return 0
    for fixture in (FIXTURE_WAV, REFERENCE_WEM):
        if not fixture.is_file():
            print(f"interop: missing fixture {fixture}", file=sys.stderr)
            return 2

    try:
        port, wem_bytes, reference_sha = run_interop(server_bin)
    except (RuntimeError, grpc.RpcError, OSError) as exc:
        print(f"interop FAIL: {exc}", file=sys.stderr)
        return 1

    print(f"INTEROP PASS port={port} wem_bytes={wem_bytes}")
    print(f"INTEROP SHA256 {reference_sha}")
    print("INTEROP ERROR PATH PROFILE_NOT_FOUND OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
