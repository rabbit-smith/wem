"""Engine parity: native kernel vs pure-Python oracle, byte-for-byte.

These tests are the executable definition of the facade cutover: the same
reference PCM must yield byte-identical WEM output whether produced by the
native kernel or by the pure-Python implementation, reachable through the
public facade (``WWISE_WEM_ENGINE`` pinning), the internal Python path, or
the raw native binding. Error paths (short input, geometry mismatch) and
the rejection conditions of the signed-16 sample domain must agree across
engines, and an explicit native pin with a missing extension must fail with
a clear ``ImportError`` instead of downgrading silently.

The whole class skips (with an install hint, mirroring the proto contract
precedent) in environments without the native extension.
"""

from __future__ import annotations

import hashlib
import os
import subprocess
import sys
import unittest
from contextlib import contextmanager
from pathlib import Path

from wwise_wem import _engine
from wwise_wem import Encoder, load_wem_profile
from wwise_wem.adapters.wav import read_pcm16
from wwise_wem.model import PcmBuffer

try:
    import _wwise_wem_native
except ImportError:
    _wwise_wem_native = None

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "src"
FIXTURES = ROOT / "tests" / "fixtures"
INPUT = FIXTURES / "input.wav"
REFERENCE = FIXTURES / "reference.wem"
PROFILE_NAME = "wwise2013-6ch-44100"
EXPECTED_SHA256 = (
    "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247"
)
EXPECTED_STATS = {
    "pcm_frames": 139398,
    "channels": 6,
    "audio_packets": 205,
    "short_packets": 77,
    "long_packets": 128,
    "bytes": 108771,
    "metadata_source": f"profile:{PROFILE_NAME}",
}


def _rows_from_pcm(pcm: PcmBuffer) -> list[list[int]]:
    """Kernel-form rows for in-domain PCM (read_pcm16 output is in-domain)."""
    return [[int(sample * 32768.0) for sample in row] for row in pcm.channels]


@contextmanager
def _pinned_engine(name: str):
    previous = os.environ.get(_engine.ENGINE_ENV_VAR)
    os.environ[_engine.ENGINE_ENV_VAR] = name
    try:
        yield
    finally:
        if previous is None:
            os.environ.pop(_engine.ENGINE_ENV_VAR, None)
        else:
            os.environ[_engine.ENGINE_ENV_VAR] = previous


@unittest.skipIf(
    _wwise_wem_native is None,
    "native extension _wwise_wem_native is not installed; "
    "build it with: cd crates/wem-python && maturin develop -F extension-module",
)
class EngineParityTests(unittest.TestCase):
    def test_both_engines_are_byte_identical_on_reference_input(self):
        pcm = read_pcm16(INPUT)
        profile = load_wem_profile(PROFILE_NAME)
        reference = REFERENCE.read_bytes()

        with _pinned_engine("python"):
            python_result = Encoder(profile).encode_pcm(pcm)
        with _pinned_engine("native"):
            native_result = Encoder(profile).encode_pcm(pcm)
        # The internal pure-Python path and the raw binding, directly.
        internal_result = Encoder(profile)._encode_pcm_python(pcm)
        direct = _wwise_wem_native.Encoder(PROFILE_NAME).encode_pcm(
            pcm.sample_rate, _rows_from_pcm(pcm)
        )

        for label, result in (
            ("python-pinned facade", python_result),
            ("native-pinned facade", native_result),
            ("internal python path", internal_result),
        ):
            with self.subTest(engine=label):
                self.assertEqual(result.data, reference)
                self.assertEqual(
                    hashlib.sha256(result.data).hexdigest(), EXPECTED_SHA256
                )
                self.assertEqual(result.stats.to_legacy_dict(), EXPECTED_STATS)
                self.assertEqual(result.sha256, EXPECTED_SHA256)

        self.assertEqual(bytes(direct.data), reference)
        self.assertEqual(direct.sha256(), EXPECTED_SHA256)
        self.assertEqual(native_result.stats.to_legacy_dict(), EXPECTED_STATS)
        self.assertEqual(native_result.sha256, direct.sha256())

    def test_default_auto_prefers_the_native_engine_when_importable(self):
        pcm = read_pcm16(INPUT)
        profile = load_wem_profile(PROFILE_NAME)
        with _pinned_engine("auto"):
            self.assertEqual(_engine.active_engine(), "native")
            result = Encoder(profile).encode_pcm(pcm)
        self.assertEqual(result.sha256, EXPECTED_SHA256)

    def test_error_paths_match_across_engines(self):
        profile = load_wem_profile(PROFILE_NAME)
        short = PcmBuffer(44100, tuple(tuple(0.0 for _ in range(100)) for _ in range(6)))
        geometry = PcmBuffer(44100, tuple(tuple(0.0 for _ in range(4096)) for _ in range(2)))

        for engine in ("python", "native"):
            with self.subTest(engine=engine):
                with _pinned_engine(engine):
                    with self.assertRaisesRegex(ValueError, "at least 4096 frames"):
                        Encoder(profile).encode_pcm(short)
                    with self.assertRaisesRegex(
                        ValueError, "differs from encoder profile"
                    ):
                        Encoder(profile).encode_pcm(geometry)
        # The rejection conditions are identical: both engines pre-validate
        # through the same facade checks, so the messages must match too.
        profile2 = load_wem_profile(PROFILE_NAME)
        with _pinned_engine("python"):
            with self.assertRaises(ValueError) as python_cm:
                Encoder(profile2).encode_pcm(short)
        with _pinned_engine("native"):
            with self.assertRaises(ValueError) as native_cm:
                Encoder(profile2).encode_pcm(short)
        self.assertEqual(str(native_cm.exception), str(python_cm.exception))

    def test_out_of_domain_pcm_raises_only_when_native_is_pinned(self):
        profile = load_wem_profile(PROFILE_NAME)
        # 4095.0 * 32768 is outside the signed-16 range: out of domain.
        pcm = PcmBuffer(
            44100,
            tuple(tuple(4095.0 for _ in range(4096)) for _ in range(6)),
        )
        with _pinned_engine("native"):
            with self.assertRaisesRegex(ValueError, "integer signed-16 sample"):
                Encoder(profile).encode_pcm(pcm)
        # The pure-Python implementation accepts arbitrary float domains.
        with _pinned_engine("python"):
            result = Encoder(profile).encode_pcm(pcm)
        self.assertIsInstance(result.data, bytes)
        self.assertGreater(len(result.data), 0)

    def test_native_pin_without_extension_raises_clear_import_error(self):
        code = r"""
import os
import sys


class _NativeBlocker:
    def find_spec(self, name, path=None, target=None):
        if name == "_wwise_wem_native":
            raise ImportError("native extension blocked for this test")
        return None


sys.meta_path.insert(0, _NativeBlocker())
sys.path.insert(0, %r)
os.environ["WWISE_WEM_ENGINE"] = "native"

from wwise_wem import Encoder, load_wem_profile
from wwise_wem.model import PcmBuffer

profile = load_wem_profile("wwise2013-6ch-44100")
pcm = PcmBuffer(44100, tuple(tuple(0.0 for _ in range(4096)) for _ in range(6)))
try:
    Encoder(profile).encode_pcm(pcm)
except ImportError as error:
    message = str(error)
    assert "_wwise_wem_native" in message, message
    assert "WWISE_WEM_ENGINE" in message, message
    print("import-error-ok")
else:
    raise SystemExit("expected ImportError for pinned native engine")
""" % str(SRC)
        environment = dict(os.environ)
        environment["PYTHONPATH"] = str(SRC)
        completed = subprocess.run(
            [sys.executable, "-c", code],
            capture_output=True,
            text=True,
            env=environment,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("import-error-ok", completed.stdout)

    def test_engine_status_reports_the_active_choice(self):
        with _pinned_engine("native"):
            status = _engine.engine_status()
            self.assertEqual(
                (status.requested, status.native_available, status.active),
                ("native", True, "native"),
            )
        with _pinned_engine("python"):
            self.assertEqual(_engine.engine_status().active, "python")
        with _pinned_engine("auto"):
            self.assertEqual(_engine.engine_status().active, "native")


if __name__ == "__main__":
    unittest.main()
