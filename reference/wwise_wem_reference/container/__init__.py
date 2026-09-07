"""Pure byte-level codecs for Wwise container structures."""

from .fmt import (
    pack_pcm_ext_fmt,
    pack_vorbis_fmt,
    parse_pcm_ext_fmt,
    parse_vorbis_fmt,
)
from .model import RiffChunk, WemFmt, WemParts
from .packets import (
    build_packet_stream,
    extract_packets,
    recompute_vorbis_fmt_sizes,
    walk_packets,
)
from .riff import build_riff, parse_chunks
from .wem import (
    build_pcm_wem,
    build_vorbis_wem,
    load_wem_parts_bytes,
    parse_wem_bytes,
)

__all__ = [
    "build_riff",
    "build_packet_stream",
    "build_pcm_wem",
    "extract_packets",
    "build_vorbis_wem",
    "load_wem_parts_bytes",
    "pack_pcm_ext_fmt",
    "pack_vorbis_fmt",
    "parse_chunks",
    "parse_pcm_ext_fmt",
    "parse_vorbis_fmt",
    "parse_wem_bytes",
    "RiffChunk",
    "recompute_vorbis_fmt_sizes",
    "WemFmt",
    "WemParts",
    "walk_packets",
]
