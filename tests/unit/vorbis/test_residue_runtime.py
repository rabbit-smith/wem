"""Pure runtime regressions for residue classification, quantization, and VQ."""
from __future__ import annotations

import math
import unittest

from wwise_wem.vorbis.bitio import BitReader
from wwise_wem.vorbis.codebook import Codebook, codebook_from_static
from wwise_wem.vorbis.bitio import OggPack
from wwise_wem.vorbis.residue import (
    classify_partition,
    consume_residue,
    mdct_to_residue,
    pack_residue_silent,
    pack_residue_vq,
    partitions_to_read,
    quantize_residue_value,
)
from wwise_wem.vorbis.codebook import StaticCodebook


class ClassifierAndQuantizerTests(unittest.TestCase):
    def test_quantizer_uses_nearest_even(self):
        self.assertEqual(
            [quantize_residue_value(value) for value in (0.5, 1.5, 2.5, -0.5, -1.5, -2.5)],
            [0, 2, 2, 0, -2, -2],
        )

    def test_recovered_classifier_boundaries(self):
        cases = (
            ([0.0] * 16, 0),
            ([1.0] * 16, 2),
            ([2.0] * 16, 4),
            ([3.0] + [0.0] * 15, 5),
            ([29.0] + [0.0] * 15, 7),
        )
        for samples, expected in cases:
            with self.subTest(expected=expected):
                self.assertEqual(classify_partition(samples), expected)

    def test_classifier_quantizes_before_measuring(self):
        self.assertEqual(classify_partition([0.49] * 16), 0)
        self.assertEqual(classify_partition([0.51] * 16), 2)

    def test_nonstandard_class_count_stays_bounded(self):
        for samples in ([], [0.0] * 8, [0.01] * 8, [100.0] * 8):
            result = classify_partition(samples, nclass=5)
            self.assertGreaterEqual(result, 0)
            self.assertLess(result, 5)


class ResidueRuntimeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        phrasebook = codebook_from_static(
            StaticCodebook(dim=2, entries=64, lengthlist=[6] * 64)
        )
        stage_static = StaticCodebook(
            dim=2, entries=2, lengthlist=[1, 1], maptype=1
        )
        stagebook = Codebook(
            static=stage_static,
            codelist=[0, 1],
            _tree={0: 0, 1: 1},
            valuallist=[[0.0, 0.0], [1.0, 1.0]],
            quantvals=1,
        )
        cls.books = [phrasebook, stagebook]
        cls.residue = {
            "begin": 0,
            "end": 16,
            "partition_size": 4,
            "classbook": 0,
            "classifications": 8,
            "cascades": [0] + [1] * 7,
            "books": [[-1] * 8]
            + [[1] + [-1] * 7 for _ in range(7)],
        }

    def test_partition_count_clips_to_spectrum(self):
        self.assertEqual(partitions_to_read(self.residue, 16), 4)
        self.assertEqual(partitions_to_read(self.residue, 8), 2)

    def test_vq_pack_and_consume_smoke(self):
        residuals = []
        for channel in range(6):
            row = [0.0] * 16
            for index in range(4, 12):
                row[index] = 3.0 * math.sin(index * 0.4 + channel)
            residuals.append(row)

        op = OggPack(4096)
        stats = pack_residue_vq(
            op,
            self.residue,
            self.books,
            residuals,
            [True] * 6,
            n_spectrum=16,
        )
        self.assertEqual(stats["status"], "ok")
        self.assertGreater(stats["vq_count"], 0)
        self.assertGreater(len(op.get_buffer()), 10)

        decoded = consume_residue(
            BitReader(op.get_buffer()),
            self.residue,
            self.books,
            [True] * 6,
            n_spectrum=16,
        )
        self.assertIn(decoded["status"], ("complete", "eop"))

    def test_silent_pack_is_completely_consumed(self):
        op = OggPack(256)
        pack_residue_silent(
            op, self.residue, self.books, 6, n_spectrum=16
        )
        decoded = consume_residue(
            BitReader(op.get_buffer()),
            self.residue,
            self.books,
            [True] * 6,
            n_spectrum=16,
        )
        self.assertEqual(decoded["status"], "complete")

    def test_stage_book_selects_a_valid_zero_vector(self):
        book = self.books[1]
        entry = book.best_vq([0.0] * book.dim)
        self.assertGreater(book.lengthlist[entry], 0)

    def test_mdct_to_residue_preserves_tail_and_zero_floor(self):
        self.assertEqual(
            mdct_to_residue([2.0, 3.0, 4.0], [0.5, 0.0]),
            [4.0, 0.0, 4.0],
        )

if __name__ == "__main__":
    unittest.main()
