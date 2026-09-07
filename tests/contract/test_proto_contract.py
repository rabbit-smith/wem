"""Structural contract checks for the wwise.v1 gRPC proto files.

The proto sources are compiled in-process with grpc_tools.protoc into a
temporary directory, and the resulting descriptors are asserted against
the v1 contract: package naming, file layout, service/RPC shape, oneof
payloads, error codes, and field-number hygiene.  No generated files are
written into the repository, and no network access is used.
"""

from __future__ import annotations

import importlib
import os
import sys
import tempfile
import unittest
from pathlib import Path

try:
    import grpc_tools.protoc
    from google.protobuf import descriptor_pb2
except ImportError:
    grpc_tools = None
    descriptor_pb2 = None

ROOT = Path(__file__).resolve().parents[2]
PROTO_ROOT = ROOT / "proto"
PACKAGE = "wwise.v1"
PROTOS = (
    "wwise/v1/common.proto",
    "wwise/v1/profile.proto",
    "wwise/v1/encode.proto",
)


def _proto_text(name: str) -> str:
    return (PROTO_ROOT / name).read_text(encoding="utf-8")


@unittest.skipIf(
    grpc_tools is None,
    "grpcio-tools is not installed; install it with: pip install grpcio-tools",
)
class ProtoContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="wwise-v1-contract-")
        previous_cwd = os.getcwd()
        try:
            os.chdir(PROTO_ROOT)
            try:
                grpc_tools.protoc.main(
                    [
                        "-I.",
                        f"--python_out={cls._tmp.name}",
                        f"--grpc_python_out={cls._tmp.name}",
                        *PROTOS,
                    ]
                )
            except SystemExit as exc:
                raise AssertionError(
                    f"protoc failed to compile {PROTOS} (exit {exc.code})"
                ) from exc
        finally:
            os.chdir(previous_cwd)
        sys.path.insert(0, cls._tmp.name)
        try:
            cls.common = importlib.import_module("wwise.v1.common_pb2")
            cls.profile = importlib.import_module("wwise.v1.profile_pb2")
            cls.encode = importlib.import_module("wwise.v1.encode_pb2")
            cls.encode_grpc = importlib.import_module("wwise.v1.encode_pb2_grpc")
        finally:
            # Keep the shared import namespace clean between test runs.
            sys.path.pop(0)

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def _msg(self, module, name: str):
        return module.DESCRIPTOR.message_types_by_name[name]

    def _file_proto(self, module) -> descriptor_pb2.FileDescriptorProto:
        return descriptor_pb2.FileDescriptorProto.FromString(
            module.DESCRIPTOR.serialized_pb
        )

    def _all_messages(self, module):
        stack = list(module.DESCRIPTOR.message_types_by_name.values())
        while stack:
            message = stack.pop()
            yield message
            stack.extend(message.nested_types)

    def test_files_exist_with_expected_paths(self):
        for name in PROTOS:
            self.assertTrue((PROTO_ROOT / name).is_file(), f"missing {name}")

    def test_package_and_file_conventions(self):
        for module, name in (
            (self.common, "wwise/v1/common.proto"),
            (self.profile, "wwise/v1/profile.proto"),
            (self.encode, "wwise/v1/encode.proto"),
        ):
            self.assertEqual(module.DESCRIPTOR.package, PACKAGE, name)
            self.assertEqual(module.DESCRIPTOR.name, name)

    def test_proto3_syntax_declared(self):
        for name in PROTOS:
            text = _proto_text(name)
            self.assertIn('syntax = "proto3";', text, name)
            self.assertIn(f"package {PACKAGE};", text, name)

    def test_pcm_types(self):
        fmt = self._msg(self.common, "PcmFormat")
        numbers = {f.name: f.number for f in fmt.fields}
        self.assertEqual(numbers, {"channels": 1, "sample_rate": 2, "layout": 3})
        for field in (fmt.fields_by_name["channels"], fmt.fields_by_name["sample_rate"]):
            self.assertEqual(field.type, field.TYPE_UINT32, field.name)
        layout = fmt.fields_by_name["layout"]
        self.assertEqual(layout.type, layout.TYPE_ENUM)
        self.assertEqual(layout.enum_type.name, "PcmSampleLayout")

        values = {v.name: v.number for v in layout.enum_type.values}
        self.assertEqual(
            values,
            {
                "PCM_SAMPLE_LAYOUT_UNSPECIFIED": 0,
                "PCM_SAMPLE_LAYOUT_SIGNED_16_INTERLEAVED": 1,
            },
        )
        self.assertEqual(layout.enum_type.values[0].number, 0)

        frames = self._msg(self.common, "PcmFrames")
        data = frames.fields_by_name["data"]
        self.assertEqual(data.number, 1)
        self.assertEqual(data.type, data.TYPE_BYTES)

    def test_profile_types(self):
        key = self._msg(self.profile, "ProfileKey")
        self.assertEqual(
            {f.name for f in key.fields}, {"channels", "sample_rate"}
        )

        info = self._msg(self.profile, "ProfileInfo")
        expected = {
            "name": 1,
            "wwise_generation": 2,
            "setup_sha256": 3,
            "manifest_sha256": 4,
            "supported_formats": 5,
        }
        self.assertEqual({f.name: f.number for f in info.fields}, expected)
        formats = info.fields_by_name["supported_formats"]
        self.assertTrue(formats.is_repeated)
        self.assertEqual(formats.message_type.full_name, "wwise.v1.PcmFormat")
        for field in info.fields:
            if field.name != "supported_formats":
                self.assertEqual(field.type, field.TYPE_STRING, field.name)

    def test_service_and_rpc_shape(self):
        services = self.encode.DESCRIPTOR.services_by_name
        self.assertEqual(list(services), ["WemEncoder"])
        service = services["WemEncoder"]
        self.assertEqual(list(service.methods_by_name), [
            "ListProfiles",
            "Encode",
        ])

        list_profiles = service.methods_by_name["ListProfiles"]
        self.assertEqual(list_profiles.input_type.full_name, "wwise.v1.ListProfilesRequest")
        self.assertEqual(
            list_profiles.output_type.full_name, "wwise.v1.ListProfilesResponse"
        )
        self.assertFalse(list_profiles.client_streaming)
        self.assertFalse(list_profiles.server_streaming)

        encode = service.methods_by_name["Encode"]
        self.assertEqual(encode.input_type.full_name, "wwise.v1.EncodeRequest")
        self.assertEqual(encode.output_type.full_name, "wwise.v1.EncodeResponse")
        self.assertTrue(encode.client_streaming)
        self.assertTrue(encode.server_streaming)

    def test_encode_request_lifecycle_oneofs(self):
        request = self._msg(self.encode, "EncodeRequest")
        self.assertEqual(list(request.oneofs_by_name), ["payload"])
        self.assertEqual(
            [f.name for f in request.oneofs_by_name["payload"].fields],
            ["init", "chunk", "finish"],
        )
        self.assertEqual(
            [f.number for f in request.oneofs_by_name["payload"].fields],
            [1, 2, 3],
        )
        self.assertEqual(
            request.fields_by_name["init"].message_type.name, "Init"
        )
        self.assertEqual(
            request.fields_by_name["chunk"].message_type.name, "PcmChunk"
        )
        self.assertEqual(
            request.fields_by_name["finish"].message_type.name, "Finish"
        )

    def test_init_selection_oneof(self):
        init = self._msg(self.encode, "Init")
        self.assertEqual(list(init.oneofs_by_name), ["ref"])
        self.assertEqual(
            [f.name for f in init.oneofs_by_name["ref"].fields],
            ["profile", "template"],
        )
        self.assertEqual(
            init.fields_by_name["profile"].message_type.name, "ProfileRef"
        )
        self.assertEqual(
            init.fields_by_name["template"].message_type.name, "TemplateRef"
        )

        profile_ref = self._msg(self.encode, "ProfileRef")
        self.assertEqual(
            {f.name: f.number for f in profile_ref.fields},
            {"setup_sha256": 1, "name": 2},
        )

        template_ref = self._msg(self.encode, "TemplateRef")
        self.assertEqual(list(template_ref.fields), [])

        chunk = self._msg(self.encode, "PcmChunk")
        self.assertEqual(
            {f.name: f.number for f in chunk.fields},
            {"format": 1, "frames": 2},
        )
        self.assertEqual(
            chunk.fields_by_name["format"].message_type.name, "PcmFormat"
        )
        self.assertEqual(
            chunk.fields_by_name["frames"].message_type.name, "PcmFrames"
        )

        self.assertEqual(list(self._msg(self.encode, "Finish").fields), [])

    def test_encode_response_payloads(self):
        response = self._msg(self.encode, "EncodeResponse")
        self.assertEqual(list(response.oneofs_by_name), ["payload"])
        self.assertEqual(
            [f.name for f in response.oneofs_by_name["payload"].fields],
            ["packet", "complete", "error"],
        )

        packet = self._msg(self.encode, "Packet")
        self.assertEqual(
            {f.name: f.number for f in packet.fields}, {"seq": 1, "data": 2}
        )
        self.assertEqual(packet.fields_by_name["seq"].type, packet.fields_by_name["seq"].TYPE_UINT32)
        self.assertEqual(packet.fields_by_name["data"].type, packet.fields_by_name["data"].TYPE_BYTES)

        complete = self._msg(self.encode, "WemComplete")
        self.assertEqual(
            {f.name: f.number for f in complete.fields},
            {"total_len": 1, "sha256": 2, "inline_bytes": 3},
        )

        error = self._msg(self.encode, "EncoderError")
        self.assertEqual(
            {f.name: f.number for f in error.fields}, {"code": 1, "message": 2}
        )
        self.assertEqual(
            error.fields_by_name["code"].enum_type.name, "EncoderErrorCode"
        )

    def test_error_codes(self):
        enum = self.encode.DESCRIPTOR.enum_types_by_name["EncoderErrorCode"]
        self.assertEqual(
            {v.name: v.number for v in enum.values},
            {
                "ENCODER_ERROR_CODE_UNSPECIFIED": 0,
                "ENCODER_ERROR_CODE_PROFILE_NOT_FOUND": 1,
                "ENCODER_ERROR_CODE_GEOMETRY_MISMATCH": 2,
                "ENCODER_ERROR_CODE_INPUT_TOO_SHORT": 3,
                "ENCODER_ERROR_CODE_FORMAT_UNSUPPORTED": 4,
                "ENCODER_ERROR_CODE_STATE_ERROR": 5,
                "ENCODER_ERROR_CODE_INTERNAL": 6,
            },
        )
        self.assertEqual(enum.values[0].number, 0)

    def test_field_numbers_unique_and_unreserved(self):
        failures: list[str] = []
        for module in (self.common, self.profile, self.encode):
            file_proto = self._file_proto(module)
            for message_proto in file_proto.message_type:
                message = module.DESCRIPTOR.message_types_by_name.get(
                    message_proto.name
                )
                if message is None:
                    continue
                numbers = [field.number for field in message.fields]
                if len(numbers) != len(set(numbers)):
                    failures.append(
                        f"{message.full_name}: duplicate field numbers {numbers}"
                    )
                for field in message.fields:
                    for rng in message_proto.reserved_range:
                        if rng.start <= field.number < rng.end:
                            failures.append(
                                f"{message.full_name}.{field.name} uses reserved "
                                f"number {field.number}"
                            )
                    if field.name in message_proto.reserved_name:
                        failures.append(
                            f"{message.full_name}.{field.name} uses reserved name"
                        )
        self.assertEqual(failures, [])

    def test_reserved_ranges_present(self):
        # The v1 header reserves field numbers for forward evolution.
        for module, name, start in (
            (self.common, "PcmFormat", 4),
            (self.profile, "ProfileInfo", 6),
            (self.encode, "EncodeRequest", 4),
            (self.encode, "EncodeResponse", 4),
        ):
            file_proto = self._file_proto(module)
            target = next(
                (m for m in file_proto.message_type if m.name == name), None
            )
            self.assertIsNotNone(target, f"{name} missing in {module.__name__}")
            ranges = [(r.start, r.end) for r in target.reserved_range]
            self.assertTrue(
                any(start in rng for rng in ranges),
                f"{name}: expected a reserved range covering {start}",
            )

    def test_grpc_service_stub_generated(self):
        self.assertTrue(
            hasattr(self.encode_grpc, "add_WemEncoderServicer_to_server")
        )


if __name__ == "__main__":
    unittest.main()
