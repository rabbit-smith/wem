import tempfile
import unittest
import wave
from pathlib import Path

from wwise_wem import encode_wav_to_wem


FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"


class TemplateCompatibilityTests(unittest.TestCase):
    def test_template_and_profile_match_on_short_input(self):
        with wave.open(str(FIXTURES / "input.wav"), "rb") as source:
            params = source.getparams()
            pcm = source.readframes(4096)
        with tempfile.TemporaryDirectory() as directory:
            short_wav = Path(directory) / "short.wav"
            with wave.open(str(short_wav), "wb") as target:
                target.setparams(params)
                target.writeframes(pcm)
            profiled, _ = encode_wav_to_wem(short_wav)
            templated, _ = encode_wav_to_wem(
                short_wav, FIXTURES / "reference.wem"
            )
        self.assertEqual(profiled, templated)


if __name__ == "__main__":
    unittest.main()
