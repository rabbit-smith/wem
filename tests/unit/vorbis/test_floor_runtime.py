from __future__ import annotations

import unittest
from pathlib import Path

from wwise_wem_reference.vorbis.floor import (
    FLOOR1_RANGES,
    floor1_curve_from_posts,
    floor1_neighbor_tables,
    floor1_render_line,
    floor1_unwrap,
    floor1_wrap,
    floor1_wrap_with_posts,
    postlist_from_floor,
    render_point,
)
from wwise_wem_reference.vorbis.floor_fit import (
    FLOOR1_WWISE_FIT_PARAMS,
    _DB_QUANT_SCALE,
    _inspect_error,
    _propagate_high_neighbor,
    _wwise_db_quant,
    floor1_fit_simple,
    floor1_fit_wwise,
    floor1_quantize_posts,
)
from wwise_wem_reference.vorbis.setup import parse_setup
from wwise_wem_reference.container.fmt import parse_vorbis_fmt
from wwise_wem_reference.container.packets import extract_packets
from wwise_wem_reference.container.riff import parse_chunks


FIXTURES = Path(__file__).resolve().parents[2] / "fixtures"


def _reference_floors() -> list[dict]:
    raw = (FIXTURES / "reference.wem").read_bytes()
    endian, chunks = parse_chunks(raw)
    fmt_chunk = next(chunk for chunk in chunks if chunk["id"] == "fmt ")
    data_chunk = next(chunk for chunk in chunks if chunk["id"] == "data")
    fmt = parse_vorbis_fmt(fmt_chunk["payload"])
    packets = extract_packets(
        data_chunk["payload"], fmt["dwSeekTableSize"], endian=endian
    )["packets"]
    return parse_setup(packets[0], channels=6)["floors"]


class FloorRuntimeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.short_floor, cls.long_floor = _reference_floors()

    def test_postlist_neighbors_and_render_point(self):
        postlist = postlist_from_floor(self.short_floor)
        self.assertEqual(
            postlist,
            [0, 128, 8, 33, 4, 16, 70, 2, 6, 12, 23, 46, 90],
        )
        self.assertEqual(
            floor1_neighbor_tables(postlist),
            (
                [0, 2, 0, 2, 3, 0, 4, 2, 5, 3, 6],
                [1, 1, 2, 3, 1, 4, 2, 5, 3, 6, 1],
            ),
        )
        self.assertEqual(render_point(0, 10, 0, 100, 5), 50)
        self.assertEqual(render_point(0, 10, 100, 0, 5), 50)
        self.assertEqual(render_point(0, 4, 0, 3, 2), 1)
        self.assertEqual(render_point(3, 3, 0x8007, 99, 3), 7)

    def test_residual_wrap_unwrap_roundtrip_and_flags(self):
        postlist = postlist_from_floor(self.short_floor)
        range_ = FLOOR1_RANGES[self.short_floor["multiplier"]]
        residuals = [64, 42, 1, 0, 2, 0, 0, 3, 0, 0, 0, 0, 0]
        posts = floor1_unwrap(residuals, postlist, range_)
        self.assertEqual(
            posts,
            [64, 42, 62, 32826, 64, 32829, 32820, 62, 32831, 32830, 32828, 32824, 32817],
        )
        self.assertEqual(floor1_wrap(posts, postlist, range_), residuals)
        normalized, wrapped = floor1_wrap_with_posts(posts, postlist, range_)
        self.assertEqual(wrapped, residuals)
        self.assertEqual(
            [value & 0x7FFF for value in normalized],
            [value & 0x7FFF for value in posts],
        )

        long_postlist = postlist_from_floor(self.long_floor)
        long_range = FLOOR1_RANGES[self.long_floor["multiplier"]]
        long_residuals = [long_range // 2, long_range // 3] + [0] * (
            len(long_postlist) - 2
        )
        self.assertEqual(
            floor1_wrap(
                floor1_unwrap(long_residuals, long_postlist, long_range),
                long_postlist,
                long_range,
            ),
            long_residuals,
        )

    def test_render_curve_and_exclusive_line_end(self):
        line = [0.0] * 32
        floor1_render_line(0, 16, 0, 80, line)
        self.assertEqual(line[:17], [float(value) for value in range(0, 80, 5)] + [0.0])

        postlist = postlist_from_floor(self.short_floor)
        posts = [64, 42, 62, 32826, 64, 32829, 32820, 62, 32831, 32830, 32828, 32824, 32817]
        curve = floor1_curve_from_posts(
            posts, postlist, 128, self.short_floor["multiplier"]
        )
        self.assertEqual(len(curve), 128)
        self.assertTrue(all(value > 0.0 for value in curve))

    def test_simple_and_wwise_fit_paths(self):
        floor = self.short_floor
        postlist = postlist_from_floor(floor)
        range_ = FLOOR1_RANGES[floor["multiplier"]]
        self.assertIsNone(floor1_fit_simple([0.0] * 128, floor, 128))
        mags = [1e-6] * 128
        mags[10], mags[20] = 0.5, 0.3
        simple = floor1_fit_simple(mags, floor, 128)
        self.assertIsNotNone(simple)
        self.assertEqual(len(simple), len(postlist))
        self.assertTrue(all(0 <= value < range_ for value in simple))

        self.assertIsNone(floor1_fit_wwise([-140.0] * 128, [-140.0] * 128, floor))
        fitted = floor1_fit_wwise([-40.0] * 128, [-30.0] * 128, floor)
        self.assertEqual(
            fitted,
            [730, 730] + [730 | 0x8000] * (len(postlist) - 2),
        )
        self.assertIsNone(floor1_fit_wwise([-10.0] * 128, [-40.0] * 128, floor))
        quantized = floor1_quantize_posts(fitted, floor["multiplier"])
        wrapped = floor1_wrap(quantized, postlist, range_)
        self.assertTrue(all(0 <= value < range_ for value in wrapped))

    def test_integer_mse_and_high_neighbor_edges(self):
        one_quant_db = (1.0 - 1023.5) / _DB_QUANT_SCALE + 0.01
        self.assertEqual(_wwise_db_quant(one_quant_db), 1)
        self.assertFalse(
            _inspect_error(
                0,
                4,
                0,
                0,
                [-140.0, one_quant_db, -140.0, -140.0],
                [-140.0] * 4,
                {**FLOOR1_WWISE_FIT_PARAMS, "maxerr": 0.0},
            )
        )
        local_hi = [1, 1, 1, 2]
        _propagate_high_neighbor(local_hi, 2, 1, 7)
        self.assertEqual(local_hi, [7, 7, 1, 2])

    def test_quantize_multiplier_validation(self):
        self.assertEqual(floor1_quantize_posts([1023, 0x8000 | 511], 1), [255, 0x8000 | 127])
        self.assertEqual(floor1_quantize_posts([1023], 2), [127])
        self.assertEqual(floor1_quantize_posts([1023], 3), [85])
        self.assertEqual(floor1_quantize_posts([1023], 4), [63])
        with self.assertRaisesRegex(ValueError, "invalid floor1 multiplier"):
            floor1_quantize_posts([1], 0)


if __name__ == "__main__":
    unittest.main()
