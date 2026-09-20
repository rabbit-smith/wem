from __future__ import annotations

import unittest
from dataclasses import FrozenInstanceError

from wwise_wem_reference.analysis.model import PsyFrame, SpectrumFrame
from wwise_wem_reference.scheduling.model import FramePlan
from wwise_wem_reference.analysis.preprocessing.windowing import WindowedFrame


class FrameModelTests(unittest.TestCase):
    def test_psy_frame_exposes_its_nested_spectrum_without_copying(self):
        plan = FramePlan(2, 0, 1, 0, 0, 2048, 2048, 1)
        window = WindowedFrame(plan, 960, ((0.0,) * 2048,))
        spectrum = SpectrumFrame(
            window,
            ((1.0,),),
            ((2.0,),),
            ((3.0,),),
            (-4.0,),
            -4.0,
        )
        frame = PsyFrame(
            spectrum,
            ((5.0,),),
            ((6.0,),),
            ((7.0,),),
            ((8.0,),),
            ((9.0,),),
        )

        self.assertIs(frame.spectrum, spectrum)
        self.assertIs(frame.window, window)
        self.assertIs(frame.coefficients, spectrum.coefficients)
        self.assertIs(frame.raw_mdct, spectrum.raw_mdct)
        self.assertIs(frame.fft, spectrum.fft)
        self.assertIs(frame.channel_specmax, spectrum.channel_specmax)
        self.assertEqual(frame.global_specmax, -4.0)
        with self.assertRaises(FrozenInstanceError):
            frame.side = ()  # type: ignore[misc]


if __name__ == "__main__":
    unittest.main()
