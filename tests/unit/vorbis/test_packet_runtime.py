import hashlib
import math
import unittest

import wwise_wem_reference.vorbis.packet_encoder as packet_encoder
from wwise_wem_reference.profiles.codebooks import load_setup_codebooks
from wwise_wem_reference.profiles.assembly import assemble_encoder_profile_resources
from tests.codebook_resource_support import installed_codebook_tables
from wwise_wem_reference.vorbis.floor_fit import floor1_fit_simple
from wwise_wem_reference.vorbis.setup import parse_setup
from wwise_wem.profiles.registry import resolve_wem_profile
from wwise_wem.profiles.bundle import load_profile_bundle


class PacketRuntimeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        profile = resolve_wem_profile(6, 44100)
        cls.setup = parse_setup(
            profile.setup_packet(), channels=6
        )
        cls.books = load_setup_codebooks(
            cls.setup["book_ids"], installed_codebook_tables()
        )
        stereo = resolve_wem_profile(2, 48000)
        cls.stereo_setup = parse_setup(stereo.setup_packet(), channels=2)
        cls.stereo_books = assemble_encoder_profile_resources(
            load_profile_bundle(profile=stereo.name, verify_all=False),
            setup_packet=stereo.setup_packet(),
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
        self.assertEqual(
            hashlib.sha256(packet).hexdigest(),
            "c944326ea93b1004577243073e7cd087a886f3c7dd874662e290abd447e6fab0",
        )
        q_after = result.quantized_residue
        self.assertEqual((len(q_after), len(q_after[0])), (6, 128))
        q_bytes = b"".join(
            int(value).to_bytes(4, "little", signed=True)
            for row in q_after
            for value in row
        )
        self.assertEqual(
            hashlib.sha256(q_bytes).hexdigest(),
            "7af3033268a641f6078352a0a3d25460fd8c50400ed4b852c845044ac19f19bc",
        )

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

if __name__ == "__main__":
    unittest.main()
