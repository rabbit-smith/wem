import hashlib
import unittest
from pathlib import Path

from wwise_wem import encode_wav_to_wem


FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"
EXPECTED_SHA256 = "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247"
EXPECTED_STATS = {
    "pcm_frames": 139398,
    "channels": 6,
    "audio_packets": 205,
    "short_packets": 77,
    "long_packets": 128,
    "bytes": 108771,
    "metadata_source": "profile:wwise2013-6ch-44100",
}


class GoldenEncoderTests(unittest.TestCase):
    def test_complete_wem_is_bit_exact(self):
        encoded, stats = encode_wav_to_wem(FIXTURES / "input.wav")
        reference = (FIXTURES / "reference.wem").read_bytes()
        self.assertEqual(encoded, reference)
        self.assertEqual(hashlib.sha256(encoded).hexdigest(), EXPECTED_SHA256)
        self.assertEqual(stats, EXPECTED_STATS)


if __name__ == "__main__":
    unittest.main()
