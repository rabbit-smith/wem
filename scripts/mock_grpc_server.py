#!/usr/bin/env python3
"""Local mock gRPC server for the wwise.v1 WEM encode contract.

Pins the streaming semantics of proto/wwise/v1/encode.proto on top of
the current Python encoder: a stream of Init -> chunk* -> Finish maps to
a reply of Packet* -> WemComplete, where Packet sequence 0 is the setup
packet.  gRPC stubs are generated into a temporary directory at start-up
and are never written into the repository.

Usage:
    python scripts/mock_grpc_server.py [--port 50051] [--host localhost]
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import struct
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
SRC_PATH = REPO_ROOT / "src"
if str(SRC_PATH) not in sys.path:
    sys.path.insert(0, str(SRC_PATH))

import grpc  # noqa: E402
import grpc_tools.protoc  # noqa: E402

MIN_FRAMES = 4096
PROTO_FILES = (
    "wwise/v1/common.proto",
    "wwise/v1/profile.proto",
    "wwise/v1/encode.proto",
)


def generate_stubs(dest: Path) -> None:
    """Compile the contract protos into ``dest`` (protobuf + gRPC code)."""
    previous_cwd = os.getcwd()
    try:
        os.chdir(REPO_ROOT / "proto")
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
            raise SystemExit(f"protoc failed with exit code {exc.code}") from exc
    finally:
        os.chdir(previous_cwd)


def load_stubs(dest: Path):
    """Import the generated wwise.v1 modules from ``dest``."""
    import importlib

    previous = dest.as_posix() in sys.path
    if not previous:
        sys.path.insert(0, str(dest))
    try:
        common = importlib.import_module("wwise.v1.common_pb2")
        profile = importlib.import_module("wwise.v1.profile_pb2")
        encode = importlib.import_module("wwise.v1.encode_pb2")
        grpc_module = importlib.import_module("wwise.v1.encode_pb2_grpc")
    except Exception:
        if not previous:
            sys.path.remove(str(dest))
        raise
    return common, profile, encode, grpc_module


def resolve_profile_by_setup_sha256(setup_sha256: str):
    """Find the installed profile whose setup digest matches, or None."""
    from wwise_wem.profiles.registry import PROFILE_REGISTRY

    wanted = setup_sha256.strip().lower()
    for candidate in PROFILE_REGISTRY.list():
        if candidate.setup_sha256 == wanted:
            return candidate
    return None


def profile_info_payload(common, profile, manifest_sha256: str):
    """Map one installed profile onto the contract ProfileInfo shape."""
    return {
        "name": profile.name,
        "wwise_generation": profile.key.generation or "",
        "setup_sha256": profile.setup_sha256,
        "manifest_sha256": manifest_sha256,
        "supported_formats": [
            common.PcmFormat(
                channels=profile.channels,
                sample_rate=profile.sample_rate,
                layout=common.PCM_SAMPLE_LAYOUT_SIGNED_16_INTERLEAVED,
            )
        ],
    }


def _manifest_sha256(profile_name: str) -> str:
    """Read the manifest digest recorded in the packaged profile index."""
    import wwise_wem

    index_path = Path(wwise_wem.__file__).parent / "data" / "profiles" / "index.json"
    index = json.loads(index_path.read_text(encoding="utf-8"))
    return index["profiles"][profile_name]["sha256"]


def encode_profile_stream(profile, pcm_raw: bytes):
    """Run one complete encode and yield the contract reply sequence.

    Returns the encoder result plus the packet and completion replies so
    the caller can stream them in order.
    """
    from wwise_wem.application.encoder import Encoder
    from wwise_wem.container.wem import load_wem_parts_bytes
    from wwise_wem.model import PcmBuffer

    channels = profile.channels
    frame_count = len(pcm_raw) // (2 * channels)
    values = struct.unpack(f"<{frame_count * channels}h", pcm_raw)
    pcm = tuple(
        tuple(
            values[frame * channels + channel] / 32768.0
            for frame in range(frame_count)
        )
        for channel in range(channels)
    )
    result = Encoder(profile).encode_pcm(
        PcmBuffer(sample_rate=profile.sample_rate, channels=pcm)
    )
    parts = load_wem_parts_bytes(result.data, file="<stream>", path="<stream>")
    return result.data, parts["packets"]


class WemEncoderServicer:
    """Reference implementation of the wwise.v1 WemEncoder service."""

    # Populated by the test harness and main() before serving.
    common = None
    profile_mod = None
    encode = None
    grpc_mod = None

    # -- unary: ListProfiles -------------------------------------------------

    def ListProfiles(self, request, context):  # noqa: N802 (contract name)
        from wwise_wem.profiles.registry import PROFILE_REGISTRY

        profiles = PROFILE_REGISTRY.list()
        key = request.key
        if int(key.channels) > 0 and int(key.sample_rate) > 0:
            profiles = [
                p
                for p in profiles
                if (p.channels, p.sample_rate) == (int(key.channels), int(key.sample_rate))
            ]
        payloads = [
            self.profile_mod.ProfileInfo(
                **profile_info_payload(self.common, p, _manifest_sha256(p.name))
            )
            for p in profiles
        ]
        return self.encode.ListProfilesResponse(profiles=payloads)

    # -- bidirectional: Encode -----------------------------------------------

    def Encode(self, request_iterator, context):  # noqa: N802 (contract name)
        common = self.common
        encode = self.encode

        def _error(code, message):
            return encode.EncodeResponse(
                error=encode.EncoderError(code=code, message=message)
            )

        def _profile_not_found(message):
            return _error(
                encode.ENCODER_ERROR_CODE_PROFILE_NOT_FOUND, message
            )

        def _state_error(message):
            return _error(encode.ENCODER_ERROR_CODE_STATE_ERROR, message)

        profile = None
        pcm_bytes = bytearray()
        finished = False

        for request in request_iterator:
            which = request.WhichOneof("payload")
            if which == "init":
                if profile is not None:
                    yield _state_error("Init must be the first request")
                    return
                ref_which = request.init.WhichOneof("ref")
                if ref_which == "template":
                    yield _state_error("template selection is not supported in v1")
                    return
                if ref_which != "profile":
                    yield _state_error("Init requires a profile or template ref")
                    return
                ref = request.init.profile
                candidate = resolve_profile_by_setup_sha256(ref.setup_sha256)
                if candidate is None:
                    yield _profile_not_found(
                        f"no installed profile matches setup_sha256 {ref.setup_sha256!r}"
                    )
                    return
                if ref.name and ref.name != candidate.name:
                    yield _state_error(
                        f"profile name mismatch: setup_sha256 resolves to "
                        f"{candidate.name!r}, not {ref.name!r}"
                    )
                    return
                profile = candidate
            elif which == "chunk":
                if profile is None:
                    yield _state_error("Init is required before chunks")
                    return
                if finished:
                    yield _state_error("chunks are not allowed after Finish")
                    return
                chunk = request.chunk
                fmt = chunk.format
                if int(fmt.layout) != int(
                    common.PCM_SAMPLE_LAYOUT_SIGNED_16_INTERLEAVED
                ):
                    yield _error(
                        encode.ENCODER_ERROR_CODE_FORMAT_UNSUPPORTED,
                        "v1 only supports SIGNED_16_INTERLEAVED PCM",
                    )
                    return
                if (int(fmt.channels), int(fmt.sample_rate)) != (
                    profile.channels,
                    profile.sample_rate,
                ):
                    yield _error(
                        encode.ENCODER_ERROR_CODE_GEOMETRY_MISMATCH,
                        f"PCM geometry {fmt.channels}ch/{fmt.sample_rate}Hz does "
                        f"not match profile {profile.name} "
                        f"({profile.channels}ch/{profile.sample_rate}Hz)",
                    )
                    return
                data = bytes(chunk.frames.data)
                if len(data) % (2 * profile.channels):
                    yield _error(
                        encode.ENCODER_ERROR_CODE_GEOMETRY_MISMATCH,
                        "chunk carries a trailing partial PCM frame",
                    )
                    return
                pcm_bytes.extend(data)
            elif which == "finish":
                if profile is None or finished:
                    yield _state_error("Finish must come exactly once, after Init")
                    return
                finished = True
                try:
                    frame_count = len(pcm_bytes) // (2 * profile.channels)
                    if frame_count < MIN_FRAMES:
                        yield _error(
                            encode.ENCODER_ERROR_CODE_INPUT_TOO_SHORT,
                            f"PCM input must contain at least {MIN_FRAMES} frames, "
                            f"got {frame_count}",
                        )
                        return
                    wem_bytes, packets = encode_profile_stream(profile, bytes(pcm_bytes))
                    for seq, data in enumerate(packets):
                        yield encode.EncodeResponse(
                            packet=encode.Packet(seq=seq, data=data)
                        )
                    yield encode.EncodeResponse(
                        complete=encode.WemComplete(
                            total_len=len(wem_bytes),
                            sha256=hashlib.sha256(wem_bytes).hexdigest(),
                            inline_bytes=wem_bytes,
                        )
                    )
                except Exception as exc:  # contract: terminal INTERNAL error
                    yield _error(
                        encode.ENCODER_ERROR_CODE_INTERNAL,
                        f"encoder fault: {exc}",
                    )
                return
            else:
                yield _state_error("empty EncodeRequest payload")
                return

        # The client closed the request stream without sending Finish.
        if not finished:
            yield _state_error("request stream ended before Finish")


def serve(host: str, port: int) -> None:
    with tempfile.TemporaryDirectory(prefix="wwise-v1-mock-stubs-") as tmp:
        dest = Path(tmp)
        generate_stubs(dest)
        common, profile_mod, encode, grpc_mod = load_stubs(dest)

        servicer = WemEncoderServicer()
        servicer.common = common
        servicer.profile_mod = profile_mod
        servicer.encode = encode
        servicer.grpc_mod = grpc_mod

        server = grpc.server(concurrent.futures.ThreadPoolExecutor(max_workers=4))
        grpc_mod.add_WemEncoderServicer_to_server(servicer, server)
        bound = server.add_insecure_port(f"{host}:{port}")
        if bound == 0:
            raise SystemExit(f"could not bind {host}:{port}")
        server.start()
        print(f"READY {bound}", flush=True)
        server.wait_for_termination()


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="localhost")
    parser.add_argument("--port", type=int, default=50051)
    args = parser.parse_args(argv)
    serve(args.host, args.port)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
