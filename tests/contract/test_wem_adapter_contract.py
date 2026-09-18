"""Path-adapter contracts for synthetic WEM containers.

These tests characterize the dictionary schemas and byte preservation
boundary of the canonical container codecs.
"""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from wwise_wem_reference.container.fmt import pack_pcm_ext_fmt
from wwise_wem_reference.container.riff import build_riff
from wwise_wem_reference.container.wem import (
    build_pcm_wem,
    build_vorbis_wem,
    load_wem_parts_bytes,
    parse_wem_bytes,
)


def _load_path(path: Path) -> dict:
    return load_wem_parts_bytes(
        path.read_bytes(), file=path.name, path=str(path)
    )


def _parse_path(path: Path) -> dict:
    return parse_wem_bytes(path.read_bytes(), file=path.name, path=str(path))


def _vorbis_fmt() -> dict:
    return {
        "wFormatTag": 0xFFFF,
        "nChannels": 6,
        "nSamplesPerSec": 44100,
        "nAvgBytesPerSec": 34381,
        "nBlockAlign": 0,
        "wBitsPerSample": 0,
        "cbSize": 48,
        "wReserved0": 0,
        "dwChannelMask": 0x3F,
        "dwTotalPCMFrames": 4096,
        "dwDataPayloadSize": 0,
        "dwUnknown_0x24": 0x03BA0000,
        "dwSeekTableSize": 0,
        "dwVorbisDataOffset": 0,
        "uMaxPacketSize": 0,
        "uUnknown_0x32": 954,
        "dwUnknown_0x34": 18180,
        "dwUnknown_0x38": 18636,
        "dwUnknown_0x3C": 0xB3DEA448,
        "uBlocksize0Pow": 8,
        "uBlocksize1Pow": 11,
    }


def _pcm_fmt(tag: int = 0xFFFE) -> dict:
    return {
        "wFormatTag": tag,
        "nChannels": 6,
        "nSamplesPerSec": 48000,
        "nAvgBytesPerSec": 576000,
        "nBlockAlign": 12,
        "wBitsPerSample": 16,
        "cbSize": 6 if tag == 0xFFFE else 0,
        "wSamples": 0,
        "dwChannelMask": 0x3F,
    }


class WemAdapterContractTests(unittest.TestCase):
    def _write(self, directory: str, name: str, raw: bytes) -> Path:
        path = Path(directory) / name
        path.write_bytes(raw)
        return path

    def test_path_adapters_match_pure_bytes_results_exactly(self):
        raw = build_vorbis_wem(
            _vorbis_fmt(),
            [b"setup", b"audio"],
            seek_table=b"seek",
            extra_chunks=[(b"JUNK", b"abc")],
        )
        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "pure-match.wem", raw)
            self.assertEqual(
                _parse_path(path),
                parse_wem_bytes(raw, file=path.name, path=str(path)),
            )
            self.assertEqual(
                _load_path(path),
                load_wem_parts_bytes(raw, file=path.name, path=str(path)),
            )

    def test_pure_bytes_defaults_supply_stable_source_metadata(self):
        raw = build_pcm_wem(_pcm_fmt(), b"\x00" * 12, junk=None)

        summary = parse_wem_bytes(raw)
        parts = load_wem_parts_bytes(raw)

        self.assertEqual((summary["file"], summary["path"]), ("<bytes>", "<bytes>"))
        self.assertEqual((parts["file"], parts["path"]), ("<bytes>", "<bytes>"))
        self.assertEqual(summary["kind"], "pcm_extensible")
        self.assertEqual(parts["pcm_data"], b"\x00" * 12)

    def test_vorbis_path_parts_summary_and_byte_exact_rebuild(self):
        packets = [b"setup-bytes", b"", b"\x01audio", b"\xff" * 5]
        seek = b"\x01\x00\x00\x00\x02\x00\x00\x00"
        extras = [(b"JUNK", b"abc"), (b"akd ", b"meta")]
        original = build_vorbis_wem(
            _vorbis_fmt(), packets, seek_table=seek, extra_chunks=extras
        )

        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "vorbis.wem", original)
            parts = _load_path(path)
            self.assertEqual(
                set(parts),
                {
                    "path",
                    "file",
                    "endian",
                    "raw",
                    "fmt_raw",
                    "data_raw",
                    "extras_before_data",
                    "extras_after_data",
                    "chunk_ids",
                    "kind",
                    "fmt",
                    "seek_table",
                    "packets",
                    "sizes",
                },
            )
            self.assertEqual(parts["path"], str(path))
            self.assertEqual(parts["file"], "vorbis.wem")
            self.assertEqual(parts["endian"], "le")
            self.assertEqual(parts["raw"], original)
            self.assertEqual(parts["kind"], "wwise_vorbis")
            self.assertEqual(parts["seek_table"], seek)
            self.assertEqual(parts["packets"], packets)
            self.assertEqual(parts["sizes"], [len(packet) for packet in packets])
            self.assertEqual(parts["extras_before_data"], extras)
            self.assertEqual(parts["extras_after_data"], [])
            self.assertEqual(parts["chunk_ids"], ["fmt ", "JUNK", "akd ", "data"])

            fmt = parts["fmt"]
            expected_first_audio = len(seek) + 2 + len(packets[0])
            self.assertEqual(fmt["dwSeekTableSize"], len(seek))
            self.assertEqual(fmt["dwDataPayloadSize"], len(parts["data_raw"]))
            self.assertEqual(fmt["dwFirstAudioPacketOffset"], expected_first_audio)
            self.assertEqual(fmt["dwVorbisDataOffset"], expected_first_audio)
            # largest *audio* packet: the paired build writes 1 for an all-silent
            # stream whose setup packet is 215 bytes, so the setup is not counted
            self.assertEqual(fmt["uMaxPacketSize"], max(map(len, packets[1:])))

            rebuilt = build_vorbis_wem(
                fmt,
                parts["packets"],
                seek_table=parts["seek_table"],
                endian=parts["endian"],
                extra_chunks=parts["extras_before_data"],
                recompute_sizes=False,
                fmt_raw=parts["fmt_raw"],
            )
            self.assertEqual(rebuilt, original)

            summary = _parse_path(path)
            self.assertEqual(
                set(summary),
                {
                    "file",
                    "path",
                    "size",
                    "endian",
                    "chunks",
                    "fmt_chunk_size",
                    "kind",
                    "fmt",
                    "data",
                    "has_junk",
                    "has_smpl",
                },
            )
            self.assertEqual(summary["kind"], "wwise_vorbis")
            self.assertEqual(summary["data"]["packet_count"], len(packets))
            self.assertEqual(summary["data"]["setup_packet_size"], len(packets[0]))
            self.assertTrue(summary["data"]["checks"]["data_size_field"])
            self.assertTrue(summary["data"]["checks"]["first_audio_field_eq_vorbis_off"])
            self.assertTrue(summary["data"]["checks"]["packets_consume_data"])
            self.assertTrue(summary["data"]["checks"]["vorbis_offset_vs_setup"])
            self.assertTrue(summary["has_junk"])
            self.assertFalse(summary["has_smpl"])

    def test_big_endian_vorbis_packet_framing_and_exact_rebuild(self):
        packets = [b"setup", b"a", b"audio"]
        seek = b"seek"
        extras = [(b"JUNK", b"even")]
        original = build_vorbis_wem(
            _vorbis_fmt(),
            packets,
            seek_table=seek,
            endian="be",
            extra_chunks=extras,
        )

        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "big-endian.wem", original)
            parts = _load_path(path)
            self.assertEqual(original[:4], b"RIFX")
            self.assertEqual(parts["endian"], "be")
            self.assertEqual(parts["seek_table"], seek)
            self.assertEqual(parts["packets"], packets)
            self.assertEqual(parts["sizes"], [5, 1, 5])

            summary = _parse_path(path)
            self.assertTrue(summary["data"]["checks"]["packets_consume_data"])
            self.assertEqual(summary["data"]["packet_count"], len(packets))
            self.assertTrue(summary["data"]["checks"]["vorbis_offset_vs_setup"])

            rebuilt = build_vorbis_wem(
                parts["fmt"],
                parts["packets"],
                seek_table=parts["seek_table"],
                endian=parts["endian"],
                extra_chunks=parts["extras_before_data"],
                recompute_sizes=False,
                fmt_raw=parts["fmt_raw"],
            )
            self.assertEqual(rebuilt, original)

    def test_pcm_extensible_path_summary_and_exact_rebuild(self):
        pcm = bytes(range(36))  # three 6-channel, 16-bit frames
        extras = [(b"smpl", b"loop"), (b"LIST", b"tags")]
        original = build_pcm_wem(
            _pcm_fmt(), pcm, junk=b"pad!", extra_chunks=extras
        )

        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "pcm-ext.wem", original)
            parts = _load_path(path)
            self.assertEqual(
                set(parts),
                {
                    "path",
                    "file",
                    "endian",
                    "raw",
                    "fmt_raw",
                    "data_raw",
                    "extras_before_data",
                    "extras_after_data",
                    "chunk_ids",
                    "kind",
                    "fmt",
                    "pcm_data",
                },
            )
            self.assertEqual(parts["kind"], "pcm_extensible")
            self.assertEqual(parts["pcm_data"], pcm)
            self.assertEqual(
                parts["extras_before_data"], [(b"JUNK", b"pad!"), *extras]
            )
            self.assertEqual(parts["chunk_ids"], ["fmt ", "JUNK", "smpl", "LIST", "data"])

            summary = _parse_path(path)
            self.assertEqual(summary["kind"], "pcm_extensible")
            self.assertEqual(summary["data"]["frames"], 3)
            self.assertEqual(summary["data"]["duration_sec"], round(3 / 48000, 6))
            self.assertTrue(summary["has_junk"])
            self.assertTrue(summary["has_smpl"])

            rebuilt = build_pcm_wem(
                parts["fmt"],
                parts["pcm_data"],
                endian=parts["endian"],
                junk=parts["extras_before_data"][0][1],
                extra_chunks=parts["extras_before_data"][1:],
                fmt_raw=parts["fmt_raw"],
            )
            self.assertEqual(rebuilt, original)

    def test_pcm_plain_rifx_path_adapter_and_exact_rebuild(self):
        pcm = b"\x00\x01" * 8
        original = build_pcm_wem(
            _pcm_fmt(1),
            pcm,
            endian="be",
            junk=None,
            extra_chunks=[(b"LIST", b"metadata")],
        )

        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "pcm-plain-rifx.wem", original)
            parts = _load_path(path)
            summary = _parse_path(path)
            self.assertEqual(original[:4], b"RIFX")
            self.assertEqual(parts["endian"], "be")
            self.assertEqual(parts["kind"], "pcm_plain")
            self.assertEqual(parts["pcm_data"], pcm)
            self.assertEqual(parts["extras_before_data"], [(b"LIST", b"metadata")])
            self.assertEqual(summary["endian"], "be")
            self.assertEqual(summary["kind"], "pcm_plain")
            self.assertEqual(summary["data"]["frames"], len(pcm) // 12)

            rebuilt = build_pcm_wem(
                parts["fmt"],
                parts["pcm_data"],
                endian=parts["endian"],
                junk=None,
                extra_chunks=parts["extras_before_data"],
                fmt_raw=parts["fmt_raw"],
            )
            self.assertEqual(rebuilt, original)

    def test_load_parts_partitions_extras_around_fmt_and_data(self):
        fmt = pack_pcm_ext_fmt(_pcm_fmt())
        raw = build_riff(
            [
                (b"PRE ", b"before-fmt"),
                (b"fmt ", fmt),
                (b"MID ", b"before-data"),
                (b"data", b"\x00" * 12),
                (b"POST", b"after-data"),
            ]
        )
        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "extra-order.wem", raw)
            parts = _load_path(path)
            self.assertEqual(parts["extras_before_data"], [(b"MID ", b"before-data")])
            self.assertEqual(
                parts["extras_after_data"],
                [(b"PRE ", b"before-fmt"), (b"POST", b"after-data")],
            )
            self.assertEqual(
                parts["chunk_ids"], ["PRE ", "fmt ", "MID ", "data", "POST"]
            )

    def test_missing_fmt_error_contracts(self):
        raw = build_riff([(b"data", b"payload")])
        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "missing-fmt.wem", raw)
            with self.assertRaisesRegex(ValueError, "missing fmt or data"):
                _load_path(path)
            summary = _parse_path(path)
            self.assertEqual(summary["error"], "missing fmt")
            self.assertNotIn("kind", summary)


if __name__ == "__main__":
    unittest.main()
