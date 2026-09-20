from __future__ import annotations

import unittest

from wwise_wem_reference.analysis.config import InputConditionerConfig
from wwise_wem_reference.analysis.preprocessing.conditioner import InputConditioner


class InputConditionerTests(unittest.TestCase):
    def test_recorded_stereo_prefix_and_chunking_are_exact(self) -> None:
        config = InputConditionerConfig.from_bits(0x3F7F546D)
        rows = (
            (0.3812255859375, 0.352569580078125, 0.3597412109375, 0.3544921875),
            (0.080780029296875, 0.091217041015625, 0.089935302734375, 0.0841064453125),
        )
        expected = (
            (0.3812255859375, 0.3515625, 0.357818603515625, 0.35162353515625),
            (0.080780029296875, 0.09100341796875, 0.0894775390625, 0.08343505859375),
        )

        whole = InputConditioner(2, config).process(rows)
        self.assertEqual(whole, expected)

        chunked = InputConditioner(2, config)
        first = chunked.process(tuple(row[:1] for row in rows))
        rest = chunked.process(tuple(row[1:] for row in rows))
        joined = tuple(a + b for a, b in zip(first, rest))
        self.assertEqual(joined, expected)


if __name__ == "__main__":
    unittest.main()
