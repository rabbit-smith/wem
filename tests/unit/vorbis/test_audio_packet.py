"""The audio packet: its residue rows, its packing, and its parser.

One object, four angles. The residue cases pin classification,
quantization and the two packing paths; the block cases pin the assembled
packet and its quantized rows; the frame cases pin the entry that ties a
psy frame to a packet; the parser cases read the same packet back —
header, floor body, residue status — so the writer and the reader of one
syntax sit together. Each class keeps its own test names and failure
messages.
"""

from __future__ import annotations

import math
import unittest
import wwise_wem_reference.vorbis.packet_decoder as parser_module
import wwise_wem_reference.vorbis.packet_encoder as packet_encoder

from tests.codebook_resource_support import installed_codebook_tables
from unittest.mock import patch
from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem_reference.analysis.model import PsyFrame, SpectrumFrame
from wwise_wem_reference.analysis.preprocessing.windowing import WindowedFrame
from wwise_wem_reference.profiles.artifact import resolve_selection
from wwise_wem_reference.profiles.assembly import assemble_encoder_profile_resources
from wwise_wem_reference.profiles.codebooks import load_setup_codebooks
from wwise_wem_reference.scheduling.model import FramePlan
from wwise_wem_reference.vorbis.bitio import BitReader, OggPack
from wwise_wem_reference.vorbis.codebook import (
    Codebook,
    StaticCodebook,
    codebook_from_static,
)
from wwise_wem_reference.vorbis.floor_fit import floor1_fit_simple
from wwise_wem_reference.vorbis.packet_decoder import (
    decode_floor1_body,
    extract_packets,
    parse_audio_header,
    parse_audio_packet,
)
from wwise_wem_reference.vorbis.packet_encoder import (
    BlockPacketResult,
    EncodedPacket,
    pack_analysis_frame,
)
from wwise_wem_reference.vorbis.residue import (
    classify_partition,
    consume_residue,
    mdct_to_residue,
    pack_residue_silent,
    pack_residue_vq,
    partitions_to_read,
    quantize_residue_value,
)
from wwise_wem_reference.vorbis.setup import parse_setup

# --------------------------------------------------------------------------
# merged from tests/unit/vorbis/test_residue_runtime.py
# --------------------------------------------------------------------------

# Pure runtime regressions for residue classification, quantization, and VQ.

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


# --------------------------------------------------------------------------
# merged from tests/unit/vorbis/test_packet_runtime.py
# --------------------------------------------------------------------------

class PacketRuntimeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        profile = resolve_selection(
            WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
        )
        cls.setup = parse_setup(
            profile.setup_packet, channels=6
        )
        cls.books = load_setup_codebooks(
            cls.setup["book_ids"], installed_codebook_tables()
        )
        stereo = resolve_selection(
            WwiseProfile(WwiseVersion.WWISE2013, 2, 48000)
        )
        cls.stereo_setup = parse_setup(stereo.setup_packet, channels=2)
        cls.stereo_books = assemble_encoder_profile_resources(
            stereo,
            setup_packet=stereo.setup_packet,
            quality=stereo.quality,
        )
        cls.stereo_books = cls.stereo_books.codebooks

    def test_mode_only_silence_headers_remain_exact(self):
        self.assertEqual(packet_encoder.pack_silence_packet(self.setup, 6, 0), b"\x00")
        self.assertEqual(packet_encoder.pack_silence_packet(self.setup, 6, 1), b"\x01")

    def test_deterministic_floor_residue_packet_and_quantized_values(self):
        n = 128
        mdct = []
        posts = []
        for channel in range(6):
            row = [
                0.05
                * math.sin(0.3 * index + channel)
                * (1 if 10 < index < 100 else 0.1)
                for index in range(n)
            ]
            mdct.append(row)
            posts.append(
                floor1_fit_simple(
                    [abs(value) for value in row], self.setup["floors"][0], n
                )
            )
        result = packet_encoder.pack_block_packet_details(
            self.setup, self.books, 6, 0, posts, mdct
        )
        packet = result.packet
        self.assertEqual(len(packet), 331)
        q_after = result.quantized_residue
        self.assertEqual((len(q_after), len(q_after[0])), (6, 128))
        # The packed words and the quantized-residue rows of this synthetic
        # state are asserted cross-implementation in
        # crates/wem-core/tests/packet_parity.rs (recorded from this oracle
        # stage by scripts/record_packet_parity.py).

    def test_type2_coupling_propagates_one_sided_floor_use(self):
        n = 128
        active = [2.0 if index % 2 == 0 else -1.5 for index in range(n)]
        posts = floor1_fit_simple(
            [abs(value) for value in active], self.stereo_setup["floors"][0], n
        )

        for floor_posts in ([posts, None], [None, posts]):
            with self.subTest(floor_posts=[value is not None for value in floor_posts]):
                result = packet_encoder.pack_block_packet_details(
                    self.stereo_setup,
                    self.stereo_books,
                    2,
                    0,
                    floor_posts,
                    [active, active],
                )
                self.assertTrue(any(result.quantized_residue[0]))
                self.assertTrue(any(result.quantized_residue[1]))


# --------------------------------------------------------------------------
# merged from tests/unit/vorbis/test_audio_packet.py
# --------------------------------------------------------------------------

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
        ((0.0,),),
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
        self.assertEqual(pack.call_args.kwargs["coupling_peak"], ((0.0,),))

    def test_channel_count_is_checked_before_floor_fitting(self):
        window = _window(0, 0, 0, 0, 128)
        analysis = _analysis(window, raw=(0.0,), post=(0.0,), side=(0.0,))
        with self.assertRaisesRegex(ValueError, "channel count"):
            pack_analysis_frame({}, [], analysis, channels=2)


# --------------------------------------------------------------------------
# merged from tests/unit/vorbis/test_audio_packet_parser.py
# --------------------------------------------------------------------------

# Pure packet-header, floor, and residue parser regressions.

def _setup(*, channels: int = 2) -> dict:
    floor = {
        "multiplier": 1,
        "x_list": [],
        "partitions": 0,
        "partition_classes": [],
        "class_dims": [],
        "class_subs": [],
        "class_masterbooks": [],
        "subclass_books": [],
    }
    residue = {
        "begin": 0,
        "end": 0,
        "partition_size": 1,
        "classbook": 0,
        "classifications": 1,
        "cascades": [0],
        "books": [[-1] * 8],
    }
    return {
        "nmodes": 2,
        "modes": [
            {"blockflag": 0, "mapping": 0},
            {"blockflag": 1, "mapping": 0},
        ],
        "maps": [
            {
                "submaps": 1,
                "floors": [0],
                "residues": [0],
                "chmux": [0] * channels,
            }
        ],
        "floors": [floor],
        "residues": [residue],
    }


class PacketExtractionTests(unittest.TestCase):
    def test_extracts_size_prefixed_packets(self):
        data = b"\x02\x00ab\x01\x00z"
        self.assertEqual(extract_packets(data), [b"ab", b"z"])
        self.assertEqual(extract_packets(b"skip" + data, 4), [b"ab", b"z"])

    def test_rejects_truncated_packet_and_trailing_byte(self):
        with self.assertRaisesRegex(ValueError, "truncated packet"):
            extract_packets(b"\x03\x00ab")
        with self.assertRaisesRegex(ValueError, "trailing bytes"):
            extract_packets(b"\x01\x00ab")


class AudioHeaderTests(unittest.TestCase):
    def test_long_header_serializes_only_mode(self):
        br = BitReader(b"\x01")
        header = parse_audio_header(br, _setup())
        self.assertEqual(header["mode"], 1)
        self.assertEqual(header["blockflag"], 1)
        self.assertEqual(header["bit_end"], 1)
        self.assertIsNone(header["prev_window"])
        self.assertIsNone(header["next_window"])

    def test_rejects_mode_outside_setup(self):
        setup = _setup()
        setup["nmodes"] = 3
        setup["modes"].append({"blockflag": 0, "mapping": 0})
        with self.assertRaisesRegex(ValueError, "out of range"):
            parse_audio_header(BitReader(b"\x03"), setup)


class FloorAndResidueTests(unittest.TestCase):
    def test_minimal_floor_body_reads_two_y_posts(self):
        op = OggPack()
        op.write(17, 8)
        op.write(99, 8)
        floor = _setup(channels=1)["floors"][0]
        self.assertEqual(
            decode_floor1_body(BitReader(op.get_buffer()), floor, []),
            [17, 99],
        )

    def test_one_byte_long_zero_floor_packet_is_complete(self):
        parsed = parse_audio_packet(b"\x01", _setup(), [], 2)
        self.assertTrue(parsed["complete"])
        self.assertEqual(parsed["header"]["mode"], 1)
        self.assertEqual(parsed["floors"]["nonzero"], [0, 0])
        self.assertEqual(parsed["residues"][0]["status"], "empty")

    def test_nonzero_floor_body_reaches_empty_residue(self):
        op = OggPack()
        op.write(0, 1)
        op.write(1, 1)
        op.write(17, 8)
        op.write(99, 8)
        parsed = parse_audio_packet(op.get_buffer(), _setup(channels=1), [], 1)
        self.assertTrue(parsed["complete"])
        self.assertEqual(parsed["floors"]["nonzero"], [1])
        self.assertEqual(parsed["floors"]["y0_head"], [17, 99])
        self.assertEqual(parsed["residues"][0]["status"], "empty")

    def test_reports_header_and_floor_eof(self):
        header_eof = parse_audio_packet(b"", _setup(), [], 2)
        self.assertFalse(header_eof["complete"])
        self.assertRegex(header_eof["error"], "header")

        op = OggPack()
        op.write(0, 1)
        op.write(1, 1)
        floor_eof = parse_audio_packet(
            op.get_buffer(), _setup(channels=1), [], 1
        )
        self.assertFalse(floor_eof["complete"])
        self.assertRegex(floor_eof["error"], "floor body")

    def test_module_has_no_file_analyzer_or_hidden_cli(self):
        for name in ("analyze_wem_audio", "self_test", "main"):
            self.assertFalse(hasattr(parser_module, name), name)


if __name__ == "__main__":
    unittest.main()
