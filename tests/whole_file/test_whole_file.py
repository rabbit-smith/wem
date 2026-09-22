import unittest
from pathlib import Path

from wwise_wem import encode


FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"


class WholeFileEncoderTests(unittest.TestCase):
    def test_complete_wem_is_bit_exact(self):
        # The whole-file claim is the byte comparison against the committed
        # reference WEM (the paired build's output for this input).
        result = encode(FIXTURES / "input.wav")
        reference = (FIXTURES / "reference.wem").read_bytes()
        self.assertEqual(result.data, reference)
        stats = result.stats
        self.assertEqual(stats.pcm_frames, 139398)
        self.assertEqual(stats.channels, 6)
        self.assertEqual(stats.audio_packets, 205)
        self.assertEqual(stats.short_packets, 77)
        self.assertEqual(stats.long_packets, 128)
        # The container length is the caller's own count of the bytes it
        # holds; the library returns it nowhere.
        self.assertEqual(len(result), len(reference))
        self.assertEqual(len(result), 108771)


if __name__ == "__main__":
    unittest.main()
