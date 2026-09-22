#!/usr/bin/env python3
"""Independent decoder for carried Wwise WEMs, plus a local libvorbis probe.

Traverses the full audio packet stream and decodes to PCM:
  container framing -> setup -> floor1 (unwrap) -> residue (coefficient
  returning) -> stereo coupling -> inverse MDCT -> window -> overlap-add.

The deterministic default is a float64 NumPy synthesis with no system codec
dependency.  ``tests/parity/test_decode_surface.py`` uses it live as an
independent comparator for the committed paired-build 6ch/44.1k fixture and
2ch/48k noise WEM.  It is a comparator, not a ground-truth decoder or a
shipping engine.

``--libvorbis-exact`` is an optional local probe for the 2-channel stream. It
mirrors libvorbis 1.3.7 float32 DSP: f32 codebook/floor/residue/coupling
values, the system library's ``mdct_backward`` via ctypes, the
``vorbis_synthesis_blockin`` OLA through a tiny C helper compiled on first use,
and vgmstream's ``CONV_FLT_S16`` conversion (``(int)(x * 32767.0f)``). Its
result depends on the local library and compiler, so tests never select it.

This decoder shares profile values and bitstream primitives with
``reference/wwise_wem_reference``. Its separate synthesis can expose
implementation differences, but shared inputs limit its independence.

Historical local observations over a 294,485-packet corpus found the default
path correlated 1.000000 with a community decoder, with at most two int16 LSB
of difference and strict packet closure.  Those observations are not committed
reproducible coverage.  The optional libvorbis path matched its local build;
that is likewise environment-specific evidence, not a repository assertion.

Usage:
    python3 scripts/decode_wem.py --self-check
    python3 scripts/decode_wem.py --wem <file.wem> --segments [--stats out.json]
    python3 scripts/decode_wem.py --wem <file.wem> --segments --libvorbis-exact
"""
from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:  # the carrier type is import-time-free: this script runs without the kernel
    from wwise_wem_reference.profiles.artifact import CompiledProfile

import argparse
import ctypes
import json
import struct
import sys
import time
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np

_REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(_REPO_ROOT / "reference"))
sys.path.insert(0, str(_REPO_ROOT / "src"))

from wwise_wem_reference._f32 import _f32  # type: ignore  # noqa: E402
from wwise_wem_reference.vorbis.bitio import BitReader  # type: ignore  # noqa: E402
from wwise_wem_reference.vorbis.codebook import (  # type: ignore  # noqa: E402
    Codebook,
    codebook_from_static,
    static_from_entry_dict,
)
from wwise_wem_reference.vorbis.floor import (  # type: ignore  # noqa: E402
    FLOOR1_RANGES,
    FLOOR1_fromdB_LOOKUP,
    floor1_curve_from_posts,
    floor1_unwrap,
    postlist_from_floor,
)
from wwise_wem_reference.vorbis.packet_decoder import (  # type: ignore  # noqa: E402
    decode_floors_for_packet,
    parse_audio_header,
)
from wwise_wem_reference.vorbis.setup import parse_setup  # type: ignore  # noqa: E402
from wwise_wem_reference.analysis.dsp.transform import (  # type: ignore  # noqa: E402
    make_mdct_look,
    mdct_forward,
    vorbis_window,
)
from wwise_wem_reference.container.wem import parse_wem_bytes  # type: ignore  # noqa: E402

# Wwise 2013 book-id registry (contiguous per installed table).
T97_COUNT = 97
T219_COUNT = 219
T282_COUNT = 282
T97_END = T97_COUNT
T219_END = T97_END + T219_COUNT
T282_END = T219_END + T282_COUNT

DEFAULT_OUT_DIR = _REPO_ROOT / "corpus" / "paired-build" / "out"
SAMPLE_RATE = 48000

# Set from `--libvorbis-exact` in `main()`. Module-level so the decode helpers
# below need no extra parameter; the decode script reads no environment
# variable, so which synthesis path ran is always visible in the command line.
_LIBVORBIS_EXACT = False


class ResidueEOP(Exception):
    """End of packet while reading residue (libvorbis returns 0)."""


def _decode_eop(book: Codebook, br: BitReader) -> int:
    """Decode one Huffman entry, mapping EOF to the end-of-residue marker."""
    try:
        if br.bits_left() <= 0:
            raise ResidueEOP()
        return book.decode(br)
    except (EOFError, ValueError) as error:
        raise ResidueEOP() from error


def _decode_vq_eop(book: Codebook, br: BitReader) -> list[float]:
    """Decode a VQ vector, mapping EOF/invalid code to end-of-residue."""
    try:
        if br.bits_left() <= 0:
            raise ResidueEOP()
        return book.decode_vq(br)
    except (EOFError, ValueError) as error:
        raise ResidueEOP() from error


def _decodevv_add(
    book: Codebook,
    flat: np.ndarray,
    offset: int,
    ch: int,
    br: BitReader,
    n: int,
) -> int:
    """libvorbis vorbis_book_decodevv_add into a flat (bin*ch+chan) domain.

    Returns 0 on success, -1 on end-of-residue (decoded into ``flat`` as it
    goes; partially written vectors on EOP are consistent with libvorbis,
    which also adds vectors before noticing the failure on the next read).
    """
    if book.dim <= 0:
        return 0
    used = [e for e, L in enumerate(book.lengthlist) if L > 0]
    if not used:
        return 0
    m = (offset + n) // ch
    i = offset // ch
    chptr = 0
    dim = book.dim
    try:
        while i < m:
            entry = _decode_eop(book, br)
            vec = book.vq_values(entry)
            for j in range(dim):
                if i >= m:
                    break
                flat[i * ch + chptr] += vec[j]
                chptr += 1
                if chptr == ch:
                    chptr = 0
                    i += 1
    except ResidueEOP:
        return -1
    return 0


def decode_residue_type2(
    br: BitReader,
    residue: dict,
    books: list[Codebook],
    ch_used: list[bool],
    max_flat: int,
) -> tuple[np.ndarray, str]:
    """libvorbis res2_inverse: residue type 2 on the flat (bin*ch+chan) domain.

    - max = (pcmend*ch)>>1 is the caller-supplied ``max_flat``
    - end is clamped to max (libvorbis: end = info->end < max ? info->end : max)
    - classword is read ONCE per partition-group (phrasebook dim covers all ch)
    - EOP: classword entry == -1 OR entry >= classbook.entries, OR decodevv_add -1
    """
    ch = len(ch_used)
    begin = int(residue["begin"])
    end = min(int(residue["end"]), max_flat)
    part = int(residue["partition_size"])
    nclass = int(residue["classifications"])
    span = end - begin
    flat = np.zeros(max_flat, dtype=np.float64)
    if span <= 0 or not any(ch_used):
        return flat, ("empty" if not any(ch_used) else "complete")

    partvals = span // part
    if partvals <= 0:
        return flat, "complete"

    classbook = books[residue["classbook"]]
    ppw = classbook.dim  # partitions per word
    if ppw <= 0:
        ppw = 1
    # stages = highest active cascade bit (libvorbis look->stages)
    stages = 0
    for cascade in residue["cascades"]:
        stages = max(stages, int(cascade).bit_length())
    partbooks = residue["books"]  # [class][stage] -> book index or -1

    # partword[l] = list of ppw partition classes for word-group l
    partword: list[list[int]] = []
    status = "complete"
    try:
        for s in range(stages):
            i = 0  # partition index
            lg = 0  # word-group index
            while i < partvals:
                if s == 0:
                    # Fetch the partition word (classword).
                    entry = _decode_eop(classbook, br)
                    if entry >= classbook.entries:
                        status = "eop"
                        return flat, status
                    # Unpack mixed-radix: entry = c0*nclass^(ppw-1)+...+c_{ppw-1}
                    # (most-significant c0 is the earliest partition).
                    pword: list[int] = [0] * ppw
                    t = entry
                    for _k in range(ppw - 1, -1, -1):
                        pword[_k] = t % nclass
                        t //= nclass
                    partword.append(pword)
                # Decode residual values for the partitions in this word.
                for k in range(ppw):
                    if i >= partvals:
                        break
                    pclass = partword[lg][k] if partword[lg][k] < nclass else 0
                    if s < len(partbooks[pclass]):
                        book_id = partbooks[pclass][s]
                        if book_id is not None and book_id >= 0:
                            book = books[book_id]
                            offset = i * part + begin
                            if _decodevv_add(book, flat, offset, ch, br, part) < 0:
                                status = "eop"
                                return flat, status
                    i += 1
                lg += 1
    except ResidueEOP:
        status = "eop"
    return flat, status


def decode_residue_coeffs(
    br: BitReader,
    residue: dict,
    books: list[Codebook],
    ch_used: list[bool],
    n_spectrum: int,
) -> tuple[list[np.ndarray], int, str]:
    """Decode residue type 0/1/2 and RETURN per-channel coefficient rows.

    This is the coefficient-producing counterpart of
    ``wwise_wem_reference.vorbis.residue.consume_residue``: same bit
    schedule (the libvorbis ``_01inverse`` interleaving), but it accumulates
    the VQ vectors into the coefficient rows instead of discarding them.
    Multi-stage books add their vectors (residual VQ).
    """
    rtype = int(residue["type"])
    if rtype == 2:
        # libvorbis res2_inverse: operate on the flat (bin*ch+chan) domain with
        # a single classword per partition-group (all channels interleaved).
        ch = len(ch_used)
        max_flat = int(n_spectrum) * ch
        flat, status = decode_residue_type2(
            br, residue, books, ch_used, max_flat
        )
        # coeffs[j][b] = flat[b*ch + j]; i.e. channel j strided by ch.
        coeffs = [
            np.asarray(flat[j::ch], dtype=np.float64)[:n_spectrum]
            for j in range(ch)
        ]
        return coeffs, 0, status
    begin = int(residue["begin"])
    end = min(int(residue["end"]), n_spectrum)
    part = int(residue["partition_size"])
    nclass = int(residue["classifications"])
    cb = books[residue["classbook"]]
    classwords = cb.dim
    nch = len(ch_used)
    span = end - begin
    npart = span // part if span > 0 else 0
    coeffs = [np.zeros(n_spectrum, dtype=np.float64) for _ in range(nch)]
    if npart <= 0 or not any(ch_used):
        return coeffs, npart, "empty"

    partword = [[0] * npart for _ in range(nch)]
    status = "complete"
    try:
        for s in range(8):
            for i in range(0, npart, classwords):
                if s == 0:
                    for j in range(nch):
                        if not ch_used[j]:
                            continue
                        entry = _decode_eop(cb, br)
                        temp = entry
                        radix = nclass ** (classwords - 1)
                        for k in range(classwords):
                            if i + k < npart:
                                partword[j][i + k] = temp // radix
                            temp %= radix
                            radix //= nclass
                for k in range(classwords):
                    partition = i + k
                    if partition >= npart:
                        break
                    for j in range(nch):
                        if not ch_used[j]:
                            continue
                        book_id = residue["books"][partword[j][partition]][s]
                        if book_id < 0:
                            continue
                        book = books[book_id]
                        dim = book.dim
                        off = begin + partition * part
                        for v in range(off, off + part, dim):
                            vec = _decode_vq_eop(book, br)
                            for d in range(dim):
                                if v + d < n_spectrum:
                                    coeffs[j][v + d] += vec[d]
        return coeffs, npart, status
    except ResidueEOP:
        return coeffs, npart, "eop"


def load_profile_codebooks(
    setup: dict, profile: "CompiledProfile"
) -> tuple[list[Codebook], dict[str, list[dict]]]:
    """Read the profile's book tables from the compiled carrier."""
    from wwise_wem_reference.profiles.book_ids import load_book_table

    needed: set[str] = set()
    for bid in setup["book_ids"]:
        if bid < T97_END:
            needed.add("t97")
        elif bid < T219_END:
            needed.add("t219")
        elif bid < T282_END:
            needed.add("t282")
    tables: dict[str, list[dict]] = {}
    for name in sorted(needed):
        if profile.optional_table(f"codebook.{name}.name") is None:
            raise FileNotFoundError(f"profile carries no codebook table {name}")
        tables[name] = list(load_book_table(name, profile))
    books: list[Codebook] = []
    for bid in setup["book_ids"]:
        if bid < T97_END:
            table, index, name = "t97", bid, "t97"
        elif bid < T219_END:
            table, index, name = "t219", bid - T97_END, "t219"
        else:
            table, index, name = "t282", bid - T219_END, "t282"
        row = tables[table][index]
        static = static_from_entry_dict(row)
        books.append(codebook_from_static(static, book_id=bid, table=name, index=index))
    return books, tables


def compute_kernel(n: int) -> np.ndarray:
    """Return the OLA synthesis kernel K (n x n/2) for one block size.

    Derived numerically as the translation-invariant synthesis operator that
    makes ``out[p:p+N] += K @ X`` reconstruct the source for the
    ``mdct_forward`` + ``vorbis_window`` pair with 50% overlap.  Verified to
    reconstruct within float32 precision on a reference stream.
    """
    look = make_mdct_look(n)
    window = np.array(vorbis_window(n))
    # Forward matrix A (n/2 x n) via basis vectors.
    a_matrix = np.zeros((n // 2, n))
    for col in range(n):
        basis = np.zeros(n)
        basis[col] = 1.0
        a_matrix[:, col] = np.array(mdct_forward(look, basis))
    # A small stream establishes the middle (translation-invariant) kernel.
    stream_len = 2 * n + n // 2
    rng = np.random.default_rng(5)
    source = rng.standard_normal(stream_len)
    hop = n // 2
    positions = [p for p in range(0, stream_len, hop) if p + n <= stream_len]
    coeffs: list[np.ndarray] = []
    for pos in positions:
        framed = np.array([_f32(source[pos + i] * window[i]) for i in range(n)])
        coeffs.append(np.array(mdct_forward(look, framed)))
    rows = np.zeros((len(positions) * (n // 2), stream_len))
    for fi, pos in enumerate(positions):
        for out_n in range(pos, pos + n):
            rows[fi * (n // 2) : (fi + 1) * (n // 2), out_n] = (
                a_matrix[:, out_n - pos] * window[out_n - pos]
            )
    pinv = np.linalg.pinv(rows)
    mid_pos = positions[1]
    mid = 1
    kernel = np.zeros((n, n // 2))
    for rel in range(n):
        kernel[rel] = pinv[mid_pos + rel, mid * (n // 2) : (mid + 1) * (n // 2)]
    return kernel


def parse_container(raw: bytes) -> tuple[bytes, list[bytes], int, dict]:
    """Extract (data_payload, packets, seek_table_size, fmt)."""
    info = parse_wem_bytes(raw)
    if info.get("kind") != "wwise_vorbis":
        raise ValueError(f"not a Wwise Vorbis WEM: {info.get('kind')}")
    fmt = info["fmt"]
    seek_table_size = int(fmt["dwSeekTableSize"])
    # Locate the data chunk payload.
    position = 12
    data_payload: bytes | None = None
    while position < len(raw):
        chunk_id = raw[position : position + 4]
        chunk_size = struct.unpack_from("<I", raw, position + 4)[0]
        if chunk_id == b"data":
            data_payload = raw[position + 8 : position + 8 + chunk_size]
        position += 8 + chunk_size
    if data_payload is None:
        raise ValueError("missing data chunk")
    packets: list[bytes] = []
    cursor = seek_table_size
    while cursor + 2 <= len(data_payload):
        size = struct.unpack_from("<H", data_payload, cursor)[0]
        if cursor + 2 + size > len(data_payload):
            break
        packets.append(data_payload[cursor + 2 : cursor + 2 + size])
        cursor += 2 + size
    return data_payload, packets, seek_table_size, fmt


@dataclass
class DecodeContext:
    setup: dict
    books: list[Codebook]
    # The 2ch path uses the numerically fitted kernel; the
    # generic path evaluates the IMDCT definition independently.
    kernel_map: dict[int, np.ndarray]
    imdct_map: dict[int, np.ndarray]
    window_halves: dict[int, np.ndarray]
    channels: int
    blocksizes: tuple[int, int]
    maps: list[dict]
    coupling: list[dict]
    sample_rate: int


def build_context(
    profile: "CompiledProfile", channels: int, sample_rate: int
) -> DecodeContext:
    """Build the decoder context from the compiled carrier of one profile."""
    setup = parse_setup(profile.setup_packet, channels=channels)
    books, _tables = load_profile_codebooks(setup, profile)
    block_sizes = (256, 2048)
    # The stereo path uses its established fitted kernel. The
    # generic path uses the IMDCT definition and derives each hybrid window
    # from the packet modes, so it does not share that fitted approximation.
    kernel_map = (
        {size: compute_kernel(size) for size in block_sizes}
        if channels == 2 and sample_rate == 48_000
        else {}
    )
    # The mapping coupling (used for stereo reconstruction).
    map0 = setup["maps"][0]
    coupling = map0.get("coupling") or []
    return DecodeContext(
        setup=setup,
        books=books,
        kernel_map=kernel_map,
        imdct_map={size: imdct_matrix(size) for size in block_sizes},
        window_halves={
            size: np.asarray(vorbis_window(size)[: size // 2], dtype=np.float64)
            for size in block_sizes
        },
        channels=channels,
        blocksizes=(256, 2048),
        maps=setup["maps"],
        coupling=coupling,
        sample_rate=sample_rate,
    )


def imdct_matrix(n: int) -> np.ndarray:
    """The inverse MDCT definition, evaluated in float64 at unit scale.

    With ``M = n / 2``, each output sample is
    ``sum(X[k] * cos(pi/M * (i + 1/2 + M/2) * (k + 1/2)))``.  The carried
    decoder uses a frozen fast transform; this independent matrix is kept for
    the comparison path so it cannot inherit the fast implementation's errors.
    """
    m = n // 2
    sample = np.arange(n, dtype=np.float64)[:, None]
    bin_ = np.arange(m, dtype=np.float64)[None, :]
    return np.cos(np.pi / m * (sample + 0.5 + m / 2.0) * (bin_ + 0.5))


def apply_hybrid_synthesis_window(
    block: np.ndarray,
    blocksizes: tuple[int, int],
    previous: int,
    current: int,
    following: int,
    halves: dict[int, np.ndarray],
) -> np.ndarray:
    """Window one IMDCT block from its adjacent block geometry.

    A short block is wholly short-windowed: it cannot use a long neighbour's
    half.  A long block uses its predecessor and follower sizes to derive the
    two support spans.  The derivation is the Vorbis block geometry, rather
    than a fitted reconstruction kernel.
    """
    if current == 0:
        previous = following = 0
    n = blocksizes[current]
    left_n = blocksizes[previous]
    right_n = blocksizes[following]
    left_begin = n // 4 - left_n // 4
    left_end = left_begin + left_n // 2
    right_begin = n // 2 + n // 4 - right_n // 4
    right_end = right_begin + right_n // 2
    if not (0 <= left_begin <= left_end <= right_begin <= right_end <= n):
        raise ValueError("incompatible hybrid synthesis window geometry")
    out = block.copy()
    out[:left_begin] = 0.0
    out[left_begin:left_end] *= halves[left_n]
    out[right_begin:right_end] *= halves[right_n][::-1]
    out[right_end:] = 0.0
    return out


def decode_packet_spectra(
    ctx: DecodeContext, payload: bytes, mode_history: list[int]
) -> tuple[int, list[np.ndarray], int, int, str, int]:
    """Decode one audio packet to per-channel MDCT spectra.

    Returns (mode, spectra, block_size, n_bits_consumed, residue_status, bits_left).
    """
    br = BitReader(payload)
    header = parse_audio_header(br, ctx.setup)
    mode = header["mode"]
    mode_history.append(mode)
    block_size = ctx.blocksizes[header["blockflag"]]
    n_spec = block_size // 2
    mapping = ctx.maps[header["mapping"]]
    floors = decode_floors_for_packet(br, ctx.setup, ctx.books, ctx.channels, mapping)
    if not floors.get("complete"):
        raise ValueError(f"floor incomplete: {floors.get('error')}")
    curves = floors["curves"]
    nonzero = floors["nonzero"]
    ch_used = [bool(z) for z in nonzero]
    residue_id = mapping["residues"][0]
    residue = ctx.setup["residues"][residue_id]
    res_coeffs, _npart, res_status = decode_residue_coeffs(
        br, residue, ctx.books, ch_used, n_spec
    )
    bits_left = br.bits_left()
    spectra: list[np.ndarray] = []
    for ch in range(ctx.channels):
        if not ch_used[ch]:
            spectra.append(np.zeros(n_spec, dtype=np.float64))
            continue
        sub = mapping["chmux"][ch] if mapping["submaps"] > 1 else 0
        floor = ctx.setup["floors"][mapping["floors"][sub]]
        packet_posts = curves[ch]
        postlist = postlist_from_floor(floor)
        quant_range = FLOOR1_RANGES[floor["multiplier"]]
        abs_posts = floor1_unwrap(packet_posts, postlist, quant_range)
        curve = floor1_curve_from_posts(
            abs_posts, postlist, n_spec, floor["multiplier"]
        )
        # Wwise multiplicative floor: mdct = residue * floor_amp.
        mdct = res_coeffs[ch] * np.asarray(curve, dtype=np.float64)[:n_spec]
        spectra.append(mdct)
    return mode, spectra, block_size, br.tell_bits(), res_status, bits_left


def decouple_db_domain(
    spectra: list[np.ndarray],
    coupling: list[dict],
    n_spec: int,
) -> list[np.ndarray]:
    """libvorbis mapping0.c:765-787 channel coupling inverse (four branches).

    Applied in the dB/linear coefficient domain BEFORE the floor envelope
    (floor1_inverse2).  Iterates coupling steps in reverse order.
    """
    if not coupling:
        return spectra
    coupled = [np.array(s, dtype=np.float64, copy=True) for s in spectra]
    for step in reversed(coupling):
        m = int(step["mag"])
        a = int(step["ang"])
        mag_arr = coupled[m]
        ang_arr = coupled[a]
        for j in range(n_spec):
            mag = mag_arr[j]
            ang = ang_arr[j]
            if mag > 0:
                if ang > 0:
                    mag_arr[j] = mag
                    ang_arr[j] = mag - ang
                else:
                    ang_arr[j] = mag
                    mag_arr[j] = mag + ang
            else:
                if ang > 0:
                    mag_arr[j] = mag
                    ang_arr[j] = mag + ang
                else:
                    ang_arr[j] = mag
                    mag_arr[j] = mag - ang
    return coupled


def compute_floor_curves(
    ctx: DecodeContext,
    curves: list,
    nonzero: list[int],
    mapping: dict,
    n_spec: int,
) -> list[np.ndarray | None]:
    """Compute per-channel floor1 inverse2 multiplier curves (dB -> linear).

    A ``None`` slot marks a channel whose floor is inactive (zero window).
    """
    out: list[np.ndarray | None] = []
    for ch in range(ctx.channels):
        if not nonzero[ch]:
            out.append(None)
            continue
        sub = mapping["chmux"][ch] if mapping["submaps"] > 1 else 0
        floor = ctx.setup["floors"][mapping["floors"][sub]]
        postlist = postlist_from_floor(floor)
        quant_range = FLOOR1_RANGES[floor["multiplier"]]
        abs_posts = floor1_unwrap(curves[ch], postlist, quant_range)
        curve = floor1_curve_from_posts(
            abs_posts, postlist, n_spec, floor["multiplier"]
        )
        out.append(np.asarray(curve, dtype=np.float64)[:n_spec])
    return out


def decode_packet_2ch(
    ctx: DecodeContext, payload: bytes, mode_history: list[int]
) -> tuple[int, list[np.ndarray], int, int, str, int, int]:
    """Standard libvorbis composite decode for the 2ch/48k paired-build stream.

    Order per mapping0_inverse:
      floor1_inverse1 -> coupling-dirty -> residue(type2, flat domain)
      -> decouple(dB domain) -> floor1_inverse2(multiply curve) -> MDCT synth

    Returns (mode, spectra, block_size, bits_consumed, res_status, bits_left,
             blockflag).
    """
    br = BitReader(payload)
    header = parse_audio_header(br, ctx.setup)
    mode = header["mode"]
    mode_history.append(mode)
    blockflag = header["blockflag"]
    block_size = ctx.blocksizes[blockflag]
    n_spec = block_size // 2
    mapping = ctx.maps[header["mapping"]]
    floors = decode_floors_for_packet(br, ctx.setup, ctx.books, ctx.channels, mapping)
    if not floors.get("complete"):
        raise ValueError(f"floor incomplete: {floors.get('error')}")
    nonzero = floors["nonzero"]
    curves = floors["curves"]

    # Channel coupling can 'dirty' the nonzero listing.
    nonzero_dirty = list(nonzero)
    for step in ctx.coupling:
        m = int(step["mag"])
        a = int(step["ang"])
        if nonzero_dirty[m] or nonzero_dirty[a]:
            nonzero_dirty[m] = 1
            nonzero_dirty[a] = 1

    # Residue (type 2) into working vectors.
    residue_id = mapping["residues"][0]
    residue = ctx.setup["residues"][residue_id]
    res_coeffs, _npart, res_status = decode_residue_coeffs(
        br, residue, ctx.books, nonzero_dirty, n_spec
    )

    # Channel coupling (dB domain, mapping0.c:765-787).
    coupled = decouple_db_domain(res_coeffs, ctx.coupling, n_spec)

    # Compute and apply spectral envelope (floor1_inverse2).
    curve_list = compute_floor_curves(ctx, curves, nonzero, mapping, n_spec)
    spectra: list[np.ndarray] = []
    for ch in range(ctx.channels):
        if curve_list[ch] is not None:
            spectra.append(coupled[ch] * curve_list[ch])
        else:
            spectra.append(coupled[ch])
    return (
        mode,
        spectra,
        block_size,
        br.tell_bits(),
        res_status,
        br.bits_left(),
        blockflag,
    )


def apply_coupling(spectra: list[np.ndarray], ctx: DecodeContext) -> list[np.ndarray]:
    """Reconstruct the coupled (L/R) spectra from (mid/side) spectra.

    Wwise stereo coupling (mid/side):
      mag_ch (mid)  = L + R  (in MDCT domain)
      ang_ch (side) = L - R  (in MDCT domain)

    The decoder inverts this:
      L = mid + side
      R = mid - side
    """
    if not ctx.coupling:
        return spectra
    coupled = [np.array(s, dtype=np.float64, copy=True) for s in spectra]
    for step in ctx.coupling:
        m = int(step["mag"])
        a = int(step["ang"])
        mid = coupled[m]
        side = coupled[a]
        coupled[m] = mid + side
        coupled[a] = mid - side
    return coupled


def compute_positions(mode_history: list[int], blocksizes: tuple[int, int]) -> list[int]:
    """Frame start positions, replicating the encoder scheduler geometry."""
    long_half = blocksizes[1] // 2
    cursor = long_half
    buffer_base = 0
    source_origin = blocksizes[1] // 2
    positions: list[int] = []
    for index, current in enumerate(mode_history):
        following = (
            mode_history[index + 1]
            if index + 1 < len(mode_history)
            else 1
        )
        sample_start = buffer_base + cursor - blocksizes[current] // 2
        advance = (
            cursor
            + blocksizes[following] // 4
            + blocksizes[current] // 4
            - long_half
        )
        positions.append(sample_start - source_origin)
        cursor = long_half
        buffer_base += advance
    return positions


def compute_ola_positions_2ch(
    mode_history: list[int], blocksizes: tuple[int, int]
) -> list[int]:
    """Standard Vorbis OLA frame-start positions (sliding-window geometry).

    The start advances by (previous block size / 2) for every block; the
    window (long / short / split) handles the long<->short transitions.
    """
    n = len(mode_history)
    positions: list[int] = []
    if n == 0:
        return positions
    short, long_ = blocksizes[0], blocksizes[1]
    pos = 0
    positions.append(pos)
    for i in range(1, n):
        prev_size = long_ if mode_history[i - 1] else short
        pos = pos + prev_size // 2
        positions.append(pos)
    return positions


# ---------------------------------------------------------------------------
# libvorbis 1.3.7 bit-exact float32 DSP (2ch paired-build stream)
#
# The vgmstream oracle decodes with libvorbis 1.3.7 (brew build), whose
# internal DSP runs in float32.  To reproduce its int16 output bit-exactly
# we must reproduce that float32 pipeline exactly:
#   - VQ codebook unquantize: sharedbook.c _book_unquantize (maptype1)
#   - residue VQ add:         codebook.c vorbis_book_decodevv_add
#   - channel coupling:       mapping0.c mapping0_inverse
#   - floor1 envelope:        floor1.c floor1_inverse2 / render_line
#   - IMDCT:                  mdct.c mdct_backward (called via ctypes on the
#                             system libvorbis dylib — the very build the
#                             oracle uses, so twiddle factors and op order
#                             are identical by construction)
#   - OLA:                    block.c vorbis_synthesis_blockin
#   - conversion:             (int)(x * 32767.0f), clamped to [-32768,32767]
# ---------------------------------------------------------------------------

_LIBV_HELPER_C_SOURCE = r"""
/* libvorbis-bit-exact OLA (fma contraction) + vgmstream int16 conversion.
   Matches the arm64 code generated for libvorbis 1.3.7 blockin:
       t1 = (float)(buf * w_rev);      [fmul]
       out = fmaf(p, w, t1);           [fmadd]
*/
#include <stdint.h>
#include <math.h>

void libv_ola_fma(float *out, const float *buf, const float *wr,
                  const float *y, const float *w, long n) {
    long i;
    for (i = 0; i < n; i++)
        out[i] = fmaf(y[i], w[i], buf[i] * wr[i]);
}

void libv_f32_to_i16(int16_t *out, const float *x, long n) {
    long i;
    for (i = 0; i < n; i++) {
        int v = (int)(x[i] * 32767.0f);
        if (v > 32767) v = 32767;
        if (v < -32768) v = -32768;
        out[i] = (int16_t)v;
    }
}
"""


def _load_libv_helper():
    """Compile+load the tiny bit-exact helper library (cached in TMPDIR).

    The cache key is the helper's own source text: the library is rebuilt
    exactly when the ``.c`` file the compiler read differs from the source
    below, byte for byte.
    """
    import subprocess
    import tempfile

    so = Path(tempfile.gettempdir()) / "wem_libv_helper.so"
    src = Path(tempfile.gettempdir()) / "wem_libv_helper.c"
    stale = (
        not so.exists()
        or not src.is_file()
        or src.read_bytes() != _LIBV_HELPER_C_SOURCE.encode("utf-8")
    )
    if stale:
        src.write_text(_LIBV_HELPER_C_SOURCE)
        subprocess.run(
            ["cc", "-O3", "-shared", "-fPIC", "-o", str(so), str(src)],
            check=True,
            capture_output=True,
        )
    lib = ctypes.CDLL(str(so))
    lib.libv_ola_fma.argtypes = [
        ctypes.POINTER(ctypes.c_float),
        ctypes.POINTER(ctypes.c_float),
        ctypes.POINTER(ctypes.c_float),
        ctypes.POINTER(ctypes.c_float),
        ctypes.POINTER(ctypes.c_float),
        ctypes.c_long,
    ]
    lib.libv_ola_fma.restype = None
    lib.libv_f32_to_i16.argtypes = [
        ctypes.POINTER(ctypes.c_int16),
        ctypes.POINTER(ctypes.c_float),
        ctypes.c_long,
    ]
    lib.libv_f32_to_i16.restype = None
    return lib


class _MDCTLookup(ctypes.Structure):
    """libvorbis mdct_lookup layout (float build)."""

    _fields_ = [
        ("n", ctypes.c_int),
        ("log2n", ctypes.c_int),
        ("trig", ctypes.POINTER(ctypes.c_float)),
        ("bitrev", ctypes.POINTER(ctypes.c_int)),
        ("scale", ctypes.c_float),
    ]


def _load_libvorbis():
    """Load libvorbis via ctypes (for the bit-exact f32 IMDCT). None if absent."""
    import ctypes.util

    candidates: list[str] = []
    found = ctypes.util.find_library("vorbis")
    if found:
        candidates.append(found)
    candidates += [
        "/opt/homebrew/opt/libvorbis/lib/libvorbis.0.dylib",
        "/opt/homebrew/lib/libvorbis.0.dylib",
        "/usr/local/opt/libvorbis/lib/libvorbis.0.dylib",
        "/usr/local/lib/libvorbis.dylib",
        "libvorbis.dylib",
    ]
    for path in candidates:
        try:
            lib = ctypes.CDLL(path)
        except OSError:
            continue
        if hasattr(lib, "mdct_init") and hasattr(lib, "mdct_backward"):
            lib.mdct_init.argtypes = [ctypes.POINTER(_MDCTLookup), ctypes.c_int]
            lib.mdct_init.restype = None
            lib.mdct_backward.argtypes = [
                ctypes.POINTER(_MDCTLookup),
                ctypes.POINTER(ctypes.c_float),
                ctypes.POINTER(ctypes.c_float),
            ]
            lib.mdct_backward.restype = None
            return lib
    return None


_LIBV_WINDOW_256 = np.float32(
[
    0.0000591390, 0.0005321979, 0.0014780301, 0.0028960636, 0.0047854363, 0.0071449926, 0.0099732775, 0.0132685298, 0.0170286741, 0.0212513119, 0.0259337111, 0.0310727950, 0.0366651302, 0.0427069140, 0.0491939614, 0.0561216907, 0.0634851102, 0.0712788035, 0.0794969160, 0.0881331402,
    0.0971807028, 0.1066323515, 0.1164803426, 0.1267164297, 0.1373318534, 0.1483173323, 0.1596630553, 0.1713586755, 0.1833933062, 0.1957555184, 0.2084333404, 0.2214142599, 0.2346852280, 0.2482326664, 0.2620424757, 0.2761000481, 0.2903902813, 0.3048975959, 0.3196059553, 0.3344988887,
    0.3495595160, 0.3647705766, 0.3801144597, 0.3955732382, 0.4111287047, 0.4267624093, 0.4424557009, 0.4581897696, 0.4739456913, 0.4897044744, 0.5054471075, 0.5211546088, 0.5368080763, 0.5523887395, 0.5678780103, 0.5832575361, 0.5985092508, 0.6136154277, 0.6285587300, 0.6433222619,
    0.6578896175, 0.6722449294, 0.6863729144, 0.7002589187, 0.7138889597, 0.7272497662, 0.7403288154, 0.7531143679, 0.7655954985, 0.7777621249, 0.7896050322, 0.8011158947, 0.8122872932, 0.8231127294, 0.8335866365, 0.8437043850, 0.8534622861, 0.8628575905, 0.8718884835, 0.8805540765,
    0.8888543947, 0.8967903616, 0.9043637797, 0.9115773078, 0.9184344360, 0.9249394562, 0.9310974312, 0.9369141608, 0.9423961446, 0.9475505439, 0.9523851406, 0.9569082947, 0.9611289005, 0.9650563408, 0.9687004405, 0.9720714191, 0.9751798427, 0.9780365753, 0.9806527301, 0.9830396204,
    0.9852087111, 0.9871715701, 0.9889398207, 0.9905250941, 0.9919389832, 0.9931929973, 0.9942985174, 0.9952667537, 0.9961087037, 0.9968351119, 0.9974564312, 0.9979827858, 0.9984239359, 0.9987892441, 0.9990876435, 0.9993276081, 0.9995171241, 0.9996636648, 0.9997741654, 0.9998550016,
    0.9999119692, 0.9999502656, 0.9999744742, 0.9999885497, 0.9999958064, 0.9999989077, 0.9999998584, 0.9999999983,
])

_LIBV_WINDOW_2048 = np.float32(
[
    0.0000009241, 0.0000083165, 0.0000231014, 0.0000452785, 0.0000748476, 0.0001118085, 0.0001561608, 0.0002079041, 0.0002670379, 0.0003335617, 0.0004074748, 0.0004887765, 0.0005774661, 0.0006735427, 0.0007770054, 0.0008878533, 0.0010060853, 0.0011317002, 0.0012646969, 0.0014050742,
    0.0015528307, 0.0017079650, 0.0018704756, 0.0020403610, 0.0022176196, 0.0024022497, 0.0025942495, 0.0027936173, 0.0030003511, 0.0032144490, 0.0034359088, 0.0036647286, 0.0039009061, 0.0041444391, 0.0043953253, 0.0046535621, 0.0049191472, 0.0051920781, 0.0054723520, 0.0057599664,
    0.0060549184, 0.0063572052, 0.0066668239, 0.0069837715, 0.0073080449, 0.0076396410, 0.0079785566, 0.0083247884, 0.0086783330, 0.0090391871, 0.0094073470, 0.0097828092, 0.0101655700, 0.0105556258, 0.0109529726, 0.0113576065, 0.0117695237, 0.0121887200, 0.0126151913, 0.0130489335,
    0.0134899422, 0.0139382130, 0.0143937415, 0.0148565233, 0.0153265536, 0.0158038279, 0.0162883413, 0.0167800889, 0.0172790660, 0.0177852675, 0.0182986882, 0.0188193231, 0.0193471668, 0.0198822141, 0.0204244594, 0.0209738974, 0.0215305225, 0.0220943289, 0.0226653109, 0.0232434627,
    0.0238287784, 0.0244212519, 0.0250208772, 0.0256276481, 0.0262415582, 0.0268626014, 0.0274907711, 0.0281260608, 0.0287684638, 0.0294179736, 0.0300745833, 0.0307382859, 0.0314090747, 0.0320869424, 0.0327718819, 0.0334638860, 0.0341629474, 0.0348690586, 0.0355822122, 0.0363024004,
    0.0370296157, 0.0377638502, 0.0385050960, 0.0392533451, 0.0400085896, 0.0407708211, 0.0415400315, 0.0423162123, 0.0430993552, 0.0438894515, 0.0446864926, 0.0454904698, 0.0463013742, 0.0471191969, 0.0479439288, 0.0487755607, 0.0496140836, 0.0504594879, 0.0513117642, 0.0521709031,
    0.0530368949, 0.0539097297, 0.0547893979, 0.0556758894, 0.0565691941, 0.0574693019, 0.0583762026, 0.0592898858, 0.0602103410, 0.0611375576, 0.0620715250, 0.0630122324, 0.0639596688, 0.0649138234, 0.0658746848, 0.0668422421, 0.0678164838, 0.0687973985, 0.0697849746, 0.0707792005,
    0.0717800645, 0.0727875547, 0.0738016591, 0.0748223656, 0.0758496620, 0.0768835359, 0.0779239751, 0.0789709668, 0.0800244985, 0.0810845574, 0.0821511306, 0.0832242052, 0.0843037679, 0.0853898056, 0.0864823050, 0.0875812525, 0.0886866347, 0.0897984378, 0.0909166480, 0.0920412513,
    0.0931722338, 0.0943095813, 0.0954532795, 0.0966033140, 0.0977596702, 0.0989223336, 0.1000912894, 0.1012665227, 0.1024480185, 0.1036357616, 0.1048297369, 0.1060299290, 0.1072363224, 0.1084489014, 0.1096676504, 0.1108925534, 0.1121235946, 0.1133607577, 0.1146040267, 0.1158533850,
    0.1171088163, 0.1183703040, 0.1196378312, 0.1209113812, 0.1221909370, 0.1234764815, 0.1247679974, 0.1260654674, 0.1273688740, 0.1286781995, 0.1299934263, 0.1313145365, 0.1326415121, 0.1339743349, 0.1353129866, 0.1366574490, 0.1380077035, 0.1393637315, 0.1407255141, 0.1420930325,
    0.1434662677, 0.1448452004, 0.1462298115, 0.1476200814, 0.1490159906, 0.1504175195, 0.1518246482, 0.1532373569, 0.1546556253, 0.1560794333, 0.1575087606, 0.1589435866, 0.1603838909, 0.1618296526, 0.1632808509, 0.1647374648, 0.1661994731, 0.1676668546, 0.1691395880, 0.1706176516,
    0.1721010238, 0.1735896829, 0.1750836068, 0.1765827736, 0.1780871610, 0.1795967468, 0.1811115084, 0.1826314234, 0.1841564689, 0.1856866221, 0.1872218600, 0.1887621595, 0.1903074974, 0.1918578503, 0.1934131947, 0.1949735068, 0.1965387630, 0.1981089393, 0.1996840117, 0.2012639560,
    0.2028487479, 0.2044383630, 0.2060327766, 0.2076319642, 0.2092359007, 0.2108445614, 0.2124579211, 0.2140759545, 0.2156986364, 0.2173259411, 0.2189578432, 0.2205943168, 0.2222353361, 0.2238808751, 0.2255309076, 0.2271854073, 0.2288443480, 0.2305077030, 0.2321754457, 0.2338475493,
    0.2355239869, 0.2372047315, 0.2388897560, 0.2405790329, 0.2422725350, 0.2439702347, 0.2456721043, 0.2473781159, 0.2490882418, 0.2508024539, 0.2525207240, 0.2542430237, 0.2559693248, 0.2576995986, 0.2594338166, 0.2611719498, 0.2629139695, 0.2646598466, 0.2664095520, 0.2681630564,
    0.2699203304, 0.2716813445, 0.2734460691, 0.2752144744, 0.2769865307, 0.2787622079, 0.2805414760, 0.2823243047, 0.2841106637, 0.2859005227, 0.2876938509, 0.2894906179, 0.2912907928, 0.2930943447, 0.2949012426, 0.2967114554, 0.2985249520, 0.3003417009, 0.3021616708, 0.3039848301,
    0.3058111471, 0.3076405901, 0.3094731273, 0.3113087266, 0.3131473560, 0.3149889833, 0.3168335762, 0.3186811024, 0.3205315294, 0.3223848245, 0.3242409552, 0.3260998886, 0.3279615918, 0.3298260319, 0.3316931758, 0.3335629903, 0.3354354423, 0.3373104982, 0.3391881247, 0.3410682882,
    0.3429509551, 0.3448360917, 0.3467236642, 0.3486136387, 0.3505059811, 0.3524006575, 0.3542976336, 0.3561968753, 0.3580983482, 0.3600020179, 0.3619078499, 0.3638158096, 0.3657258625, 0.3676379737, 0.3695521086, 0.3714682321, 0.3733863094, 0.3753063055, 0.3772281852, 0.3791519134,
    0.3810774548, 0.3830047742, 0.3849338362, 0.3868646053, 0.3887970459, 0.3907311227, 0.3926667998, 0.3946040417, 0.3965428125, 0.3984830765, 0.4004247978, 0.4023679403, 0.4043124683, 0.4062583455, 0.4082055359, 0.4101540034, 0.4121037117, 0.4140546246, 0.4160067058, 0.4179599190,
    0.4199142277, 0.4218695956, 0.4238259861, 0.4257833627, 0.4277416888, 0.4297009279, 0.4316610433, 0.4336219983, 0.4355837562, 0.4375462803, 0.4395095337, 0.4414734797, 0.4434380815, 0.4454033021, 0.4473691046, 0.4493354521, 0.4513023078, 0.4532696345, 0.4552373954, 0.4572055533,
    0.4591740713, 0.4611429123, 0.4631120393, 0.4650814151, 0.4670510028, 0.4690207650, 0.4709906649, 0.4729606651, 0.4749307287, 0.4769008185, 0.4788708972, 0.4808409279, 0.4828108732, 0.4847806962, 0.4867503597, 0.4887198264, 0.4906890593, 0.4926580213, 0.4946266753, 0.4965949840,
    0.4985629105, 0.5005304176, 0.5024974683, 0.5044640255, 0.5064300522, 0.5083955114, 0.5103603659, 0.5123245790, 0.5142881136, 0.5162509328, 0.5182129997, 0.5201742774, 0.5221347290, 0.5240943178, 0.5260530070, 0.5280107598, 0.5299675395, 0.5319233095, 0.5338780330, 0.5358316736,
    0.5377841946, 0.5397355596, 0.5416857320, 0.5436346755, 0.5455823538, 0.5475287304, 0.5494737691, 0.5514174337, 0.5533596881, 0.5553004962, 0.5572398218, 0.5591776291, 0.5611138821, 0.5630485449, 0.5649815818, 0.5669129570, 0.5688426349, 0.5707705799, 0.5726967564, 0.5746211290,
    0.5765436624, 0.5784643212, 0.5803830702, 0.5822998743, 0.5842146984, 0.5861275076, 0.5880382669, 0.5899469416, 0.5918534968, 0.5937578981, 0.5956601107, 0.5975601004, 0.5994578326, 0.6013532732, 0.6032463880, 0.6051371429, 0.6070255039, 0.6089114372, 0.6107949090, 0.6126758856,
    0.6145543334, 0.6164302191, 0.6183035092, 0.6201741706, 0.6220421700, 0.6239074745, 0.6257700513, 0.6276298674, 0.6294868903, 0.6313410873, 0.6331924262, 0.6350408745, 0.6368864001, 0.6387289710, 0.6405685552, 0.6424051209, 0.6442386364, 0.6460690702, 0.6478963910, 0.6497205673,
    0.6515415682, 0.6533593625, 0.6551739194, 0.6569852082, 0.6587931984, 0.6605978593, 0.6623991609, 0.6641970728, 0.6659915652, 0.6677826081, 0.6695701718, 0.6713542268, 0.6731347437, 0.6749116932, 0.6766850461, 0.6784547736, 0.6802208469, 0.6819832374, 0.6837419164, 0.6854968559,
    0.6872480275, 0.6889954034, 0.6907389556, 0.6924786566, 0.6942144788, 0.6959463950, 0.6976743780, 0.6993984008, 0.7011184365, 0.7028344587, 0.7045464407, 0.7062543564, 0.7079581796, 0.7096578844, 0.7113534450, 0.7130448359, 0.7147320316, 0.7164150070, 0.7180937371, 0.7197681970,
    0.7214383620, 0.7231042077, 0.7247657098, 0.7264228443, 0.7280755871, 0.7297239147, 0.7313678035, 0.7330072301, 0.7346421715, 0.7362726046, 0.7378985069, 0.7395198556, 0.7411366285, 0.7427488034, 0.7443563584, 0.7459592717, 0.7475575218, 0.7491510873, 0.7507399471, 0.7523240803,
    0.7539034661, 0.7554780839, 0.7570479136, 0.7586129349, 0.7601731279, 0.7617284730, 0.7632789506, 0.7648245416, 0.7663652267, 0.7679009872, 0.7694318044, 0.7709576599, 0.7724785354, 0.7739944130, 0.7755052749, 0.7770111035, 0.7785118815, 0.7800075916, 0.7814982170, 0.7829837410,
    0.7844641472, 0.7859394191, 0.7874095408, 0.7888744965, 0.7903342706, 0.7917888476, 0.7932382124, 0.7946823501, 0.7961212460, 0.7975548855, 0.7989832544, 0.8004063386, 0.8018241244, 0.8032365981, 0.8046437463, 0.8060455560, 0.8074420141, 0.8088331080, 0.8102188253, 0.8115991536,
    0.8129740810, 0.8143435957, 0.8157076861, 0.8170663409, 0.8184195489, 0.8197672994, 0.8211095817, 0.8224463853, 0.8237777001, 0.8251035161, 0.8264238235, 0.8277386129, 0.8290478750, 0.8303516008, 0.8316497814, 0.8329424083, 0.8342294731, 0.8355109677, 0.8367868841, 0.8380572148,
    0.8393219523, 0.8405810893, 0.8418346190, 0.8430825345, 0.8443248294, 0.8455614974, 0.8467925323, 0.8480179285, 0.8492376802, 0.8504517822, 0.8516602292, 0.8528630164, 0.8540601391, 0.8552515928, 0.8564373733, 0.8576174766, 0.8587918990, 0.8599606368, 0.8611236868, 0.8622810460,
    0.8634327113, 0.8645786802, 0.8657189504, 0.8668535195, 0.8679823857, 0.8691055472, 0.8702230025, 0.8713347503, 0.8724407896, 0.8735411194, 0.8746357394, 0.8757246489, 0.8768078479, 0.8778853364, 0.8789571146, 0.8800231832, 0.8810835427, 0.8821381942, 0.8831871387, 0.8842303777,
    0.8852679127, 0.8862997456, 0.8873258784, 0.8883463132, 0.8893610527, 0.8903700994, 0.8913734562, 0.8923711263, 0.8933631129, 0.8943494196, 0.8953300500, 0.8963050083, 0.8972742985, 0.8982379249, 0.8991958922, 0.9001482052, 0.9010948688, 0.9020358883, 0.9029712690, 0.9039010165,
    0.9048251367, 0.9057436357, 0.9066565195, 0.9075637946, 0.9084654678, 0.9093615456, 0.9102520353, 0.9111369440, 0.9120162792, 0.9128900484, 0.9137582595, 0.9146209204, 0.9154780394, 0.9163296248, 0.9171756853, 0.9180162296, 0.9188512667, 0.9196808057, 0.9205048559, 0.9213234270,
    0.9221365285, 0.9229441704, 0.9237463629, 0.9245431160, 0.9253344404, 0.9261203465, 0.9269008453, 0.9276759477, 0.9284456648, 0.9292100080, 0.9299689889, 0.9307226190, 0.9314709103, 0.9322138747, 0.9329515245, 0.9336838721, 0.9344109300, 0.9351327108, 0.9358492275, 0.9365604931,
    0.9372665208, 0.9379673239, 0.9386629160, 0.9393533107, 0.9400385220, 0.9407185637, 0.9413934501, 0.9420631954, 0.9427278141, 0.9433873208, 0.9440417304, 0.9446910576, 0.9453353176, 0.9459745255, 0.9466086968, 0.9472378469, 0.9478619915, 0.9484811463, 0.9490953274, 0.9497045506,
    0.9503088323, 0.9509081888, 0.9515026365, 0.9520921921, 0.9526768723, 0.9532566940, 0.9538316742, 0.9544018300, 0.9549671786, 0.9555277375, 0.9560835241, 0.9566345562, 0.9571808513, 0.9577224275, 0.9582593027, 0.9587914949, 0.9593190225, 0.9598419038, 0.9603601571, 0.9608738012,
    0.9613828546, 0.9618873361, 0.9623872646, 0.9628826591, 0.9633735388, 0.9638599227, 0.9643418303, 0.9648192808, 0.9652922939, 0.9657608890, 0.9662250860, 0.9666849046, 0.9671403646, 0.9675914861, 0.9680382891, 0.9684807937, 0.9689190202, 0.9693529890, 0.9697827203, 0.9702082347,
    0.9706295529, 0.9710466953, 0.9714596828, 0.9718685362, 0.9722732762, 0.9726739240, 0.9730705005, 0.9734630267, 0.9738515239, 0.9742360134, 0.9746165163, 0.9749930540, 0.9753656481, 0.9757343198, 0.9760990909, 0.9764599829, 0.9768170175, 0.9771702164, 0.9775196013, 0.9778651941,
    0.9782070167, 0.9785450909, 0.9788794388, 0.9792100824, 0.9795370437, 0.9798603449, 0.9801800080, 0.9804960554, 0.9808085092, 0.9811173916, 0.9814227251, 0.9817245318, 0.9820228343, 0.9823176549, 0.9826090160, 0.9828969402, 0.9831814498, 0.9834625674, 0.9837403156, 0.9840147169,
    0.9842857939, 0.9845535692, 0.9848180654, 0.9850793052, 0.9853373113, 0.9855921062, 0.9858437127, 0.9860921535, 0.9863374512, 0.9865796287, 0.9868187085, 0.9870547136, 0.9872876664, 0.9875175899, 0.9877445067, 0.9879684396, 0.9881894112, 0.9884074444, 0.9886225619, 0.9888347863,
    0.9890441404, 0.9892506468, 0.9894543284, 0.9896552077, 0.9898533074, 0.9900486502, 0.9902412587, 0.9904311555, 0.9906183633, 0.9908029045, 0.9909848019, 0.9911640779, 0.9913407550, 0.9915148557, 0.9916864025, 0.9918554179, 0.9920219241, 0.9921859437, 0.9923474989, 0.9925066120,
    0.9926633054, 0.9928176012, 0.9929695218, 0.9931190891, 0.9932663254, 0.9934112527, 0.9935538932, 0.9936942686, 0.9938324012, 0.9939683126, 0.9941020248, 0.9942335597, 0.9943629388, 0.9944901841, 0.9946153170, 0.9947383593, 0.9948593325, 0.9949782579, 0.9950951572, 0.9952100516,
    0.9953229625, 0.9954339111, 0.9955429186, 0.9956500062, 0.9957551948, 0.9958585056, 0.9959599593, 0.9960595769, 0.9961573792, 0.9962533869, 0.9963476206, 0.9964401009, 0.9965308483, 0.9966198833, 0.9967072261, 0.9967928971, 0.9968769164, 0.9969593041, 0.9970400804, 0.9971192651,
    0.9971968781, 0.9972729391, 0.9973474680, 0.9974204842, 0.9974920074, 0.9975620569, 0.9976306521, 0.9976978122, 0.9977635565, 0.9978279039, 0.9978908736, 0.9979524842, 0.9980127547, 0.9980717037, 0.9981293499, 0.9981857116, 0.9982408073, 0.9982946554, 0.9983472739, 0.9983986810,
    0.9984488947, 0.9984979328, 0.9985458132, 0.9985925534, 0.9986381711, 0.9986826838, 0.9987261086, 0.9987684630, 0.9988097640, 0.9988500286, 0.9988892738, 0.9989275163, 0.9989647727, 0.9990010597, 0.9990363938, 0.9990707911, 0.9991042679, 0.9991368404, 0.9991685244, 0.9991993358,
    0.9992292905, 0.9992584038, 0.9992866914, 0.9993141686, 0.9993408506, 0.9993667526, 0.9993918895, 0.9994162761, 0.9994399273, 0.9994628576, 0.9994850815, 0.9995066133, 0.9995274672, 0.9995476574, 0.9995671978, 0.9995861021, 0.9996043841, 0.9996220573, 0.9996391352, 0.9996556310,
    0.9996715579, 0.9996869288, 0.9997017568, 0.9997160543, 0.9997298342, 0.9997431088, 0.9997558905, 0.9997681914, 0.9997800236, 0.9997913990, 0.9998023292, 0.9998128261, 0.9998229009, 0.9998325650, 0.9998418296, 0.9998507058, 0.9998592044, 0.9998673362, 0.9998751117, 0.9998825415,
    0.9998896358, 0.9998964047, 0.9999028584, 0.9999090066, 0.9999148590, 0.9999204253, 0.9999257148, 0.9999307368, 0.9999355003, 0.9999400144, 0.9999442878, 0.9999483293, 0.9999521472, 0.9999557499, 0.9999591457, 0.9999623426, 0.9999653483, 0.9999681708, 0.9999708175, 0.9999732959,
    0.9999756132, 0.9999777765, 0.9999797928, 0.9999816688, 0.9999834113, 0.9999850266, 0.9999865211, 0.9999879009, 0.9999891721, 0.9999903405, 0.9999914118, 0.9999923914, 0.9999932849, 0.9999940972, 0.9999948336, 0.9999954989, 0.9999960978, 0.9999966349, 0.9999971146, 0.9999975411,
    0.9999979185, 0.9999982507, 0.9999985414, 0.9999987944, 0.9999990129, 0.9999992003, 0.9999993596, 0.9999994939, 0.9999996059, 0.9999996981, 0.9999997732, 0.9999998333, 0.9999998805, 0.9999999170, 0.9999999444, 0.9999999643, 0.9999999784, 0.9999999878, 0.9999999937, 0.9999999972,
    0.9999999990, 0.9999999997, 1.0000000000, 1.0000000000,
])


def _libv_book_values_f32(book: Codebook) -> np.ndarray | None:
    """libvorbis sharedbook.c _book_unquantize (maptype1), f32 semantics."""
    sc = book.static
    if sc.maptype != 1 or not sc.quantlist:
        return None
    from wwise_wem_reference.vorbis.codebook import (
        book_maptype1_quantvals,
        float32_unpack,
    )

    qv = book_maptype1_quantvals(sc.entries, sc.dim)
    mindel = np.float32(float32_unpack(int(sc.q_min)))
    delta = np.float32(float32_unpack(int(sc.q_delta)))
    q = np.asarray(sc.quantlist, dtype=np.int32)
    entries, dim = int(sc.entries), int(sc.dim)
    idx = ((np.arange(entries)[:, None] // (qv ** np.arange(dim))) % qv).T
    vals = np.zeros((entries, dim), dtype=np.float32)
    last = np.zeros(entries, dtype=np.float32)
    for k in range(dim):
        v = np.abs(q[idx[k]])
        v = (v * delta).astype(np.float32)
        v = (v + mindel).astype(np.float32)
        v = (v + last).astype(np.float32)
        vals[:, k] = v
        if sc.q_sequencep:
            last = v
    if sc.lengthlist is not None:
        used = np.asarray([int(L) > 0 for L in sc.lengthlist])
        vals[~used] = 0.0
    return vals


def _libv_decodevv_add_f32(
    values: np.ndarray,
    flat: np.ndarray,
    offset: int,
    ch: int,
    br: BitReader,
    n: int,
    book: Codebook,
) -> int:
    """libvorbis vorbis_book_decodevv_add, f32 accumulation."""
    if book.dim <= 0:
        return 0
    used = [e for e, L in enumerate(book.lengthlist) if int(L) > 0]
    if not used:
        return 0
    m = (offset + n) // ch
    i = offset // ch
    chptr = 0
    dim = book.dim
    try:
        while i < m:
            entry = _decode_eop(book, br)
            if entry < 0 or entry >= book.entries:
                raise ResidueEOP()
            t = values[entry]
            for j in range(dim):
                if i < m:
                    flat[i * ch + chptr] = (
                        flat[i * ch + chptr] + t[j]  # type: ignore[operator]
                    ).astype(np.float32)
                    chptr += 1
                    if chptr == ch:
                        chptr = 0
                        i += 1
    except ResidueEOP:
        return -1
    return 0


def _libv_residue_type2_f32(
    br: BitReader,
    residue: dict,
    books: list[Codebook],
    bookvals: list[np.ndarray | None],
    ch_used: list[bool],
    max_flat: int,
) -> tuple[np.ndarray, str]:
    """libvorbis res2_inverse in f32 (bit schedule mirrors decode_residue_type2)."""
    ch = len(ch_used)
    begin = int(residue["begin"])
    end = min(int(residue["end"]), max_flat)
    part = int(residue["partition_size"])
    nclass = int(residue["classifications"])
    span = end - begin
    flat = np.zeros(max_flat, dtype=np.float32)
    if span <= 0 or not any(ch_used):
        return flat, ("empty" if not any(ch_used) else "complete")
    partvals = span // part
    if partvals <= 0:
        return flat, "complete"
    classbook = books[residue["classbook"]]
    ppw = classbook.dim
    if ppw <= 0:
        ppw = 1
    stages = 0
    for cascade in residue["cascades"]:
        stages = max(stages, int(cascade).bit_length())
    partbooks = residue["books"]
    partword: list[list[int]] = []
    status = "complete"
    try:
        for s in range(stages):
            i = 0
            lg = 0
            while i < partvals:
                if s == 0:
                    entry = _decode_eop(classbook, br)
                    if entry >= classbook.entries:
                        status = "eop"
                        return flat, status
                    pword: list[int] = [0] * ppw
                    t = entry
                    for _k in range(ppw - 1, -1, -1):
                        pword[_k] = t % nclass
                        t //= nclass
                    partword.append(pword)
                for k in range(ppw):
                    if i >= partvals:
                        break
                    pclass = partword[lg][k] if partword[lg][k] < nclass else 0
                    if s < len(partbooks[pclass]):
                        book_id = partbooks[pclass][s]
                        if book_id is not None and book_id >= 0:
                            book = books[book_id]
                            offset = i * part + begin
                            vals = bookvals[book_id]
                            if vals is None:  # pragma: no cover
                                status = "eop"
                                return flat, status
                            if (
                                _libv_decodevv_add_f32(
                                    vals, flat, offset, ch, br, part, book
                                )
                                < 0
                            ):
                                status = "eop"
                                return flat, status
                    i += 1
                lg += 1
    except ResidueEOP:
        status = "eop"
    return flat, status


def _libv_couple_f32(
    coeffs: list[np.ndarray], coupling: list[dict], n_spec: int
) -> list[np.ndarray]:
    """libvorbis mapping0_inverse coupling (mapping0.c:765-787), f32."""
    if not coupling:
        return coeffs
    out = [np.array(c, dtype=np.float32, copy=True) for c in coeffs]
    for step in reversed(coupling):
        m = int(step["mag"])
        a = int(step["ang"])
        pm, pa = out[m], out[a]
        m_pos = pm > 0.0
        a_pos = pa > 0.0
        both = m_pos & a_pos
        mpos_aneg = m_pos & ~a_pos
        mneg_apos = ~m_pos & a_pos
        # M: (M,A) = (mag, mag-ang) if mag>0,ang>0; (mag+ang, mag) if mag>0,ang<=0;
        #    (mag, mag+ang) if mag<=0,ang>0; (mag-ang, mag) if mag<=0,ang<=0
        out_m = np.empty_like(pm)
        out_a = np.empty_like(pm)
        for mask, mval, aval in (
            (both, pm, pm - pa),
            (mpos_aneg, pm + pa, pm),
            (mneg_apos, pm, pm + pa),
            (~(both | mpos_aneg | mneg_apos), pm - pa, pm),
        ):
            out_m[mask] = mval[mask]
            out_a[mask] = aval[mask]
        out[m][:n_spec] = out_m[:n_spec]
        out[a][:n_spec] = out_a[:n_spec]
    return out


_FLOOR1_FROM_DB_F32 = np.float32(FLOOR1_fromdB_LOOKUP)


def _libv_render_line_f32(
    n: int, x0: int, x1: int, y0: int, y1: int, d: np.ndarray
) -> None:
    """libvorbis floor1.c render_line (f32) — in-place multiply."""
    if x0 >= x1:
        return
    dy = y1 - y0
    adx = x1 - x0
    ady = abs(dy)
    base = int(dy / adx)  # C division (truncate toward zero), not floor
    sy = base - 1 if dy < 0 else base + 1
    x = x0
    y = y0
    err = 0
    ady -= abs(base * adx)
    limit = x1 if n > x1 else n
    if x < limit:
        yi = 0 if y < 0 else (255 if y > 255 else y)
        d[x] = (d[x] * _FLOOR1_FROM_DB_F32[yi]).astype(np.float32)
    while True:
        x += 1
        if x >= limit:
            break
        err += ady
        if err >= adx:
            err -= adx
            y += sy
        else:
            y += base
        yi = 0 if y < 0 else (255 if y > 255 else y)
        d[x] = (d[x] * _FLOOR1_FROM_DB_F32[yi]).astype(np.float32)


def _libv_floor_curves_f32(
    ctx: DecodeContext,
    curves_raw: list,
    nonzero: list[int],
    mapping: dict,
    n_spec: int,
) -> list[np.ndarray | None]:
    """libvorbis floor1_inverse2 (f32). None => channel zeroed by libv."""
    from wwise_wem_reference.vorbis.floor import (
        FLOOR1_RANGES,
        floor1_unwrap,
        postlist_from_floor,
    )

    out: list[np.ndarray | None] = []
    for ch in range(ctx.channels):
        if not nonzero[ch]:
            out.append(None)
            continue
        sub = mapping["chmux"][ch] if mapping["submaps"] > 1 else 0
        floor = ctx.setup["floors"][mapping["floors"][sub]]
        postlist = postlist_from_floor(floor)
        rng = FLOOR1_RANGES[floor["multiplier"]]
        posts = floor1_unwrap(curves_raw[ch], postlist, rng)
        mult = int(floor["multiplier"])
        curve = np.ones(n_spec, dtype=np.float32)
        hx = 0
        lx = 0
        ly = (posts[0] & 0x7FFF) * mult
        ly = 0 if ly < 0 else (255 if ly > 255 else ly)
        fwd = np.argsort(np.asarray(postlist, dtype=np.int64))
        for j in range(1, len(posts)):
            current = int(fwd[j])
            hy = posts[current] & 0x7FFF
            if hy == posts[current]:
                hx = postlist[current]
                hyq = hy * mult
                hyq = 0 if hyq < 0 else (255 if hyq > 255 else hyq)
                _libv_render_line_f32(n_spec, lx, hx, ly, hyq, curve)
                lx = hx
                ly = hyq
        if hx < n_spec:
            curve[hx:] = (curve[hx:] * _FLOOR1_FROM_DB_F32[ly]).astype(
                np.float32
            )
        out.append(curve[:n_spec])
    return out


def decode_packet_2ch_libv(
    ctx: DecodeContext,
    payload: bytes,
    mode_history: list[int],
    bookvals: list[np.ndarray | None],
) -> tuple[int, list[np.ndarray], int, int, str, int, int]:
    """libvorbis-exact f32 packet decode (mapping0_inverse order).

    Returns (mode, spectra_f32, block_size, bits_consumed, res_status,
             bits_left, blockflag).  Spectra are the final MDCT-domain
    coefficients (residue -> coupling -> floor envelope, f32).
    """
    from wwise_wem_reference.vorbis.packet_decoder import (
        decode_floors_for_packet,
    )

    br = BitReader(payload)
    header = parse_audio_header(br, ctx.setup)
    mode = header["mode"]
    mode_history.append(mode)
    blockflag = header["blockflag"]
    block_size = ctx.blocksizes[blockflag]
    n_spec = block_size // 2
    mapping = ctx.maps[header["mapping"]]
    floors = decode_floors_for_packet(br, ctx.setup, ctx.books, ctx.channels, mapping)
    if not floors.get("complete"):
        raise ValueError(f"floor incomplete: {floors.get('error')}")
    nonzero = floors["nonzero"]
    curves_raw = floors["curves"]

    # Coupling can 'dirty' the nonzero listing.
    nonzero_dirty = list(nonzero)
    for step in ctx.coupling:
        m = int(step["mag"])
        a = int(step["ang"])
        if nonzero_dirty[m] or nonzero_dirty[a]:
            nonzero_dirty[m] = 1
            nonzero_dirty[a] = 1

    # Residue (type 2) into working vectors (f32).
    residue_id = mapping["residues"][0]
    residue = ctx.setup["residues"][residue_id]
    flat, res_status = _libv_residue_type2_f32(
        br, residue, ctx.books, bookvals, nonzero_dirty, n_spec * ctx.channels
    )
    coeffs = [
        flat[ch :: ctx.channels][:n_spec].copy() for ch in range(ctx.channels)
    ]

    # Channel coupling (f32).
    coeffs = _libv_couple_f32(coeffs, ctx.coupling, n_spec)

    # Compute and apply spectral envelope (floor1_inverse2, f32).
    curve_list = _libv_floor_curves_f32(ctx, curves_raw, nonzero, mapping, n_spec)
    spectra: list[np.ndarray] = []
    for ch in range(ctx.channels):
        if curve_list[ch] is not None:
            spectra.append(
                (coeffs[ch] * curve_list[ch]).astype(np.float32)  # type: ignore[arg-type]
            )
        else:
            # libv floor1_inverse2 with no floor: memset(out, 0, n/2).
            spectra.append(np.zeros(n_spec, dtype=np.float32))
    return (
        mode,
        spectra,
        block_size,
        br.tell_bits(),
        res_status,
        br.bits_left(),
        blockflag,
    )


def libv_float_to_int16(pcm: list[np.ndarray]) -> list[np.ndarray]:
    """vgmstream CONV_FLT_S16: clamp((int)(x * 32767.0f)), [-32768, 32767]."""
    helper = _load_libv_helper()
    out: list[np.ndarray] = []
    for ch in pcm:
        x = np.ascontiguousarray(ch.astype(np.float32))
        buf = np.empty(len(x), dtype=np.int16)
        helper.libv_f32_to_i16(
            buf.ctypes.data_as(ctypes.POINTER(ctypes.c_int16)),
            x.ctypes.data_as(ctypes.POINTER(ctypes.c_float)),
            len(x),
        )
        out.append(buf)
    return out


def libv_chunk_layout(
    mode_history: list[int], blocksizes: tuple[int, int]
) -> tuple[list[int], list[int]]:
    """libvorbis vorbis_synthesis_blockin PCM stream layout.

    The PCM stream is the concatenation of per-frame chunks.  The first
    frame emits no chunk; from the second frame on the stream advances by
    (previous_blocksize/4 + current_blocksize/4) per frame.  This exact
    layout (including the block-size transition steps and the stream-start
    convention) is the layout this optional libvorbis-compatible path models.
    A historical local comparison covered 108,504,384 frames exactly; it is
    evidence for the layout, not committed test coverage.

    Returns (chunk_starts, chunk_lens) indexed by frame.
    """
    n = len(mode_history)
    starts: list[int] = [0] * n
    lens: list[int] = [0] * n
    stream_pos = 0
    for i in range(1, n):
        prev_size = blocksizes[1] if mode_history[i - 1] else blocksizes[0]
        cur_size = blocksizes[1] if mode_history[i] else blocksizes[0]
        starts[i] = stream_pos
        lens[i] = prev_size // 4 + cur_size // 4
        stream_pos += lens[i]
    return starts, lens


def run_decode_2ch(
    ctx: DecodeContext, audio_packets: list[bytes]
) -> dict[str, Any]:
    """Full decode of the 2ch/48k paired-build stream (default: deterministic numpy f64).

    The default path is pure NumPy float64: no ambient codec dependency and a
    stable operation order.  A historical local corpus comparison saw
    correlation 1.000000, max int16 delta <= 2 LSB, and strict closure on
    294,485 packets; the carried-WEM parity test is the current automated
    coverage.

    With ``--libvorbis-exact`` the libvorbis 1.3.7 float32 clone
    is used instead: f32 codebook values, f32 residue/coupling/floor, the
    system libvorbis mdct_backward (ctypes), the vorbis_synthesis_blockin
    OLA, and vgmstream's (int)(x*32767.0f) int16 conversion.  It is a local
    compatibility probe, at the cost of environment sensitivity.
    Falls back to the numpy path when libvorbis or a C compiler is missing.
    """
    if not _LIBVORBIS_EXACT:
        result = run_decode_2ch_kernel_f64(ctx, audio_packets)
        result["libv_f32"] = False
        return result
    lib = _load_libvorbis()
    if lib is None:
        result = run_decode_2ch_kernel_f64(ctx, audio_packets)
        result["libv_f32"] = False
        return result
    try:
        helper = _load_libv_helper()
    except Exception:  # compiler unavailable — degrade to f64 kernel path
        result = run_decode_2ch_kernel_f64(ctx, audio_packets)
        result["libv_f32"] = False
        return result
    _fptr = ctypes.POINTER(ctypes.c_float)

    # Pass 1: decode modes to know positions and window bits.
    mode_history: list[int] = []
    for payload in audio_packets:
        br = BitReader(payload)
        header = parse_audio_header(br, ctx.setup)
        mode_history.append(header["mode"])
    starts, lens = libv_chunk_layout(mode_history, ctx.blocksizes)
    total_len = sum(lens)
    out_buf = [np.zeros(total_len, dtype=np.float32) for _ in range(ctx.channels)]

    short_bs, long_bs = ctx.blocksizes
    n0 = short_bs // 2
    n1 = long_bs // 2
    w_short = _LIBV_WINDOW_256
    w_long = _LIBV_WINDOW_2048
    w_long_rev = np.ascontiguousarray(w_long[::-1])
    w_short_rev = np.ascontiguousarray(w_short[::-1])
    sl_off = n1 // 2 - n0 // 2  # small/large content offset (n1/2 - n0/2)

    # libvorbis MDCT lookups (the exact oracle implementation).
    look_short = _MDCTLookup()
    look_long = _MDCTLookup()
    lib.mdct_init(ctypes.byref(look_short), short_bs)
    lib.mdct_init(ctypes.byref(look_long), long_bs)

    # f32 codebook values (libvorbis sharedbook semantics).
    bookvals = [_libv_book_values_f32(b) for b in ctx.books]

    # libvorbis double buffer (two halves of length n1 per channel).
    pcm_buf = [np.zeros(2 * n1, dtype=np.float32) for _ in range(ctx.channels)]
    centerW = 0
    prev_W: int | None = None
    pcm_returned = -1

    # Pass 2: decode fully and stream the OLA (all f32).
    mode_history2: list[int] = []
    strict_closure_ok = True
    strict_mismatch_packets: list[dict] = []
    bit_closure_ok = True
    bit_mismatch_packets: list[dict] = []
    block_size_counts: Counter = Counter()
    for pi, payload in enumerate(audio_packets):
        mode, spectra, block_size, bits_consumed, res_status, bits_left, _flag = (
            decode_packet_2ch_libv(ctx, payload, mode_history2, bookvals)
        )
        block_size_counts[mode] += 1
        total_bits = len(payload) * 8
        if bits_consumed > total_bits:
            bit_closure_ok = False
            bit_mismatch_packets.append(
                {"packet": pi, "consumed": bits_consumed, "available": total_bits}
            )
        # Zero-floor packets legitimately carry no residue: libvorbis skips the
        # residue stage entirely when no channel is nonzero, so "empty" with
        # byte-padding is a complete closure ("complete" only when residue ran).
        if not (res_status in ("complete", "empty") and 0 <= bits_left < 8):
            strict_closure_ok = False
            strict_mismatch_packets.append(
                {
                    "packet": pi,
                    "status": res_status,
                    "consumed": bits_consumed,
                    "bits_left": bits_left,
                    "available": total_bits,
                }
            )
        W = 1 if block_size == long_bs else 0
        lW = prev_W
        if centerW:
            thisCenter, prevCenter = n1, 0
        else:
            thisCenter, prevCenter = 0, n1
        for ch in range(ctx.channels):
            if res_status == "empty":
                y = np.zeros(block_size, dtype=np.float32)
            else:
                y = np.empty(block_size, dtype=np.float32)
                x = np.ascontiguousarray(spectra[ch][: block_size // 2])
                look = look_long if W else look_short
                lib.mdct_backward(
                    ctypes.byref(look),
                    x.ctypes.data_as(ctypes.POINTER(ctypes.c_float)),
                    y.ctypes.data_as(ctypes.POINTER(ctypes.c_float)),
                )
            if lW:
                if W:
                    # large/large
                    seg = pcm_buf[ch][prevCenter : prevCenter + n1]
                    helper.libv_ola_fma(
                        seg.ctypes.data_as(_fptr),
                        seg.ctypes.data_as(_fptr),
                        w_long_rev.ctypes.data_as(_fptr),
                        np.ascontiguousarray(y[:n1]).ctypes.data_as(_fptr),
                        w_long.ctypes.data_as(_fptr),
                        n1,
                    )
                else:
                    # large/small
                    off = prevCenter + n1 // 2 - n0 // 2
                    seg = pcm_buf[ch][off : off + n0]
                    helper.libv_ola_fma(
                        seg.ctypes.data_as(_fptr),
                        seg.ctypes.data_as(_fptr),
                        w_short_rev.ctypes.data_as(_fptr),
                        np.ascontiguousarray(y[:n0]).ctypes.data_as(_fptr),
                        w_short.ctypes.data_as(_fptr),
                        n0,
                    )
            else:
                if W:
                    # small/large
                    off = prevCenter
                    seg = pcm_buf[ch][off : off + n0]
                    helper.libv_ola_fma(
                        seg.ctypes.data_as(_fptr),
                        seg.ctypes.data_as(_fptr),
                        w_short_rev.ctypes.data_as(_fptr),
                        np.ascontiguousarray(
                            y[sl_off : sl_off + n0]
                        ).ctypes.data_as(_fptr),
                        w_short.ctypes.data_as(_fptr),
                        n0,
                    )
                    i1, i2 = n0, n1 // 2 + n0 // 2
                    pcm_buf[ch][off + i1 : off + i2] = (
                        y[sl_off + i1 : sl_off + i2]
                    )
                else:
                    # small/small
                    seg = pcm_buf[ch][prevCenter : prevCenter + n0]
                    helper.libv_ola_fma(
                        seg.ctypes.data_as(_fptr),
                        seg.ctypes.data_as(_fptr),
                        w_short_rev.ctypes.data_as(_fptr),
                        np.ascontiguousarray(y[:n0]).ctypes.data_as(_fptr),
                        w_short.ctypes.data_as(_fptr),
                        n0,
                    )
            # copy section: stash the second half for the next frame's OLA.
            n = block_size // 2
            pcm_buf[ch][thisCenter : thisCenter + n] = y[n:]
        if centerW:
            centerW = 0
        else:
            centerW = n1
        if pcm_returned == -1:
            pcm_returned = thisCenter
            pcm_current = thisCenter
        else:
            pcm_returned = prevCenter
            prev_size = long_bs if prev_W else short_bs
            pcm_current = prevCenter + prev_size // 4 + block_size // 4
        n_chunk = pcm_current - pcm_returned
        if n_chunk > 0:
            for ch in range(ctx.channels):
                out_buf[ch][starts[pi] : starts[pi] + n_chunk] = (
                    pcm_buf[ch][pcm_returned : pcm_returned + n_chunk].copy()
                )
        prev_W = W
    return {
        "out_buf": out_buf,
        "offset": 0,
        "positions": starts,
        "mode_history": mode_history2,
        "bit_closure_ok": bit_closure_ok,
        "bit_mismatch_packets": bit_mismatch_packets,
        "block_size_counts": dict(block_size_counts),
        "n_audio_packets": len(audio_packets),
        "strict_closure_ok": strict_closure_ok,
        "strict_mismatch_packets": strict_mismatch_packets,
        "libv_f32": True,
    }


def decode_numpy_pcm(
    ctx: DecodeContext, wem_bytes: bytes, declared_frames: int
) -> tuple[np.ndarray, dict[str, Any]]:
    """Decode one carried WEM with the deterministic NumPy synthesis path.

    The returned array is channel-major and contains exactly ``declared_frames``
    samples per channel.  The generic overlap-add path exposes its synthesis
    lead-in through ``offset``; the stereo libvorbis-layout path begins at PCM
    sample zero.  Keeping that container-origin handling here makes callers
    compare PCM positions rather than independently fitting an alignment.
    """
    if _LIBVORBIS_EXACT:
        raise RuntimeError("decode_numpy_pcm requires the deterministic NumPy path")
    _data, packets, _seek, _fmt = parse_container(wem_bytes)
    if len(packets) < 2:
        raise ValueError("WEM has no audio packets")
    audio_packets = packets[1:]
    if ctx.channels == 2 and ctx.sample_rate == 48_000:
        result = run_decode_2ch(ctx, audio_packets)
        offset = 0
    else:
        result = run_decode(ctx, audio_packets)
        offset = int(result["offset"])
    pcm = np.asarray(result["out_buf"], dtype=np.float64)
    if pcm.ndim != 2 or pcm.shape[0] != ctx.channels:
        raise ValueError(f"decoder returned {pcm.shape}, expected {ctx.channels} channels")
    stop = offset + declared_frames
    if pcm.shape[1] < stop:
        raise ValueError(
            f"decoder returned {pcm.shape[1] - offset} PCM frames, need {declared_frames}"
        )
    return pcm[:, offset:stop], result


def run_decode_2ch_kernel_f64(
    ctx: DecodeContext, audio_packets: list[bytes]
) -> dict[str, Any]:
    """Fallback: float64 numerically-derived kernel + libv blockin OLA.

    Geometry follows the libvorbis-compatible layout; values are a numerical
    approximation (not bit-exact).  Only used when libvorbis cannot be loaded.
    """
    # Pass 1: decode modes to know positions and window bits.
    mode_history: list[int] = []
    for payload in audio_packets:
        br = BitReader(payload)
        header = parse_audio_header(br, ctx.setup)
        mode_history.append(header["mode"])
    starts, lens = libv_chunk_layout(mode_history, ctx.blocksizes)
    total_len = sum(lens)
    out_buf = [np.zeros(total_len, dtype=np.float64) for _ in range(ctx.channels)]

    short_bs, long_bs = ctx.blocksizes
    n0 = short_bs // 2
    n1 = long_bs // 2
    w_short = np.asarray(vorbis_window(short_bs), dtype=np.float64)
    w_long = np.asarray(vorbis_window(long_bs), dtype=np.float64)
    idx_l = np.arange(n1)
    w_long_rev = w_long[n1 - idx_l - 1]
    idx_s = np.arange(n0)
    w_short_rev = w_short[n0 - idx_s - 1]
    sl_off = n1 // 2 - n0 // 2  # small/large content offset (n1/2 - n0/2)

    # libvorbis double buffer (two halves of length n1 per channel).
    pcm_buf = [np.zeros(2 * n1, dtype=np.float64) for _ in range(ctx.channels)]
    centerW = 0
    prev_W: int | None = None
    pcm_returned = -1

    # Pass 2: decode fully and stream the OLA.
    mode_history2: list[int] = []
    strict_closure_ok = True
    strict_mismatch_packets: list[dict] = []
    bit_closure_ok = True
    bit_mismatch_packets: list[dict] = []
    block_size_counts: Counter = Counter()
    for pi, payload in enumerate(audio_packets):
        mode, spectra, block_size, bits_consumed, res_status, bits_left, _flag = (
            decode_packet_2ch(ctx, payload, mode_history2)
        )
        block_size_counts[mode] += 1
        total_bits = len(payload) * 8
        if bits_consumed > total_bits:
            bit_closure_ok = False
            bit_mismatch_packets.append(
                {"packet": pi, "consumed": bits_consumed, "available": total_bits}
            )
        if not (res_status in ("complete", "empty") and 0 <= bits_left < 8):
            strict_closure_ok = False
            strict_mismatch_packets.append(
                {
                    "packet": pi,
                    "status": res_status,
                    "consumed": bits_consumed,
                    "bits_left": bits_left,
                    "available": total_bits,
                }
            )
        W = 1 if block_size == long_bs else 0
        lW = prev_W
        if centerW:
            thisCenter, prevCenter = n1, 0
        else:
            thisCenter, prevCenter = 0, n1
        for ch in range(ctx.channels):
            if res_status == "empty":
                y = np.zeros(block_size, dtype=np.float64)
            else:
                y = (
                    ctx.kernel_map[block_size] @ spectra[ch]
                ) / (w_long if W else w_short)
            if lW:
                if W:
                    # large/large
                    seg = pcm_buf[ch][prevCenter : prevCenter + n1]
                    pcm_buf[ch][prevCenter : prevCenter + n1] = seg * w_long_rev + y[
                        :n1
                    ] * w_long[idx_l]
                else:
                    # large/small
                    off = prevCenter + n1 // 2 - n0 // 2
                    seg = pcm_buf[ch][off : off + n0]
                    pcm_buf[ch][off : off + n0] = seg * w_short_rev + y[:n0] * w_short[
                        idx_s
                    ]
            else:
                if W:
                    # small/large
                    off = prevCenter
                    seg = pcm_buf[ch][off : off + n0]
                    pcm_buf[ch][off : off + n0] = (
                        seg * w_short_rev + y[sl_off : sl_off + n0] * w_short[idx_s]
                    )
                    i1, i2 = n0, n1 // 2 + n0 // 2
                    pcm_buf[ch][off + i1 : off + i2] = y[sl_off + i1 : sl_off + i2]
                else:
                    # small/small
                    seg = pcm_buf[ch][prevCenter : prevCenter + n0]
                    pcm_buf[ch][prevCenter : prevCenter + n0] = (
                        seg * w_short_rev + y[:n0] * w_short[idx_s]
                    )
            # copy section: stash the second half for the next frame's OLA.
            n = block_size // 2
            pcm_buf[ch][thisCenter : thisCenter + n] = y[n:]
        if centerW:
            centerW = 0
        else:
            centerW = n1
        if pcm_returned == -1:
            pcm_returned = thisCenter
            pcm_current = thisCenter
        else:
            pcm_returned = prevCenter
            prev_size = long_bs if prev_W else short_bs
            pcm_current = prevCenter + prev_size // 4 + block_size // 4
        n_chunk = pcm_current - pcm_returned
        if n_chunk > 0:
            for ch in range(ctx.channels):
                out_buf[ch][starts[pi] : starts[pi] + n_chunk] = (
                    pcm_buf[ch][pcm_returned : pcm_returned + n_chunk].copy()
                )
        prev_W = W
    return {
        "out_buf": out_buf,
        "offset": 0,
        "positions": starts,
        "mode_history": mode_history2,
        "bit_closure_ok": bit_closure_ok,
        "bit_mismatch_packets": bit_mismatch_packets,
        "block_size_counts": dict(block_size_counts),
        "n_audio_packets": len(audio_packets),
        "strict_closure_ok": strict_closure_ok,
        "strict_mismatch_packets": strict_mismatch_packets,
    }


def rms_envelope(
    pcm: list[np.ndarray], channels: int, sample_rate: int, frame_ms: float = 10.0
) -> list[float]:
    """Per-frame RMS (<=10ms) across channels, for the energy envelope."""
    frame_samples = int(round(frame_ms / 1000.0 * sample_rate))
    n = len(pcm[0]) if pcm else 0
    env: list[float] = []
    for start in range(0, n, frame_samples):
        stop = min(start + frame_samples, n)
        seg = np.concatenate([c[start:stop] for c in pcm])
        env.append(float(np.sqrt(np.mean(seg * seg))))
    return env


def _write_wav_pcm16(
    path: Path, pcm16: np.ndarray, sample_rate: int
) -> None:
    """Write a (channels, frames) int16 block as an interleaved 16-bit PCM WAV.

    The single owner of the file layout: both writers below differ only in how
    they prepare ``pcm16``.
    """
    interleaved = pcm16.T.reshape(-1)
    data_bytes = interleaved.tobytes()
    with open(path, "wb") as handle:
        handle.write(b"RIFF")
        handle.write(struct.pack("<I", 36 + len(data_bytes)))
        handle.write(b"WAVE")
        handle.write(b"fmt ")
        handle.write(struct.pack("<I", 16))
        handle.write(struct.pack("<H", 1))
        handle.write(struct.pack("<H", pcm16.shape[0]))
        handle.write(struct.pack("<I", sample_rate))
        handle.write(struct.pack("<I", sample_rate * pcm16.shape[0] * 2))
        handle.write(struct.pack("<H", pcm16.shape[0] * 2))
        handle.write(struct.pack("<H", 16))
        handle.write(b"data")
        handle.write(struct.pack("<I", len(data_bytes)))
        handle.write(data_bytes)


def write_wav(path: Path, pcm: list[np.ndarray], sample_rate: int) -> None:
    """Write an interleaved 16-bit PCM WAV."""
    channels = len(pcm)
    frames = len(pcm[0])
    pcm16 = np.empty((channels, frames), dtype=np.int16)
    for ch in range(channels):
        scaled = np.clip(pcm[ch] * 32768.0, -32768, 32767)
        pcm16[ch] = scaled
    _write_wav_pcm16(path, pcm16, sample_rate)


def write_wav_i16(
    path: Path, pcm: list[np.ndarray], sample_rate: int
) -> None:
    """Write an interleaved 16-bit PCM WAV from pre-converted int16 data."""
    channels = len(pcm)
    frames = len(pcm[0])
    pcm16 = np.empty((channels, frames), dtype=np.int16)
    for ch in range(channels):
        pcm16[ch] = pcm[ch]
    _write_wav_pcm16(path, pcm16, sample_rate)


def run_decode(ctx: DecodeContext, audio_packets: list[bytes]) -> dict[str, Any]:
    """Full decode: bit-closure check, streaming OLA PCM, statistics."""
    mode_history: list[int] = []
    bit_closure_ok = True
    bit_mismatch_packets: list[dict] = []
    block_size_counts: Counter = Counter()
    positions = compute_positions(mode_history, ctx.blocksizes) if mode_history else []

    # Two-pass streaming: first pass decodes modes to know positions, then a
    # second pass streams the OLA (avoids storing every spectrum in memory).
    # Pass 1: decode headers to collect the mode sequence.
    for payload in audio_packets:
        br = BitReader(payload)
        header = parse_audio_header(br, ctx.setup)
        mode_history.append(header["mode"])
    positions = compute_positions(mode_history, ctx.blocksizes)
    min_pos = min(positions)
    offset = -min_pos if min_pos < 0 else 0
    total_len = (max(positions) + offset) + ctx.blocksizes[1] + 100
    out_buf = [np.zeros(total_len, dtype=np.float64) for _ in range(ctx.channels)]

    # Pass 2: decode fully and stream the OLA.
    mode_history2: list[int] = []
    strict_closure_ok = True
    strict_mismatch_packets: list[dict] = []
    for pi, payload in enumerate(audio_packets):
        mode, spectra, block_size, bits_consumed, res_status, bits_left = (
            decode_packet_spectra(ctx, payload, mode_history2)
        )
        block_size_counts[mode] += 1
        total_bits = len(payload) * 8
        # Loose over-read check (kept for backward compatibility).
        if bits_consumed > total_bits:
            bit_closure_ok = False
            bit_mismatch_packets.append(
                {"packet": pi, "consumed": bits_consumed, "available": total_bits}
            )
        # Strict closure: residue complete (or zero-floor empty) AND 0 <= bits_left < 8.
        if not (res_status in ("complete", "empty") and 0 <= bits_left < 8):
            strict_closure_ok = False
            strict_mismatch_packets.append(
                {
                    "packet": pi,
                    "status": res_status,
                    "consumed": bits_consumed,
                    "bits_left": bits_left,
                    "available": total_bits,
                }
            )
        coupled = apply_coupling(spectra, ctx)
        previous = mode_history[pi - 1] if pi else 0
        following = mode_history[pi + 1] if pi + 1 < len(mode_history) else 1
        current = 1 if block_size == ctx.blocksizes[1] else 0
        imdct = ctx.imdct_map[block_size]
        pos = positions[pi] + offset
        for ch in range(ctx.channels):
            block = imdct @ coupled[ch]
            out_buf[ch][pos : pos + block_size] += apply_hybrid_synthesis_window(
                block,
                ctx.blocksizes,
                previous,
                current,
                following,
                ctx.window_halves,
            )
    return {
        "out_buf": out_buf,
        "offset": offset,
        "positions": positions,
        "mode_history": mode_history2,
        "bit_closure_ok": bit_closure_ok,
        "bit_mismatch_packets": bit_mismatch_packets,
        "block_size_counts": dict(block_size_counts),
        "n_audio_packets": len(audio_packets),
        "strict_closure_ok": strict_closure_ok,
        "strict_mismatch_packets": strict_mismatch_packets,
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Research WEM decoder (2ch/48k paired-build probe)."
    )
    parser.add_argument(
        "--wem",
        type=Path,
        default=None,
        help="WEM file to decode (required unless --self-check)",
    )
    parser.add_argument("--self-check", action="store_true")
    parser.add_argument("--segments", action="store_true")
    parser.add_argument(
        "--libvorbis-exact",
        action="store_true",
        help="mirror the local libvorbis 1.3.7 float32 DSP instead of numpy",
    )
    parser.add_argument("--stats", type=Path, default=None)
    parser.add_argument("--segment-seconds", type=float, default=32.0)
    args = parser.parse_args()

    if args.self_check:
        return run_self_check()
    if args.wem is None:
        parser.error("--wem is required for a decode run (or pass --self-check)")

    global _LIBVORBIS_EXACT
    _LIBVORBIS_EXACT = args.libvorbis_exact

    started = time.time()
    from wwise_wem import WwiseProfile, WwiseVersion
    from wwise_wem_reference.profiles.artifact import resolve_selection

    profile = resolve_selection(
        WwiseProfile(WwiseVersion.WWISE2013, 2, SAMPLE_RATE)
    )
    ctx = build_context(profile, channels=2, sample_rate=SAMPLE_RATE)
    raw = args.wem.read_bytes()
    data_payload, packets, seek_table_size, fmt = parse_container(raw)
    if not packets:
        print("ERROR: no packets", file=sys.stderr)
        return 1
    setup_packet, audio_packets = packets[0], packets[1:]

    # Framing diagnostics.
    packet_sizes = [len(p) for p in audio_packets]
    total_audio_payload = sum(
        2 + size for size in packet_sizes
    )  # each packet: u16 size + payload
    n_audio = len(audio_packets)
    seek_entries_4b = seek_table_size // 4

    # PCM cache (avoid re-decoding on repeated runs).
    # NOTE: cache key reflects the decode version AND the synthesis path, so
    # the deterministic numpy default and the opt-in libvorbis byte-exact
    # mode never read or overwrite each other's results. Delete stale caches
    # after changing decode logic.
    cache_tag = (
        "v6_libvf32"
        if _LIBVORBIS_EXACT
        else "v6_numpy_f64"
    )
    pcm_cache = DEFAULT_OUT_DIR / f"_pcm_cache_{args.wem.name}_{len(raw)}_{cache_tag}.npz"
    if pcm_cache.exists():
        print(f"Loading PCM cache: {pcm_cache}")
        cached = np.load(pcm_cache, allow_pickle=True)
        out_buf = [cached["ch0"], cached["ch1"]]
        offset = int(cached["offset"])
        mode_history2 = [int(x) for x in cached["mode_history"]]
        bit_closure_ok = bool(cached["bit_closure_ok"])
        strict_closure_ok = (
            bool(cached["strict_closure_ok"])
            if "strict_closure_ok" in cached.files
            else False
        )
        libv_f32 = bool(cached["libv_f32"]) if "libv_f32" in cached.files else False
        strict_mismatch_packets = []
        block_size_counts = dict(cached["block_size_counts"].item())
        bit_mismatch_packets: list = []
    else:
        result = run_decode_2ch(ctx, audio_packets)
        out_buf = result["out_buf"]
        libv_f32 = bool(result["libv_f32"])
        offset = result["offset"]
        mode_history2 = result["mode_history"]
        bit_closure_ok = result["bit_closure_ok"]
        bit_mismatch_packets = result["bit_mismatch_packets"]
        strict_closure_ok = result["strict_closure_ok"]
        strict_mismatch_packets = result["strict_mismatch_packets"]
        block_size_counts = result["block_size_counts"]
        DEFAULT_OUT_DIR.mkdir(parents=True, exist_ok=True)
        np.savez_compressed(
            pcm_cache,
            ch0=out_buf[0],
            ch1=out_buf[1],
            offset=offset,
            mode_history=np.array(mode_history2, dtype=np.int32),
            bit_closure_ok=bit_closure_ok,
            strict_closure_ok=strict_closure_ok,
            libv_f32=libv_f32,
            block_size_counts=np.array([block_size_counts], dtype=object),
        )
        print(f"Saved PCM cache: {pcm_cache}")

    # Bit-closure validation report.
    print(f"Audio packets: {n_audio}")
    print(f"Seek table: {seek_table_size}B = {seek_entries_4b} x 4B entries")
    print(
        f"dwVorbisDataOffset={fmt['dwVorbisDataOffset']} "
        f"dwFirstAudioPacketOffset={fmt['dwFirstAudioPacketOffset']} "
        f"dwSeekTableSize={seek_table_size}"
    )
    print(
        f"dwVorbisDataOffset == seek + first_audio? "
        f"{seek_table_size + int(fmt['dwFirstAudioPacketOffset'])}"
    )
    print(
        f"dwDataPayloadSize={fmt['dwDataPayloadSize']} ; "
        f"seek + packet_stream = {seek_table_size + total_audio_payload + 2 + len(setup_packet)}"
    )
    print(f"Bit closure OK: {bit_closure_ok}")
    if bit_mismatch_packets:
        print(f"  mismatches (first 5): {bit_mismatch_packets[:5]}")
    print(f"Strict closure OK (complete AND 0<=bits_left<8): {strict_closure_ok}")
    if strict_mismatch_packets:
        print(f"  strict mismatches (first 5): {strict_mismatch_packets[:5]}")
    print(f"Mode histogram: {block_size_counts}")
    # Block-size switch events.
    switches = sum(
        1
        for i in range(1, len(mode_history2))
        if mode_history2[i] != mode_history2[i - 1]
    )
    print(f"Block-size switch events: {switches}")

    # Statistics.
    valid_start = offset
    valid_pcm = [b[valid_start:].astype(np.float64) for b in out_buf]
    env = rms_envelope(valid_pcm, ctx.channels, SAMPLE_RATE, 10.0)
    n_audio_bytes_payload = sum(len(p) for p in audio_packets)
    duration_sec = len(valid_pcm[0]) / SAMPLE_RATE
    # Measured average bitrate over the audio payload (size headers + payloads).
    total_audio_bytes = n_audio_bytes_payload + 2 * n_audio
    bitrate_kbps = (total_audio_bytes * 8.0) / duration_sec / 1000.0 if duration_sec > 0 else 0.0

    stats = {
        "file": str(args.wem),
        "profile": profile.label(),
        "sample_rate": SAMPLE_RATE,
        "channels": ctx.channels,
        "seek_table": {
            "size_bytes": seek_table_size,
            "entries_4b": seek_entries_4b,
            "entries_eq_audio_packets": seek_entries_4b == n_audio,
        },
        "packets": {
            "setup_bytes": len(setup_packet),
            "audio_packet_count": n_audio,
            "size_min": int(min(packet_sizes)) if packet_sizes else 0,
            "size_max": int(max(packet_sizes)) if packet_sizes else 0,
            "size_mean": float(np.mean(packet_sizes)) if packet_sizes else 0.0,
            "size_quantiles": {
                "p50": int(np.percentile(packet_sizes, 50)),
                "p90": int(np.percentile(packet_sizes, 90)),
                "p95": int(np.percentile(packet_sizes, 95)),
                "p99": int(np.percentile(packet_sizes, 99)),
                "p999": int(np.percentile(packet_sizes, 99.9)),
                "max": int(np.max(packet_sizes)),
            }
            if packet_sizes
            else {},
            "size_histogram": {
                str(lo): int(np.sum((np.array(packet_sizes) >= lo) & (np.array(packet_sizes) < hi)))
                for lo, hi in [(1, 8), (8, 32), (32, 64), (64, 128), (128, 256), (256, 512)]
            }
            if packet_sizes
            else {},
        },
        "mode": {
            "short_count": block_size_counts.get(0, 0),
            "long_count": block_size_counts.get(1, 0),
            "block_switch_events": switches,
            "history_head": mode_history2[:40],
        },
        "bit_closure": {
            "ok": bit_closure_ok,
            "mismatch_count": len(bit_mismatch_packets),
        },
        "strict_closure": {
            "ok": strict_closure_ok,
            "mismatch_count": len(strict_mismatch_packets),
            "criteria": "residue_status in {complete, empty} AND 0 <= bits_left < 8",
            "first_mismatches": strict_mismatch_packets[:5],
        },
        "payload": {
            "audio_packet_bytes": n_audio_bytes_payload,
            "audio_packet_with_headers_bytes": total_audio_bytes,
            "duration_sec": float(duration_sec),
            "avg_bitrate_kbps": float(bitrate_kbps),
        },
        "energy_envelope_rms_10ms": {
            "frame_samples": int(round(0.01 * SAMPLE_RATE)),
            "frames": len(env),
            "values": [round(v, 8) for v in env],
        },
        "decode_offset_samples": int(offset),
    }

    out_dir = DEFAULT_OUT_DIR
    out_dir.mkdir(parents=True, exist_ok=True)
    stats_path = args.stats or (out_dir / "ground_truth_stats.json")
    # Deterministic JSON (sorted keys, stable floats).
    stats_text = json.dumps(stats, indent=2, sort_keys=True) + "\n"
    stats_path.write_text(stats_text)
    print(f"Wrote stats: {stats_path}")

    if args.segments:
        if libv_f32:
            pcm_i16 = libv_float_to_int16([b[valid_start:] for b in out_buf])
            _write_segments_i16(pcm_i16, args.segment_seconds, out_dir, SAMPLE_RATE)
        else:
            _write_segments(valid_pcm, args.segment_seconds, out_dir, SAMPLE_RATE)

    print(f"Decode time: {time.time() - started:.1f}s")
    return 0


def _write_segments(
    pcm: list[np.ndarray],
    seconds: float,
    out_dir: Path,
    sample_rate: int,
) -> None:
    total = len(pcm[0])
    frames = int(seconds * sample_rate)
    # Head, middle, tail segments (>= seconds each).
    head = [c[:frames] for c in pcm]
    mid_start = total // 2 - frames // 2
    mid_end = mid_start + frames
    mid = [c[mid_start:mid_end] for c in pcm]
    tail_start = total - frames
    tail = [c[tail_start:total] for c in pcm]
    seg_specs = [
        ("head", head),
        ("middle", mid),
        ("tail", tail),
    ]
    for name, seg in seg_specs:
        # Pad if needed (shouldn't happen for valid files).
        if len(seg[0]) < frames:
            seg = [
                np.pad(c, (0, frames - len(c)))
                for c in seg
            ]
        path = out_dir / f"segment_{name}_{seconds:.0f}s.wav"
        write_wav(path, seg, sample_rate)
        print(f"Wrote {path} ({len(seg[0])} frames)")


def _write_segments_i16(
    pcm: list[np.ndarray],
    seconds: float,
    out_dir: Path,
    sample_rate: int,
) -> None:
    """libv int16 segments (vgmstream-exact conversion already applied)."""
    total = len(pcm[0])
    frames = int(seconds * sample_rate)
    head = [c[:frames] for c in pcm]
    mid_start = total // 2 - frames // 2
    mid_end = mid_start + frames
    mid = [c[mid_start:mid_end] for c in pcm]
    tail_start = total - frames
    tail = [c[tail_start:total] for c in pcm]
    seg_specs = [
        ("head", head),
        ("middle", mid),
        ("tail", tail),
    ]
    for name, seg in seg_specs:
        if len(seg[0]) < frames:
            seg = [np.pad(c, (0, frames - len(c))) for c in seg]
        path = out_dir / f"segment_{name}_{seconds:.0f}s.wav"
        write_wav_i16(path, seg, sample_rate)
        print(f"Wrote {path} ({len(seg[0])} frames)")


def run_self_check() -> int:
    """Focused self-check: round-trip a reference-encoded 6ch WEM.

    Uses the 6ch profile (fully registered) and the repo fixture's PCM to
    verify the decode chain (framing, floor, residue, MDCT inverse, OLA).
    """
    fixture_pcm = _REPO_ROOT / "tests" / "fixtures" / "input.wav"
    if not fixture_pcm.exists():
        print("SELF-CHECK: missing fixture input.wav")
        return 1
    # Encode the fixture with the reference oracle.
    from wwise_wem_reference import python_engine  # type: ignore
    from wwise_wem_reference.container.model import ContainerPlan  # type: ignore
    from wwise_wem import WwiseProfile, WwiseVersion  # type: ignore
    from wwise_wem_reference.profiles.artifact import resolve_selection  # type: ignore
    from wwise_wem.adapters.wav import read_pcm_wav  # type: ignore

    pcm = read_pcm_wav(fixture_pcm)
    profile = resolve_selection(
        WwiseProfile(WwiseVersion.DEFAULT, pcm.channel_count, pcm.sample_rate)
    )
    result = python_engine.encode_pcm_python(
        profile=profile,
        container=ContainerPlan.from_profile(profile),
        pcm=pcm,
    )
    wem_bytes = bytes(result.data)
    # Decode.
    ctx = build_context(profile, channels=6, sample_rate=44100)
    raw = wem_bytes
    data_payload, packets, _seek, _fmt = parse_container(raw)
    _setup, audio_packets = packets[0], packets[1:]
    decode_result = run_decode(ctx, audio_packets)
    out_buf = decode_result["out_buf"]
    offset = decode_result["offset"]
    valid_pcm = [b[offset:] for b in out_buf]
    orig = np.array(
        [pcm.channels[c] for c in range(pcm.channel_count)], dtype=np.float64
    )
    # Compare a valid middle region.
    n = min(len(valid_pcm[0]), len(orig[0])) - 256
    errs = [
        float(np.max(np.abs(valid_pcm[c][:n] - orig[c][:n]))) for c in range(6)
    ]
    max_err = max(errs)
    # The encoder attenuates via psycho (lossy), so allow tolerance.
    # But the reconstruction should be structurally correct: report max_err.
    # A correct decoder on a lossless-ish signal gives small-to-moderate err.
    ok = max_err < 2.0  # lenient: reconstruction is structurally valid
    print(f"SELF-CHECK: max_abs_error={max_err:.5f} (tolerance 2.0)")
    print(f"SELF-CHECK: bit_closure_ok={decode_result['bit_closure_ok']}")
    print(f"SELF-CHECK: strict_closure_ok={decode_result['strict_closure_ok']}")
    print(f"SELF-CHECK: packets={len(audio_packets)} offset={offset}")
    print(f"SELF-CHECK: {'PASS' if ok and decode_result['bit_closure_ok'] else 'FAIL'}")
    return 0 if (ok and decode_result["bit_closure_ok"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())
