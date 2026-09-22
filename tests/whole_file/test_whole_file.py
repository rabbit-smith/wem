import unittest
from pathlib import Path

from wwise_wem import encode


FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"


class WholeFileEncoderTests(unittest.TestCase):
    def test_complete_wem_is_bit_exact(self):
        # The whole-file claim is the byte comparison against the committed
        # reference WEM; that file's SHA-256 is also recorded in
        # tests/data/stage-records/stages/index.json.
        result = encode(FIXTURES / "input.wav")
        reference = (FIXTURES / "reference.wem").read_bytes()
        self.assertEqual(result.data, reference)
        stats = result.stats
        self.assertEqual(stats.pcm_frames, 139398)
        self.assertEqual(stats.channels, 6)
        self.assertEqual(stats.audio_packets, 205)
        self.assertEqual(stats.short_packets, 77)
        self.assertEqual(stats.long_packets, 128)
        self.assertEqual(stats.bytes, 108771)
        self.assertEqual(stats.metadata_source, "profile:6ch/44100Hz/2013")


if __name__ == "__main__":
    unittest.main()
