"""Extended input forms: single-path consistency and short-input semantics.

New-format inputs (24-bit PCM, 32-bit float32 WAV, raw PCM) are converted at
the adapter boundary into the signed-16 domain.  These tests lock two
properties:

* converted inputs land on exactly the in-domain floats of the equivalent
  signed-16 stream, so the encoder consumes format-agnostic values;
* the facade path (the native kernel, ``wwise_wem._core``) and the direct
  ``wwise_wem_reference`` import (the test-time reference oracle, a pure
  test asset and not an engine) produce byte-identical WEM output for them,
  and encoding is deterministic.

Single-path environment: the native extension ``wwise_wem._core`` is a
required runtime asset; this suite deliberately does not design skip cases
for native-absent environments.  Short inputs (< 4096 frames) keep the
explicit rejection (INPUT_TOO_SHORT semantics) on every entry point.
"""

from __future__ import annotations

import struct
import unittest
import wave
from pathlib import Path

import wwise_wem as W
from wwise_wem import _core as core_mod
from wwise_wem.adapters.raw import read_raw_pcm
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem.application.models import EncodeResult
from wwise_wem.profiles.registry import load_wem_profile
from wwise_wem_reference import python_engine
from wwise_wem_reference.container.model import ContainerPlan

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
INPUT = FIXTURES / "input.wav"
REFERENCE = FIXTURES / "reference.wem"
PROFILE_NAME = "wwise2013-6ch-44100"


def _synthetic_int16(frames: int, channels: int = 6) -> list[int]:
    """Deterministic signed-16 stream; one value per (channel, frame)."""
    return [
        ((frame * 7 + channel * 11 + 3) % 64536 - 32768)
        for channel in range(channels)
        for frame in range(frames)
    ]


def _int16_bytes(values: list[int]) -> bytes:
    return b"".join(v.to_bytes(2, "little", signed=True) for v in values)


def _int24_bytes(values: list[int]) -> bytes:
    return b"".join((v << 8).to_bytes(3, "little", signed=True) for v in values)


def _float32_bytes(values: list[int]) -> bytes:
    return struct.pack(f"<{len(values)}f", *[v / 32768.0 for v in values])


def _write_int16_wav(path: Path, values: list[int], channels: int, rate: int) -> None:
    with wave.open(str(path), "wb") as target:
        target.setnchannels(channels)
        target.setsampwidth(2)
        target.setframerate(rate)
        target.writeframes(_int16_bytes(values))


def _write_int24_wav(path: Path, values: list[int], channels: int, rate: int) -> None:
    with wave.open(str(path), "wb") as target:
        target.setnchannels(channels)
        target.setsampwidth(3)
        target.setframerate(rate)
        target.writeframes(_int24_bytes(values))


def _write_float32_wav(path: Path, values: list[int], channels: int, rate: int) -> None:
    body = _float32_bytes(values)
    fmt = struct.pack(
        "<HHIIHH", 3, channels, rate, rate * channels * 4, channels * 4, 32
    )
    with open(path, "wb") as handle:
        handle.write(b"RIFF")
        handle.write(struct.pack("<I", 36 + len(body)))
        handle.write(b"WAVE")
        handle.write(b"fmt ")
        handle.write(struct.pack("<I", len(fmt)))
        handle.write(fmt)
        handle.write(b"data")
        handle.write(struct.pack("<I", len(body)))
        handle.write(body)


class ExtendedInputDomainTests(unittest.TestCase):
    """Conversion lands in-domain; encoding is deterministic on the facade."""

    def test_converted_inputs_match_the_int16_domain(self):
        values = _synthetic_int16(5000)
        pcm16 = read_raw_pcm(
            _int16_bytes(values), sample_rate=44100, channels=6, bits_per_sample=16
        )
        pcm24 = read_raw_pcm(
            _int24_bytes(values), sample_rate=44100, channels=6, bits_per_sample=24
        )
        pcm32 = read_raw_pcm(
            _float32_bytes(values), sample_rate=44100, channels=6, bits_per_sample=32
        )
        self.assertEqual(pcm16, pcm24)
        self.assertEqual(pcm24, pcm32)

    def test_converted_wav_forms_match_the_int16_domain(self):
        values = _synthetic_int16(5000)
        import tempfile

        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            wav16 = base / "a16.wav"
            wav24 = base / "a24.wav"
            wav32 = base / "a32f.wav"
            _write_int16_wav(wav16, values, 6, 44100)
            _write_int24_wav(wav24, values, 6, 44100)
            _write_float32_wav(wav32, values, 6, 44100)
            pcm16 = read_pcm_wav(wav16)
            pcm24 = read_pcm_wav(wav24)
            pcm32 = read_pcm_wav(wav32)
            self.assertEqual(pcm16, pcm24)
            self.assertEqual(pcm24, pcm32)

    def test_packed_wav_path_matches_typed_pcm(self):
        packed = W.encode(INPUT)
        typed = W.encode(read_pcm_wav(INPUT))
        self.assertEqual(packed.data, typed.data)
        self.assertEqual(packed.sha256, typed.sha256)

    def test_encoding_is_deterministic_across_runs_and_forms(self):
        import tempfile

        values = _synthetic_int16(4096)
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            wav24 = base / "a24.wav"
            wav32 = base / "a32f.wav"
            _write_int24_wav(wav24, values, 6, 44100)
            _write_float32_wav(wav32, values, 6, 44100)

            first = W.encode(wav24)
            again = W.encode(wav32)
            raw24 = W.encode(W.RawPcm(_int24_bytes(values), 44100, 6, "s24le"))
            raw32 = W.encode(W.RawPcm(_float32_bytes(values), 44100, 6, "f32le"))
            # Same samples through four entry forms: one byte stream.
            self.assertEqual(first.data, again.data)
            self.assertEqual(first.data, raw24.data)
            self.assertEqual(first.data, raw32.data)
            self.assertEqual(first.stats.pcm_frames, 4096)

    def test_short_input_keeps_the_explicit_rejection(self):
        import tempfile

        values = _synthetic_int16(4095)
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            wav24 = base / "a24.wav"
            wav32 = base / "a32f.wav"
            _write_int24_wav(wav24, values, 6, 44100)
            _write_float32_wav(wav32, values, 6, 44100)

            cases = (
                lambda: W.encode(wav24),
                lambda: W.encode(wav32),
                lambda: W.encode(W.RawPcm(_int24_bytes(values), 44100, 6, "s24le")),
                lambda: W.encode(W.RawPcm(_int16_bytes(values), 44100, 6, "s16le")),
            )
            for index, action in enumerate(cases):
                with self.subTest(form=index):
                    with self.assertRaisesRegex(
                        ValueError, "at least 4096 frames"
                    ):
                        action()

    def test_geometry_mismatch_keeps_the_explicit_rejection(self):
        # 2ch/44100 is not a registered profile (the 2ch profile is 48000):
        # unsupported geometry must still fail with the explicit rejection.
        values = _synthetic_int16(4096, channels=2)
        with self.assertRaisesRegex(ValueError, "no Wwise 2013.2 profile"):
            W.encode(W.RawPcm(_int16_bytes(values), 44100, 2, "s16le"))


class ExtendedInputConsistencyTests(unittest.TestCase):
    """Facade (native kernel) vs the direct reference import: byte identity.

    ``wwise_wem_reference`` is imported directly as a test-time oracle; it
    is a pure test asset, not a runtime engine.  The facade runs its single
    execution path — the native kernel — unconditionally.  Both must
    produce identical WEM bytes.
    """

    def _oracle_encode(self, pcm) -> EncodeResult:
        profile = load_wem_profile(PROFILE_NAME)
        return python_engine.encode_pcm_python(
            profile=profile,
            container=ContainerPlan.from_profile(profile),
            pcm=pcm,
        )

    def test_golden_input_via_extended_entry_matches_reference_and_oracle(self):
        pcm = read_pcm_wav(INPUT)
        golden = REFERENCE.read_bytes()

        facade = W.encode(INPUT, profile=PROFILE_NAME)
        oracle = self._oracle_encode(pcm)
        direct_core = core_mod.Encoder(PROFILE_NAME).encode_pcm(
            44100,
            [[int(sample * 32768.0) for sample in row] for row in pcm.channels],
        )
        self.assertEqual(facade.data, golden)
        self.assertEqual(bytes(oracle.data), golden)
        self.assertEqual(bytes(direct_core.data), golden)
        self.assertEqual(facade.data, oracle.data)
        self.assertEqual(facade.sha256, direct_core.sha256())

    def test_converted_inputs_are_byte_identical_between_facade_and_oracle(self):
        import tempfile

        values = _synthetic_int16(4096)
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            wav24 = base / "a24.wav"
            wav32 = base / "a32f.wav"
            _write_int24_wav(wav24, values, 6, 44100)
            _write_float32_wav(wav32, values, 6, 44100)
            forms = [
                ("wav-24", lambda: W.encode(wav24)),
                ("wav-32f", lambda: W.encode(wav32)),
                (
                    "raw-24",
                    lambda: W.encode(W.RawPcm(_int24_bytes(values), 44100, 6, "s24le")),
                ),
                (
                    "raw-32f",
                    lambda: W.encode(W.RawPcm(_float32_bytes(values), 44100, 6, "f32le")),
                ),
                (
                    "raw-16",
                    lambda: W.encode(W.RawPcm(_int16_bytes(values), 44100, 6, "s16le")),
                ),
            ]
            # The oracle encodes the one in-domain PCM every form converts
            # to; all facade results must equal it byte-for-byte.
            pcm = read_raw_pcm(
                _int16_bytes(values),
                sample_rate=44100,
                channels=6,
                bits_per_sample=16,
            )
            oracle_result = self._oracle_encode(pcm)
            for label, action in forms:
                with self.subTest(form=label):
                    facade_result = action()
                    self.assertEqual(facade_result.data, oracle_result.data, label)
                    self.assertGreater(len(facade_result.data), 0)
                    self.assertEqual(
                        facade_result.stats.to_dict(),
                        oracle_result.stats.to_dict(),
                        label,
                    )


if __name__ == "__main__":
    unittest.main()
