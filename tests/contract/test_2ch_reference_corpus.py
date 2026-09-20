"""Byte contracts for representative 2ch/48 kHz Wwise outputs."""

from __future__ import annotations

import hashlib
import json
import unittest
from pathlib import Path

from tests.contract.wem_byte_contract import assert_wem_equal
from tests.two_channel_corpus_support import render_wav
from wwise_wem import Encoder, load_wem_profile
from wwise_wem.adapters.wav import read_pcm16
from wwise_wem_reference.container.wem import load_wem_parts_bytes


ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / "tests" / "data" / "2ch-reference"
MANIFEST = json.loads((CORPUS / "manifest.json").read_text(encoding="utf-8"))


class TwoChannelReferenceCorpusTests(unittest.TestCase):
    def test_representative_inputs_match_paired_build_byte_for_byte(self) -> None:
        profile = load_wem_profile(MANIFEST["profile"])
        self.assertEqual(MANIFEST["schema"], "wem.2ch-reference-corpus.v1")

        for case in MANIFEST["cases"]:
            with self.subTest(signal=case["name"]):
                input_path = CORPUS / case["input"]
                input_bytes = input_path.read_bytes()
                reference = (CORPUS / case["reference"]).read_bytes()
                pcm = read_pcm16(input_path)
                encoded = Encoder(profile).encode_pcm(pcm)

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


if __name__ == "__main__":
    unittest.main()
