"""Frozen transcendental tables: registration, equality, strict domain, zero live calls."""
from __future__ import annotations

import math
import os
import struct
import subprocess
import sys
import tempfile
import unittest
import wave
from pathlib import Path

from wwise_wem_reference.analysis.config import FrozenMathTables, make_wwise_psy_look
from wwise_wem_reference._tmath import math_bits
from wwise_wem_reference.profiles.assembly import assemble_encoder_profile_resources
from wwise_wem.profiles.bundle import load_profile_bundle
from wwise_wem_reference.profiles.frozen import load_frozen_tables

def _ulp_ordered(value: float) -> int:
    """Total-order integer for float64 ULP-distance comparisons."""
    bits = struct.unpack("<q", struct.pack("<d", value))[0]
    return bits if bits >= 0 else -(bits & 0x7FFFFFFFFFFFFFFF)


ROOT = Path(__file__).resolve().parents[3]
ENUMERABLE_SITES = (
    "config.ln",
    "spectrum.cos",
    "spectrum.sin",
    "transform.window_short.sin",
)

_SUBPROCESS_ENCODE = """
import os, sys
for path_entry in os.environ["WEM_SRC"].split(os.pathsep):
    sys.path.insert(0, path_entry)
from pathlib import Path
from wwise_wem import encode_wav
result = encode_wav(Path(os.environ["WEM_INPUT"]))
from wwise_wem_reference._tmath import write_recording
counts = write_recording()
print(",".join(f"{k}={v}" for k, v in sorted(counts.items())))
"""


def _write_small_wav(path: str, frames: int = 4200) -> None:
    with wave.open(path, "wb") as handle:
        handle.setnchannels(6)
        handle.setsampwidth(2)
        handle.setframerate(44100)
        samples = []
        for frame in range(frames):
            value = int(9000 * math.sin(2.0 * math.pi * 440.0 * frame / 44100.0))
            samples.extend([value] * 6)
        handle.writeframesraw(struct.pack(f"<{len(samples)}h", *samples))


class FrozenTableRegistrationTests(unittest.TestCase):
    def test_manifest_registers_frozen_tables_and_loader_round_trips(self) -> None:
        bundle = load_profile_bundle()
        manifest = bundle.runtime_manifest
        self.assertIn("analysis.frozen-tables", manifest.resources)
        tables = load_frozen_tables(manifest.resource("analysis.frozen-tables"))
        self.assertIsInstance(tables, FrozenMathTables)
        self.assertEqual(set(tables.window_halves), {256, 2048})
        self.assertEqual(len(tables.coordinate_ln), 128)
        self.assertEqual(set(tables.fft_twiddles), {2 << k for k in range(11)})

    def test_assembled_analysis_resources_carry_frozen_tables(self) -> None:
        resources = assemble_encoder_profile_resources(load_profile_bundle())
        self.assertIsInstance(resources.analysis.frozen, FrozenMathTables)


class FrozenTableEqualityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tables = load_frozen_tables(
            load_profile_bundle().runtime_manifest.resource("analysis.frozen-tables")
        )

    def test_windows_reproduce_reference_synthesis(self) -> None:
        from wwise_wem_reference.analysis.dsp.transform import vorbis_window

        for size in (256, 2048):
            reference = vorbis_window(size)
            frozen = self.tables.window_halves[size]
            self.assertEqual(len(frozen), size // 2)
            self.assertEqual(list(frozen) + list(frozen)[::-1], reference)

    def test_twiddles_stay_within_libm_ulp_envelope(self) -> None:
        # The frozen table is the contract, not the host libm: the whole
        # point of freezing was to stop trusting per-platform transcendentals.
        # Windows msvcrt differs from the macOS-generated table by 1 ULP on
        # sin (observed in CI), so only a tight ULP envelope is asserted;
        # the encoder itself never calls the host libm for these values.
        for length, (cos_value, sin_value) in self.tables.fft_twiddles.items():
            angle = -2.0 * math.pi / length  # pure IEEE; bit-identical inputs
            for frozen, host in ((cos_value, math.cos(angle)), (sin_value, math.sin(angle))):
                distance = abs(_ulp_ordered(frozen) - _ulp_ordered(host))
                self.assertLessEqual(distance, 2, f"twiddle drift at length {length}")

    def test_coordinate_ln_covers_every_short_index(self) -> None:
        for index in range(128):
            frequency = (float(index) + 0.5) * 44100.0 / 256.0
            self.assertAlmostEqual(
                self.tables.coordinate_ln[math_bits(frequency)],
                math.log(frequency),
                delta=1e-15,
            )

    def test_psy_look_construction_uses_only_frozen_domain(self) -> None:
        surface = (
            assemble_encoder_profile_resources(load_profile_bundle()).analysis.short_surface
        )
        look = make_wwise_psy_look(surface, frozen_ln=self.tables.coordinate_ln)
        baseline = make_wwise_psy_look(surface)
        self.assertEqual(look, baseline)
        with self.assertRaisesRegex(ValueError, "frozen ln domain"):
            make_wwise_psy_look(surface, frozen_ln={})


class FrozenLiveCallTests(unittest.TestCase):
    def test_encode_with_frozen_tables_makes_zero_live_transcendentals(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            record_dir = os.path.join(directory, "capture")
            wav_path = os.path.join(directory, "small.wav")
            _write_small_wav(wav_path)
            environment = dict(os.environ)
            environment.update(
                {
                    "WEM_TMATH_RECORD": record_dir,
                    "WEM_SRC": str(ROOT / "src") + os.pathsep + str(ROOT / "reference"),
                    "WEM_INPUT": wav_path,
                    "PYTHONDONTWRITEBYTECODE": "1",
                }
            )
            completed = subprocess.run(
                [sys.executable, "-c", _SUBPROCESS_ENCODE],
                env=environment,
                capture_output=True,
                text=True,
                timeout=300,
                check=True,
            )
            counts = dict(
                pair.split("=") for pair in completed.stdout.strip().split(",") if pair
            )
            for site in ENUMERABLE_SITES:
                self.assertEqual(int(counts.get(site, -1)), 0, f"{site} fired at runtime")


if __name__ == "__main__":
    unittest.main()
