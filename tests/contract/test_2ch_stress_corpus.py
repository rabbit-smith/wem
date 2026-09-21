"""Byte-exact expectations for the 2ch/48 kHz stress inputs."""

from __future__ import annotations

import hashlib
import json
import unittest
import wave
from pathlib import Path

from tests.contract.wem_byte_contract import assert_wem_equal
from tests.two_channel_corpus_support import corpus_selection, render_wav
from wwise_wem import _core
from wwise_wem.application.encoder import Encoder
from wwise_wem.profiles.registry import resolve_selection
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem_reference.container.wem import load_wem_parts_bytes
from wwise_wem_reference.python_engine import ContainerPlan, encode_pcm_python


ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / "tests" / "data" / "2ch-stress"
MANIFEST = json.loads((CORPUS / "manifest.json").read_text(encoding="utf-8"))


class TwoChannelStressCorpusTests(unittest.TestCase):
    def test_real_builds_are_byte_exact(self) -> None:
        selection = corpus_selection(MANIFEST["profile"])
        profile = resolve_selection(selection)
        self.assertEqual(MANIFEST["schema"], "wem.2ch-stress-corpus.v2")

        for case in MANIFEST["cases"]:
            with self.subTest(signal=case["name"]):
                input_path = CORPUS / case["input"]
                input_bytes = input_path.read_bytes()
                reference = (CORPUS / case["reference"]).read_bytes()
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


if __name__ == "__main__":
    unittest.main()
