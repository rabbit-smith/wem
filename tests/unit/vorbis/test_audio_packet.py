from __future__ import annotations

import unittest
from unittest.mock import patch

from wwise_wem_reference.vorbis.packet_encoder import EncodedPacket, pack_analysis_frame
from wwise_wem_reference.vorbis.packet_encoder import BlockPacketResult
from wwise_wem_reference.analysis.model import PsyFrame, SpectrumFrame
from wwise_wem_reference.scheduling.model import FramePlan
from wwise_wem_reference.analysis.preprocessing.windowing import WindowedFrame


def _window(index: int, previous: int, current: int, following: int, center: int):
    size = (256, 2048)[current]
    return WindowedFrame(
        FramePlan(index, previous, current, following, 0, size, size, 1),
        center,
        ((0.0,) * size,),
    )


def _analysis(window, *, raw=(2.0,), post=(1.0,), side=(3.0,)) -> PsyFrame:
    return PsyFrame(
        SpectrumFrame(
            window,
            ((0.0,),),
            (tuple(raw),),
            ((0.0,),),
            (-1.0,),
            -1.0,
        ),
        ((0.0,),),
        ((0.0,),),
        (tuple(post),),
        (tuple(side),),
    )


class AudioPacketTests(unittest.TestCase):
    def test_pack_analysis_frame_preserves_floor_and_residue_details(self):
        window = _window(3, 0, 0, 1, 512)
        analysis = _analysis(window)
        setup = {
            "modes": [{"mapping": 0}],
            "maps": [{"submaps": 1, "chmux": [0], "floors": [0]}],
            "floors": [{"multiplier": 2, "rangebits": 7, "x_list": []}],
        }
        with (
            patch(
                "wwise_wem_reference.vorbis.packet_encoder.floor1_fit_wwise",
                return_value=[10],
            ) as fit,
            patch(
                "wwise_wem_reference.vorbis.packet_encoder.pack_block_packet_details",
                return_value=BlockPacketResult(b"packet", ((4,),)),
            ) as pack,
        ):
            encoded = pack_analysis_frame(setup, [], analysis, channels=1)

        self.assertEqual(
            encoded,
            EncodedPacket(analysis, ((10,),), b"packet", ((4,),)),
        )
        fit.assert_called_once_with((1.0,), (2.0,), setup["floors"][0], n=1)
        self.assertEqual(pack.call_args.kwargs["posts_are_10bit"], True)

    def test_channel_contract_is_checked_before_floor_fitting(self):
        window = _window(0, 0, 0, 0, 128)
        analysis = _analysis(window, raw=(0.0,), post=(0.0,), side=(0.0,))
        with self.assertRaisesRegex(ValueError, "channel count"):
            pack_analysis_frame({}, [], analysis, channels=2)


if __name__ == "__main__":
    unittest.main()
