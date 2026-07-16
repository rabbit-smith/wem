from __future__ import annotations

import random
import unittest

from wwise_wem.vorbis.bitio import BitReader, OggPack


def _reference_pack(writes: list[tuple[int, int]]) -> bytes:
    value = 0
    offset = 0
    for item, width in writes:
        mask = (1 << width) - 1 if width else 0
        value |= (item & mask) << offset
        offset += width
    return value.to_bytes((offset + 7) // 8, "little")


class BitIoContractTests(unittest.TestCase):
    def test_every_supported_width_roundtrips_and_masks(self) -> None:
        for width in range(33):
            with self.subTest(width=width):
                value = -(1 << 40) + 0xFEDCBA9876
                packed = OggPack()
                packed.write(value, width)
                expected = value & ((1 << width) - 1) if width else 0
                self.assertEqual(packed.get_buffer(), _reference_pack([(value, width)]))
                reader = BitReader(packed.get_buffer())
                self.assertEqual(reader.read(width), expected)
                self.assertEqual(reader.tell_bits(), width)
                self.assertEqual(reader.bits_left(), (-width) % 8)

        for width in (-1, 33):
            with self.subTest(invalid_width=width):
                with self.assertRaisesRegex(ValueError, "0..32"):
                    OggPack().write(0, width)

    def test_lsb_first_cross_byte_anchor_vector(self) -> None:
        writes = [
            (0b101, 3),
            (0b101010101, 9),
            (0xAB, 8),
            (0b11111, 5),
            (0xDEADBEEF, 32),
        ]
        packed = OggPack()
        for value, width in writes:
            packed.write(value, width)
        expected = _reference_pack(writes)
        self.assertEqual(packed.get_buffer(), expected)
        self.assertEqual(expected[:4], bytes.fromhex("adba fa df"))

        reader = BitReader(expected)
        self.assertEqual(
            [reader.read(width) for _, width in writes],
            [value & ((1 << width) - 1) for value, width in writes],
        )

    def test_growth_and_reset_reuse_storage_deterministically(self) -> None:
        packed = OggPack(initial=5)
        original_storage = packed.storage
        writes = [(index * 0x9E3779B1, 32) for index in range(80)]
        for value, width in writes:
            packed.write(value, width)
        self.assertGreater(packed.storage, original_storage)
        self.assertEqual(packed.bytes_used(), 80 * 4)
        self.assertEqual(packed.get_buffer(), _reference_pack(writes))

        grown_storage = packed.storage
        packed.reset()
        self.assertEqual((packed.endbyte, packed.endbit, packed.bytes_used()), (0, 0, 0))
        self.assertEqual(packed.get_buffer(), b"")
        self.assertEqual(packed.storage, grown_storage)
        packed.write(0b101, 3)
        self.assertEqual(packed.get_buffer(), b"\x05")

    def test_reader_position_padding_and_eof_behavior(self) -> None:
        reader = BitReader(bytes.fromhex("ad7e"))
        self.assertEqual((reader.tell_bits(), reader.bits_left()), (0, 16))
        self.assertEqual(reader.read(3), 0b101)
        self.assertEqual((reader.tell_bits(), reader.bits_left()), (3, 13))
        self.assertEqual(reader.read(5), 0b10101)
        self.assertEqual((reader.tell_bits(), reader.bits_left()), (8, 8))
        self.assertEqual(reader.read(8), 0x7E)
        self.assertEqual((reader.tell_bits(), reader.bits_left()), (16, 0))
        self.assertEqual(reader.read(0), 0)
        with self.assertRaisesRegex(EOFError, "out of bits"):
            reader.read(1)
        self.assertEqual((reader.tell_bits(), reader.bits_left()), (16, 0))

        partial = BitReader(b"\x80")
        partial.read(7)
        with self.assertRaises(EOFError):
            partial.read(2)
        self.assertEqual((partial.tell_bits(), partial.bits_left()), (8, 0))

    def test_seeded_write_read_stream_is_deterministic(self) -> None:
        rng = random.Random(0x2013C0DE)
        writes: list[tuple[int, int]] = []
        for _ in range(500):
            width = rng.randrange(33)
            value = rng.getrandbits(48)
            if rng.randrange(2):
                value = -value
            writes.append((value, width))

        packed = OggPack(initial=7)
        for value, width in writes:
            packed.write(value, width)
        expected_buffer = _reference_pack(writes)
        self.assertEqual(packed.get_buffer(), expected_buffer)

        reader = BitReader(expected_buffer)
        for value, width in writes:
            mask = (1 << width) - 1 if width else 0
            self.assertEqual(reader.read(width), value & mask)
        padding = reader.bits_left()
        self.assertEqual(padding, (-sum(width for _, width in writes)) % 8)
        self.assertEqual(reader.read(padding), 0)
        with self.assertRaises(EOFError):
            reader.read(1)

if __name__ == "__main__":
    unittest.main()
