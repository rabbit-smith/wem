"""The 2ch/48 kHz configuration against the paired build's committed outputs.

Three files, one object, three datasets. The reference corpus is the six
representative signals and their committed containers; the stress corpus is
the two adversarial inputs, compared against their containers, the Python
oracle and a chunked stream; the scalar cases pin the container and seed
values that are per-layout constants read off the paired build. Each class
keeps its own test names and failure messages.
"""

from __future__ import annotations

import hashlib
import json
import unittest
import wave

from pathlib import Path
from tests.parity.wem_byte_compare import assert_wem_equal
from tests.two_channel_corpus_support import corpus_selection, render_wav
from typing import Any, ClassVar
from wwise_wem import WwiseProfile, WwiseVersion, _core
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem.application.encoder import Encoder
from wwise_wem_reference.container.wem import load_wem_parts_bytes
from wwise_wem_reference.profiles.artifact import resolve_selection
from wwise_wem_reference.python_engine import ContainerPlan, encode_pcm_python

# --------------------------------------------------------------------------
# merged from tests/parity/test_2ch_reference_corpus.py
# --------------------------------------------------------------------------

# Byte expectations for representative 2ch/48 kHz Wwise outputs.

ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / "tests" / "data" / "2ch-reference"
MANIFEST = json.loads((CORPUS / "manifest.json").read_text(encoding="utf-8"))


class TwoChannelReferenceCorpusTests(unittest.TestCase):
    def test_representative_inputs_match_paired_build_byte_for_byte(self) -> None:
        selection = corpus_selection(MANIFEST["profile"])
        self.assertEqual(MANIFEST["schema"], "wem.2ch-reference-corpus.v1")

        for case in MANIFEST["cases"]:
            with self.subTest(signal=case["name"]):
                input_path = CORPUS / case["input"]
                input_bytes = input_path.read_bytes()
                reference = (CORPUS / case["reference"]).read_bytes()
                pcm = read_pcm_wav(input_path)
                encoded = Encoder(selection).encode_pcm(pcm)

                self.assertEqual(input_bytes, render_wav(input_path.stem))
                self.assertEqual(
                    hashlib.sha256(input_bytes).hexdigest(),
                    case["input_sha256"],
                )
                self.assertEqual(
                    hashlib.sha256(reference).hexdigest(),
                    case["reference_sha256"],
                )
                self.assertEqual(pcm.sample_rate, MANIFEST["sample_rate"])
                self.assertEqual(pcm.channel_count, MANIFEST["channels"])
                self.assertEqual(pcm.frame_count, MANIFEST["frames_per_case"])
                assert_wem_equal(self, reference, bytes(encoded.data), case["name"])
                self.assertEqual(encoded.stats.audio_packets, case["audio_packets"])
                self.assertEqual(encoded.stats.short_packets, case["short_packets"])
                self.assertEqual(encoded.stats.long_packets, case["long_packets"])

                packets = load_wem_parts_bytes(reference)["packets"][1:]
                modes = tuple(packet[0] & 1 for packet in packets)
                self.assertEqual(modes.count(0), case["short_packets"])
                self.assertEqual(modes.count(1), case["long_packets"])


# --------------------------------------------------------------------------
# merged from tests/parity/test_2ch_stress_corpus.py
# --------------------------------------------------------------------------

# Byte-exact expectations for the 2ch/48 kHz stress inputs.

ROOT = Path(__file__).resolve().parents[2]
STRESS_CORPUS = ROOT / "tests" / "data" / "2ch-stress"
STRESS_MANIFEST = json.loads((STRESS_CORPUS / "manifest.json").read_text(encoding="utf-8"))


class TwoChannelStressCorpusTests(unittest.TestCase):
    def test_real_builds_are_byte_exact(self) -> None:
        selection = corpus_selection(STRESS_MANIFEST["profile"])
        profile = resolve_selection(selection)
        self.assertEqual(STRESS_MANIFEST["schema"], "wem.2ch-stress-corpus.v2")

        for case in STRESS_MANIFEST["cases"]:
            with self.subTest(signal=case["name"]):
                input_path = STRESS_CORPUS / case["input"]
                input_bytes = input_path.read_bytes()
                reference = (STRESS_CORPUS / case["reference"]).read_bytes()
                pcm = read_pcm_wav(input_path)
                current = bytes(Encoder(selection).encode_pcm(pcm).data)
                oracle = bytes(
                    encode_pcm_python(
                        profile=profile,
                        container=ContainerPlan.from_profile(profile),
                        pcm=pcm,
                    ).data
                )

                self.assertEqual(input_bytes, render_wav(input_path.stem))
                self.assertEqual(hashlib.sha256(input_bytes).hexdigest(), case["input_sha256"])
                self.assertEqual(len(reference), case["reference_bytes"])
                self.assertEqual(hashlib.sha256(reference).hexdigest(), case["reference_sha256"])
                reference_packets = load_wem_parts_bytes(reference)["packets"][1:]
                current_packets = load_wem_parts_bytes(current)["packets"][1:]
                self.assertEqual(len(reference_packets), case["audio_packets"])
                self.assertEqual(len(current_packets), len(reference_packets))
                modes = tuple(packet[0] & 1 for packet in reference_packets)
                self.assertEqual(modes.count(0), case["short_packets"])
                self.assertEqual(modes.count(1), case["long_packets"])

                assert_wem_equal(self, reference, current, case["name"])
                assert_wem_equal(self, reference, oracle, f"{case['name']}: Python oracle")

                with wave.open(str(input_path), "rb") as source:
                    self.assertEqual(source.getnchannels(), 2)
                    self.assertEqual(source.getsampwidth(), 2)
                    self.assertEqual(source.getframerate(), 48000)
                    interleaved = source.readframes(source.getnframes())
                frame_bytes = profile.channels * 2
                chunk_frames = (7, 13, 997)
                prefix_bytes = sum(chunk_frames) * frame_bytes
                chunks = [frames * frame_bytes for frames in chunk_frames]
                chunks.append(len(interleaved) - prefix_bytes)
                stream = _core.StreamSession.for_selection(selection)
                offset = 0
                for size in chunks:
                    stream.push(interleaved[offset : offset + size])
                    offset += size
                streamed = bytes(stream.finish().bytes)
                assert_wem_equal(self, reference, streamed, f"{case['name']}: chunked stream")


# --------------------------------------------------------------------------
# merged from tests/parity/test_paired_2ch_scalars.py
# --------------------------------------------------------------------------

# Registered 2ch/48k container and seed scalars equal the paired build.
#
# Both groups below were read from the paired build rather than derived, and both
# were previously wrong because the 2ch registration carried the 6ch values:
#
# * Aux fmt fields.  Thirteen 2ch/48k reference streams (different sources,
#   lengths, and both the bank and external-source conversion routes) carry the
#   identical triple ``16080 / 16560 / 0xec69cb18`` with ``0x24 = 0x32 = 0``, so
#   they are per-layout constants, not per-sample data.  The 6ch profile carries
#   its own triple (18180 / 18636 / 0xb3dea448), which the byte-exact 6ch reference
#   already pins.
#
# * ``seed.outer_u32`` rate-dependent scalars.  The serialised array holds two
#   record copies; a live read of the running 2ch/48k build, anchored on the long
#   geometry 5-tuple at ``[7..11]``, gives the values below for both copies.  The
#   pointer-bearing words differ only by load-time relocation and must stay as
#   registered, so only the scalars are asserted here.
#
# With the aux fields pinned, every self-describing fmt field of the 2ch output
# matches the reference; the remaining header differences are the three
# content-derived ones (``dwDataPayloadSize``, ``nAvgBytesPerSec``,
# ``uMaxPacketSize``), which follow the coded audio.

#: The compiled 2ch/48000 profile: every value below is read off the carrier
#: its selection resolves to, never re-typed here.
PROFILE = resolve_selection(WwiseProfile(WwiseVersion.WWISE2013, 2, 48000))

AUX_FIELDS = {
    "dwUnknown_0x24": 0,
    "uUnknown_0x32": 0,
    "dwUnknown_0x34": 16080,
    "dwUnknown_0x38": 16560,
    "dwUnknown_0x3C": 0xEC69CB18,
}

#: index -> value, both record copies (stride 30) share the same lookup group.
OUTER_SCALARS = {
    16: 1067072881,
    17: 664,
    20: 640,
    23: 560,
    37: 0xFFFFFF1E,  # -226 as u32
    38: 5,
    39: 8,
    40: 776,
    41: 48000,
    46: 1067072881,
    47: 664,
    50: 640,
    53: 560,
}


class Paired2chScalarParityTests(unittest.TestCase):
    container_metadata: ClassVar[dict[str, Any]]
    outer_u32: ClassVar[list[int]]

    @classmethod
    def setUpClass(cls) -> None:
        cls.container_metadata = PROFILE.container_metadata.to_fmt_dict()
        cls.outer_u32 = list(PROFILE.table("long_base.seed_outer_u32"))

    def test_aux_container_fields_are_the_paired_layout_constants(self) -> None:
        for field, expected in AUX_FIELDS.items():
            with self.subTest(field=field):
                self.assertEqual(self.container_metadata[field], expected)

    def test_seed_outer_rate_dependent_scalars_come_from_the_read(self) -> None:
        for index, expected in OUTER_SCALARS.items():
            with self.subTest(index=index):
                self.assertEqual(self.outer_u32[index], expected)

    def test_both_record_copies_agree(self) -> None:
        """The array holds two copies at stride 30; the lookup group must match."""
        for index in (16, 17, 20, 23):
            with self.subTest(index=index):
                self.assertEqual(self.outer_u32[index], self.outer_u32[index + 30])


if __name__ == "__main__":
    unittest.main()
