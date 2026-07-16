from __future__ import annotations

import unittest

import wwise_wem.container as container
import wwise_wem.container.fmt as fmt_codec


def _all_vorbis_fields() -> dict:
    return {
        "wFormatTag": "0xffff",
        "nChannels": 6,
        "nSamplesPerSec": 44100,
        "nAvgBytesPerSec": 34381,
        "nBlockAlign": 7,
        "wBitsPerSample": 16,
        "cbSize": 48,
        "wReserved0": 0x1234,
        "dwChannelMask": "0x3f",
        "dwTotalPCMFrames": 0x10203040,
        "dwFirstAudioPacketOffset": 0x11223344,
        "dwDataPayloadSize": 0x55667788,
        "dwUnknown_0x24": "0x03ba0000",
        "dwSeekTableSize": 0x12345678,
        "dwVorbisDataOffset": 0x22334455,
        "uMaxPacketSize": 0xABCD,
        "uUnknown_0x32": 0x1357,
        "dwUnknown_0x34": 0x89ABCDEF,
        "dwUnknown_0x38": 0x76543210,
        "dwUnknown_0x3C": "0xb3dea448",
        "uBlocksize0Pow": 8,
        "uBlocksize1Pow": 11,
    }


class VorbisFmtCodecTests(unittest.TestCase):
    def test_all_66_bytes_and_fields_roundtrip(self) -> None:
        fields = _all_vorbis_fields()
        encoded = fmt_codec.pack_vorbis_fmt(fields)
        self.assertEqual(len(encoded), 66)
        parsed = fmt_codec.parse_vorbis_fmt(encoded)
        self.assertEqual(parsed["wFormatTag"], "0xffff")
        self.assertEqual(parsed["dwChannelMask"], "0x3f")
        self.assertEqual(parsed["dwUnknown_0x24"], "0x3ba0000")
        self.assertEqual(parsed["dwUnknown_0x3C"], "0xb3dea448")
        for name in (
            "nChannels",
            "nSamplesPerSec",
            "nAvgBytesPerSec",
            "nBlockAlign",
            "wBitsPerSample",
            "cbSize",
            "wReserved0",
            "dwTotalPCMFrames",
            "dwFirstAudioPacketOffset",
            "dwDataPayloadSize",
            "dwSeekTableSize",
            "dwVorbisDataOffset",
            "uMaxPacketSize",
            "uUnknown_0x32",
            "dwUnknown_0x34",
            "dwUnknown_0x38",
            "uBlocksize0Pow",
            "uBlocksize1Pow",
        ):
            self.assertEqual(parsed[name], fields[name], name)
        self.assertEqual((parsed["blocksize0"], parsed["blocksize1"]), (256, 2048))
        self.assertEqual(fmt_codec.pack_vorbis_fmt(parsed), encoded)

    def test_raw_override_and_malformed_payload_errors(self) -> None:
        raw = bytes((index * 37) & 0xFF for index in range(66))
        self.assertEqual(fmt_codec.pack_vorbis_fmt({"raw_hex": raw.hex()}), raw)
        with self.assertRaisesRegex(ValueError, "fmt len"):
            fmt_codec.pack_vorbis_fmt({"raw_hex": "00"})
        with self.assertRaises(ValueError):
            fmt_codec.pack_vorbis_fmt({"raw_hex": "not hex"})
        with self.assertRaisesRegex(ValueError, "expected 66 bytes"):
            fmt_codec.parse_vorbis_fmt(b"\0" * 65)


class PcmFmtCodecTests(unittest.TestCase):
    def test_extensible_24_byte_roundtrip(self) -> None:
        fields = {
            "wFormatTag": "0xfffe",
            "nChannels": 6,
            "nSamplesPerSec": 48000,
            "nAvgBytesPerSec": 576000,
            "nBlockAlign": 12,
            "wBitsPerSample": 16,
            "cbSize": 6,
            "wSamples": 16,
            "dwChannelMask": "0x3f",
        }
        encoded = fmt_codec.pack_pcm_ext_fmt(fields)
        self.assertEqual(len(encoded), 24)
        parsed = fmt_codec.parse_pcm_ext_fmt(encoded)
        self.assertEqual(parsed, fields)
        self.assertEqual(fmt_codec.pack_pcm_ext_fmt(parsed), encoded)

    def test_plain_pcm_and_raw_override_preserve_existing_shapes(self) -> None:
        plain = fmt_codec.pack_pcm_ext_fmt(
            {
                "wFormatTag": 1,
                "nChannels": 2,
                "nSamplesPerSec": 44100,
                "wBitsPerSample": 16,
            }
        )
        self.assertEqual(len(plain), 18)
        self.assertEqual(fmt_codec.parse_pcm_ext_fmt(plain)["wFormatTag"], "0x1")
        raw = bytes(range(24))
        self.assertEqual(fmt_codec.pack_pcm_ext_fmt({"raw_hex": raw.hex()}), raw)


class ContainerExportTests(unittest.TestCase):
    def test_container_exports_share_codec_identity(self) -> None:
        self.assertIs(container.parse_vorbis_fmt, fmt_codec.parse_vorbis_fmt)
        self.assertIs(container.pack_vorbis_fmt, fmt_codec.pack_vorbis_fmt)


if __name__ == "__main__":
    unittest.main()
