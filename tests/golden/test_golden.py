import hashlib
import unittest
from pathlib import Path

from wwise_wem import encode_wav


FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"
EXPECTED_SHA256 = "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247"


class GoldenEncoderTests(unittest.TestCase):
    def test_complete_wem_is_bit_exact(self):
        result = encode_wav(FIXTURES / "input.wav")
        reference = (FIXTURES / "reference.wem").read_bytes()
        self.assertEqual(result.data, reference)
        self.assertEqual(hashlib.sha256(result.data).hexdigest(), EXPECTED_SHA256)
        stats = result.stats
        self.assertEqual(stats.pcm_frames, 139398)
        self.assertEqual(stats.channels, 6)
        self.assertEqual(stats.audio_packets, 205)
        self.assertEqual(stats.short_packets, 77)
        self.assertEqual(stats.long_packets, 128)
        self.assertEqual(stats.bytes, 108771)
        self.assertEqual(stats.metadata_source, "profile:wwise2013-6ch-44100")


if __name__ == "__main__":
    unittest.main()
