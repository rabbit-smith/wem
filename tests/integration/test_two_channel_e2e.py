"""Formal 2ch/48000 end-to-end contract: facade encode, header geometry, decode.

Closes the last 2ch/48k lane end to end against the repository's own
reference ground-truth decoder (``scripts/decode_wem.py``): the facade
encodes a deterministic 2ch/48k stream (>= 4096 frames, with transient
content) through :func:`wwise_wem.encode_wav` and
:func:`wwise_wem.encode_pcm_wav`, the produced WEM must carry the correct
2ch/48k container geometry and profile setup packet, and decoding the WEM
back to PCM with the reference decoder functions must reconstruct the
profile-conditioned input to a correlation of at least 0.98 per channel
(steady region).

The 2ch profile's statically hosted analysis fields are byte-verified from
the paired encoder build; its init-computed psychoacoustic surfaces remain
behavior-paired candidates (non-static certificates in the r9 replacement
ledger). The test signal therefore uses band-limited content (pink-style
noise plus gentle decaying bursts) that the registered calibration
demonstrably reconstructs.  Correlation is the reconstruction-quality bar,
not a losslessness claim.

The reference decoder is a test asset imported by explicit file path, in
the same spirit as ``wwise_wem_reference``: it validates the output, it is
never an engine.
"""

from __future__ import annotations

import hashlib
import importlib.util
import sys
import unittest
import wave
from pathlib import Path

import numpy as np

from wwise_wem import (
    WwiseVorbisProfile,
    encode_pcm_wav,
    encode_wav,
    load_wem_profile,
    resolve_wem_profile,
)
from wwise_wem.profiles.bundle import load_profile_bundle
from wwise_wem_reference.analysis.preprocessing.conditioner import InputConditioner
from wwise_wem_reference.profiles.assembly import assemble_analysis_resources

ROOT = Path(__file__).resolve().parents[2]
PROFILE_NAME = "wwise2013-2ch-48000"
PROFILE_DIR = ROOT / "src" / "wwise_wem" / "data" / "profiles" / PROFILE_NAME
SAMPLE_RATE = 48000
CHANNELS = 2
# >= 4096 frames required by the E2E contract; 16384 keeps the native
# encode fast while covering several short/long block-size transitions.
FRAMES = 16384
# Steady-region guard: the first 1024 decoded samples overlap the MDCT
# onset of the stream and are excluded from the correlation window.
STEADY_OFFSET = 1024
# This is a decoder quality smoke bar. Byte identity is enforced separately.
CORRELATION_FLOOR = 0.98

_SPEC = importlib.util.spec_from_file_location(
    "decode_cdlc_wem", ROOT / "scripts" / "decode_wem.py"
)
if _SPEC is None or _SPEC.loader is None:
    raise ImportError("scripts/decode_wem.py is required for this suite")
DECODE = importlib.util.module_from_spec(_SPEC)

sys.modules[_SPEC.name] = DECODE
_SPEC.loader.exec_module(DECODE)


def _pink(seed: int, amplitude: float, length: int) -> np.ndarray:
    """Deterministic pink-style noise (cumulative low-pass of white noise)."""
    rng = np.random.default_rng(seed)
    values = np.cumsum(rng.standard_normal(length))
    return amplitude * values / np.abs(values).max()


def _bursts(
    seed: int, starts: tuple[int, ...], length: int, decay: float, amplitude: float
) -> tuple[np.ndarray, np.ndarray]:
    """Gentle decaying transient bursts (one pair per start, channel gains)."""
    rng = np.random.default_rng(seed)
    total = FRAMES
    ch0 = np.zeros(total)
    ch1 = np.zeros(total)
    for start in starts:
        extent = min(length, total - start)
        envelope = np.exp(-np.arange(extent) / decay)
        ch0[start : start + extent] += amplitude * envelope * rng.standard_normal(extent)
        ch1[start : start + extent] += (
            0.8 * amplitude * envelope * rng.standard_normal(extent)
        )
    return ch0, ch1


def _test_streams() -> tuple[np.ndarray, np.ndarray]:
    """Deterministic in-domain int16 streams: 2ch/48k, pink + transient bursts."""
    base0 = _pink(11, 0.3, FRAMES)
    base1 = _pink(12, 0.3, FRAMES)
    burst0, burst1 = _bursts(13, (1024, 5000, 9728, 13500), 48, 150.0, 0.07)
    return (
        np.round(np.clip(base0 + burst0, -1.0, 1.0) * 32767),
        np.round(np.clip(base1 + burst1, -1.0, 1.0) * 32767),
    )


def _write_wav(path: Path, ch0: np.ndarray, ch1: np.ndarray) -> Path:
    interleaved = np.empty(len(ch0) * 2, dtype="<i2")
    interleaved[0::2] = ch0.astype("<i2")
    interleaved[1::2] = ch1.astype("<i2")
    with wave.open(str(path), "wb") as handle:
        handle.setnchannels(CHANNELS)
        handle.setsampwidth(2)
        handle.setframerate(SAMPLE_RATE)
        handle.writeframes(interleaved.tobytes())
    return path


def _decode_wem_int16(wem_bytes: bytes) -> tuple[np.ndarray, dict]:
    """Reference-decode a 2ch/48k WEM to float64 PCM (libvorbis float domain)."""
    context = DECODE.build_context(PROFILE_DIR, channels=CHANNELS, sample_rate=SAMPLE_RATE)
    _data, packets, _seek, _fmt = DECODE.parse_container(wem_bytes)
    _setup, audio_packets = packets[0], packets[1:]
    result = DECODE.run_decode_2ch(context, audio_packets)
    return np.asarray(result["out_buf"], dtype=np.float64), result


def _to_int16_domain(float_pcm: np.ndarray) -> np.ndarray:
    """vgmstream CONV_FLT_S16 conversion: clamp(round(x * 32767), -32768, 32767)."""
    return np.clip(np.round(float_pcm * 32767.0), -32768, 32767)


class TwoChannelResolutionTests(unittest.TestCase):
    """Positive contract for the newly registered 2ch/48000 geometry."""

    def test_geometry_resolution_returns_the_registered_profile(self) -> None:
        profile = resolve_wem_profile(CHANNELS, SAMPLE_RATE)
        self.assertIsInstance(profile, WwiseVorbisProfile)
        self.assertEqual(profile.name, PROFILE_NAME)
        self.assertEqual(profile.key.channels, CHANNELS)
        self.assertEqual(profile.key.sample_rate, SAMPLE_RATE)

    def test_named_load_is_the_same_registry_instance(self) -> None:
        by_name = load_wem_profile(PROFILE_NAME)
        by_geometry = resolve_wem_profile(CHANNELS, SAMPLE_RATE)
        self.assertIs(by_name, by_geometry)

    def test_setup_packet_matches_the_profile_key_identity(self) -> None:
        profile = load_wem_profile(PROFILE_NAME)
        setup = profile.setup_packet()
        self.assertEqual(len(setup), 215)
        digest = hashlib.sha256(setup).hexdigest()
        self.assertEqual(profile.setup_sha256, digest)
        self.assertIn(digest, profile.key.quality_setup_identity)

    def test_unsupported_geometry_still_rejected(self) -> None:
        # 2ch/44100 remains outside the supported surface: only the exact
        # registered geometries may resolve, so a geometry change must
        # still produce the explicit rejection instead of a silent pick.
        with self.assertRaisesRegex(ValueError, "no Wwise 2013.2 profile"):
            resolve_wem_profile(CHANNELS, 44100)
        with self.assertRaisesRegex(ValueError, "no Wwise 2013.2 profile"):
            resolve_wem_profile(4, SAMPLE_RATE)


class TwoChannelEncodeGeometryTests(unittest.TestCase):
    """The facade encodes 2ch/48k WAV and the WEM header carries 2ch/48k."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.directory = None
        ch0, ch1 = _test_streams()
        cls.streams = (ch0, ch1)
        from tempfile import TemporaryDirectory

        cls._tmp = TemporaryDirectory()
        cls.directory = Path(cls._tmp.name)
        cls.wav_path = _write_wav(cls.directory / "two_ch.wav", ch0, ch1)
        cls.result = encode_wav(cls.wav_path)

    @classmethod
    def tearDownClass(cls) -> None:
        cls._tmp.cleanup()

    def test_encodes_through_implicit_geometry_resolution(self) -> None:
        stats = self.result.stats
        self.assertEqual(stats.channels, CHANNELS)
        self.assertEqual(stats.pcm_frames, FRAMES)
        self.assertEqual(stats.metadata_source, f"profile:{PROFILE_NAME}")
        self.assertGreater(stats.audio_packets, 0)
        self.assertEqual(
            stats.short_packets + stats.long_packets, stats.audio_packets
        )

    def test_wem_header_carries_the_two_channel_geometry(self) -> None:
        _data, packets, _seek, fmt = DECODE.parse_container(bytes(self.result.data))
        self.assertEqual(fmt["nChannels"], CHANNELS)
        self.assertEqual(fmt["nSamplesPerSec"], SAMPLE_RATE)
        # The reference parser reports format tag and channel mask as hex
        # strings; normalize before the exact-value assertions.
        self.assertEqual(int(fmt["wFormatTag"], 16), 0xFFFF)
        self.assertEqual(int(fmt["dwChannelMask"], 16), 3)
        self.assertEqual(fmt["uBlocksize0Pow"], 8)
        self.assertEqual(fmt["uBlocksize1Pow"], 11)
        # The profile's setup packet leads the audio stream.
        profile = load_wem_profile(PROFILE_NAME)
        self.assertEqual(bytes(profile.setup_packet()), packets[0])
        # Packet stream geometry matches the encoder's reported counts.
        self.assertEqual(len(packets[1:]), self.result.stats.audio_packets)
        # Container metadata registered in the profile matches the bytes
        # the encoder actually wrote (block sizes, channels, rate).
        meta = profile.container_metadata
        self.assertEqual(meta.nChannels, fmt["nChannels"])
        self.assertEqual(meta.nSamplesPerSec, fmt["nSamplesPerSec"])
        self.assertEqual(meta.dwChannelMask, int(fmt["dwChannelMask"], 16))
        self.assertEqual(2**meta.uBlocksize0Pow, 256)
        self.assertEqual(2**meta.uBlocksize1Pow, 2048)

    def test_encode_pcm_wav_is_byte_identical_to_encode_wav(self) -> None:
        converted = encode_pcm_wav(self.wav_path)
        self.assertEqual(bytes(converted.data), bytes(self.result.data))
        self.assertEqual(converted.stats, self.result.stats)


class TwoChannelRoundTripTests(unittest.TestCase):
    """Decode the facade's WEM with the reference decoder: correlation bar."""

    @classmethod
    def setUpClass(cls) -> None:
        ch0, ch1 = _test_streams()
        rows = tuple(
            tuple(float(sample) / 32768.0 for sample in row) for row in (ch0, ch1)
        )
        resources = assemble_analysis_resources(
            load_profile_bundle(profile=PROFILE_NAME, verify_all=False)
        )
        if resources.input_conditioner is None:
            raise AssertionError("2ch profile must select input conditioning")
        cls.conditioned_streams = InputConditioner(
            CHANNELS, resources.input_conditioner
        ).process(rows)
        from tempfile import TemporaryDirectory

        with TemporaryDirectory() as directory:
            wav_path = _write_wav(Path(directory) / "two_ch.wav", ch0, ch1)
            cls.result = encode_wav(wav_path)
        _float_pcm, info = _decode_wem_int16(bytes(cls.result.data))
        cls.decoded_int16 = _to_int16_domain(_float_pcm)
        cls.decode_info = info

    def test_round_trip_reconstructs_profile_conditioned_input(self) -> None:
        # The encoded stream must be fully consumable by the reference
        # decoder: every audio packet closes at the bit boundary.
        self.assertTrue(self.decode_info["bit_closure_ok"])
        self.assertTrue(self.decode_info["strict_closure_ok"])

        frames = self.result.stats.pcm_frames
        window = frames - STEADY_OFFSET
        for channel in range(CHANNELS):
            reference = self.conditioned_streams[channel][STEADY_OFFSET:]
            reconstructed = self.decoded_int16[channel, STEADY_OFFSET : STEADY_OFFSET + window]
            self.assertEqual(len(reconstructed), len(reference))
            correlation = float(
                np.corrcoef(reference, reconstructed)[0, 1]
            )
            self.assertGreaterEqual(
                correlation,
                CORRELATION_FLOOR,
                f"channel {channel} reconstruction correlation "
                f"{correlation:.6f} below {CORRELATION_FLOOR}",
            )

    def test_short_and_long_blocks_are_both_used(self) -> None:
        stats = self.result.stats
        self.assertGreater(stats.short_packets, 0)
        self.assertGreater(stats.long_packets, 0)


if __name__ == "__main__":
    unittest.main()
