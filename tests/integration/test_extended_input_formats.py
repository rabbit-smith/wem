"""Extended input forms: dual-engine consistency and short-input semantics.

New-format inputs (24-bit PCM, 32-bit float32 WAV, raw PCM) are converted at
the adapter boundary into the signed-16 domain.  These tests lock two
properties:

* converted inputs land on exactly the in-domain floats of the equivalent
  signed-16 stream, so both engines see format-agnostic values;
* encoding is deterministic and byte-identical across both engines, and
  short inputs (< 4096 frames) keep the explicit rejection (INPUT_TOO_SHORT
  semantics) on every entry point.
"""

from __future__ import annotations

import os
import struct
import unittest
import wave
from contextlib import contextmanager
from pathlib import Path

import wwise_wem as W
from wwise_wem import _engine
from wwise_wem.adapters.raw import read_raw_pcm
from wwise_wem.adapters.wav import read_pcm_wav

try:
    from wwise_wem import _native as native_mod  # in-package extension
except ImportError:
    try:
        import _wwise_wem_native as native_mod  # legacy top-level name
    except ImportError:
        native_mod = None

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
INPUT = FIXTURES / "input.wav"
REFERENCE = FIXTURES / "reference.wem"
PROFILE_NAME = "wwise2013-6ch-44100"


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
    return b"".join(
        (v << 8).to_bytes(3, "little", signed=True) for v in values
    )


def _float32_bytes(values: list[int]) -> bytes:
    return struct.pack(
        f"<{len(values)}f", *[v / 32768.0 for v in values]
    )


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
    """Pure-Python invariants: conversion lands in-domain, deterministically."""

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

    def test_encode_pcm_wav_int16_matches_encode_wav(self):
        with _pinned_engine("python"):
            legacy = W.encode_wav(INPUT)
            extended = W.encode_pcm_wav(INPUT)
        self.assertEqual(legacy.data, extended.data)
        self.assertEqual(legacy.sha256, extended.sha256)

    def test_encoding_is_deterministic_across_runs_and_forms(self):
        import tempfile

        values = _synthetic_int16(4096)
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            wav24 = base / "a24.wav"
            wav32 = base / "a32f.wav"
            _write_int24_wav(wav24, values, 6, 44100)
            _write_float32_wav(wav32, values, 6, 44100)

            with _pinned_engine("python"):
                first = W.encode_pcm_wav(wav24)
                again = W.encode_pcm_wav(wav32)
                raw24 = W.encode_raw_pcm(
                    _int24_bytes(values),
                    sample_rate=44100,
                    channels=6,
                    bits_per_sample=24,
                )
                raw32 = W.encode_raw_pcm(
                    _float32_bytes(values),
                    sample_rate=44100,
                    channels=6,
                    bits_per_sample=32,
                )
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
                (lambda: W.encode_pcm_wav(wav24), None),
                (lambda: W.encode_pcm_wav(wav32), None),
                (
                    lambda: W.encode_raw_pcm(
                        _int24_bytes(values),
                        sample_rate=44100,
                        channels=6,
                        bits_per_sample=24,
                    ),
                    None,
                ),
                (
                    lambda: W.encode_raw_pcm(
                        _int16_bytes(values),
                        sample_rate=44100,
                        channels=6,
                        bits_per_sample=16,
                    ),
                    None,
                ),
            )
            for index, (action, _) in enumerate(cases):
                with self.subTest(form=index):
                    with self.assertRaisesRegex(
                        ValueError, "at least 4096 frames"
                    ):
                        action()

    def test_geometry_mismatch_keeps_the_explicit_rejection(self):
        values = _synthetic_int16(4096, channels=2)
        with self.assertRaisesRegex(ValueError, "no Wwise 2013.2 profile"):
            W.encode_raw_pcm(
                _int16_bytes(values),
                sample_rate=48000,
                channels=2,
                bits_per_sample=16,
            )


@unittest.skipIf(
    native_mod is None,
    "native extension (wwise_wem._native or legacy _wwise_wem_native) is not "
    "importable; build or install it to run dual-engine checks",
)
class ExtendedInputDualEngineTests(unittest.TestCase):
    """New-format inputs are byte-identical across both engines."""

    def test_golden_input_via_extended_entry_matches_reference(self):
        golden = REFERENCE.read_bytes()
        with _pinned_engine("python"):
            facade_python = W.encode_pcm_wav(INPUT, profile=PROFILE_NAME)
        with _pinned_engine("native"):
            facade_native = W.encode_pcm_wav(INPUT, profile=PROFILE_NAME)
        direct_native = native_mod.Encoder(PROFILE_NAME).encode_pcm(
            44100,
            [[int(s * 32768.0) for s in row]
             for row in read_pcm_wav(INPUT).channels],
        )
        self.assertEqual(facade_python.data, golden)
        self.assertEqual(facade_native.data, golden)
        self.assertEqual(bytes(direct_native.data), golden)
        self.assertEqual(facade_python.data, facade_native.data)

    def test_converted_inputs_are_byte_identical_across_engines(self):
        import tempfile

        values = _synthetic_int16(4096)
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            forms = []
            wav24 = base / "a24.wav"
            wav32 = base / "a32f.wav"
            _write_int24_wav(wav24, values, 6, 44100)
            _write_float32_wav(wav32, values, 6, 44100)
            forms.append(("wav-24", lambda: W.encode_pcm_wav(wav24)))
            forms.append(("wav-32f", lambda: W.encode_pcm_wav(wav32)))
            forms.append(
                (
                    "raw-24",
                    lambda: W.encode_raw_pcm(
                        _int24_bytes(values),
                        sample_rate=44100,
                        channels=6,
                        bits_per_sample=24,
                    ),
                )
            )
            forms.append(
                (
                    "raw-32f",
                    lambda: W.encode_raw_pcm(
                        _float32_bytes(values),
                        sample_rate=44100,
                        channels=6,
                        bits_per_sample=32,
                    ),
                )
            )
            forms.append(
                (
                    "raw-16",
                    lambda: W.encode_raw_pcm(
                        _int16_bytes(values),
                        sample_rate=44100,
                        channels=6,
                        bits_per_sample=16,
                    ),
                )
            )
            for label, action in forms:
                with self.subTest(form=label):
                    with _pinned_engine("python"):
                        python_result = action()
                    with _pinned_engine("native"):
                        native_result = action()
                    self.assertEqual(
                        python_result.data, native_result.data, label
                    )
                    self.assertEqual(python_result.sha256, native_result.sha256)
                    self.assertGreater(len(python_result.data), 0)
                    python_stats = {
                        key: value
                        for key, value in python_result.stats.to_legacy_dict().items()
                        if key != "engine"
                    }
                    native_stats = {
                        key: value
                        for key, value in native_result.stats.to_legacy_dict().items()
                        if key != "engine"
                    }
                    self.assertEqual(python_stats, native_stats, label)


if __name__ == "__main__":
    unittest.main()
