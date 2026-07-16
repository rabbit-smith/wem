#!/usr/bin/env python3
"""Encode Wwise Vorbis audio packets: header, floor, and residue.

This Wwise build's audio header contains its mode bits only: unlike raw
Vorbis-I long packets, it does not serialize previous/next-window flags.
Floor1 body matches Vorbis I.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Sequence

from ..analysis.model import PsyFrame
from .bitio import OggPack
from .codebook import Codebook
from .floor import (
    FLOOR1_RANGES,
    floor1_curve_from_posts,
    floor1_wrap,
    floor1_wrap_with_posts,
    postlist_from_floor,
)
from .floor_fit import floor1_fit_wwise, floor1_quantize_posts
from .residue import (
    mdct_to_residue,
    pack_residue_silent,
    pack_residue_vq,
    quantize_residue_value,
)
from .setup import ilog


def _nearest_used_entry(book: Codebook, value: int) -> int:
    """Map residual value to a used codebook entry (dim-1 maptype0: entry≈value)."""
    used = book.used_entries()
    if not used:
        raise ValueError("codebook has no used entries")
    if value in used or (
        0 <= value < book.entries and book.lengthlist[value] > 0
    ):
        return value
    # nearest used
    return min(used, key=lambda e: abs(e - value))


def _pick_subclass_cval(
    floor: dict, cl: int, Y_slice: Sequence[int], books: Sequence[Codebook]
) -> int:
    """Choose masterbook cval so each dim's subclass book can represent Y."""
    cbits = floor["class_subs"][cl]
    cdim = floor["class_dims"][cl]
    csub = (1 << cbits) - 1
    sbooks = floor["subclass_books"][cl]
    if cbits == 0:
        return 0
    best = 0
    best_score = -1
    limit = 1 << (cbits * cdim)
    # cap search for large dims
    if limit > 4096:
        limit = 4096
    for cval in range(limit):
        t = cval
        score = 0
        ok = True
        for j in range(cdim):
            book_id = sbooks[t & csub]
            t >>= cbits
            y = Y_slice[j]
            if book_id < 0:
                if y != 0:
                    ok = False
                    break
                score += 1
            else:
                b = books[book_id]
                if y < b.entries and b.lengthlist[y] > 0:
                    score += 2
                elif any(abs(e - y) <= 2 for e in b.used_entries()):
                    score += 1
                else:
                    score += 0
        if ok and score > best_score:
            best_score = score
            best = cval
            if score == 2 * cdim:
                break
    return best


def pack_audio_header(
    op: OggPack,
    setup: dict,
    mode: int,
    prev_window: int = 0,
    next_window: int = 0,
) -> None:
    """Write the Wwise audio header.

    ``prev_window`` and ``next_window`` remain accepted for compatibility
    with callers that also track the PCM window scheduler.  They are not
    packet fields in this format revision, which writes only the mode value.
    """
    nmodes = setup["nmodes"]
    mode_bits = ilog(nmodes - 1) if nmodes > 1 else 0
    if mode_bits:
        op.write(mode, mode_bits)


def pack_floor1_body(op: OggPack, floor: dict, books: Sequence[Codebook], Y: list[int]) -> None:
    """Pack floor1 packet Y posts (nonzero flag already written as 1).

    ``Y`` must be *wrapped residuals* as produced by ``floor1.floor1_wrap``,
    not absolute post heights. Endpoints Y[0]/Y[1] are absolute quant values;
    interior Y[i] are prediction residuals (0 ⇒ “use predicted”). Callers that
    fit absolute curves (e.g. floor1_fit_simple) must wrap before packing::

        from .floor import postlist_from_floor, floor1_wrap, FLOOR1_RANGES
        pl = postlist_from_floor(floor)
        Y = floor1_wrap(absolute_posts, pl, FLOOR1_RANGES[floor["multiplier"]])
        pack_floor1_body(op, floor, books, Y)
    """
    rng = FLOOR1_RANGES[floor["multiplier"]]
    ybits = ilog(rng - 1)
    nvals = 2 + len(floor["x_list"])
    if len(Y) != nvals:
        raise ValueError(f"Y len {len(Y)} != {nvals}")
    # endpoints are absolute quant values in [0, range)
    op.write(max(0, min(rng - 1, int(Y[0]))), ybits)
    op.write(max(0, min(rng - 1, int(Y[1]))), ybits)
    ppos = 2
    for p in range(floor["partitions"]):
        cl = floor["partition_classes"][p]
        cdim = floor["class_dims"][cl]
        cbits = floor["class_subs"][cl]
        csub = (1 << cbits) - 1
        y_slice = [int(Y[ppos + j]) for j in range(cdim)]
        if cbits:
            cval = _pick_subclass_cval(floor, cl, y_slice, books)
            mb = floor["class_masterbooks"][cl]
            books[mb].encode(op, _nearest_used_entry(books[mb], cval))
        else:
            cval = 0
        for j in range(cdim):
            book_id = floor["subclass_books"][cl][cval & csub]
            cval >>= cbits
            if book_id >= 0:
                entry = _nearest_used_entry(books[book_id], y_slice[j])
                books[book_id].encode(op, entry)
            # book_id < 0 ⇒ residual forced 0 (no bits)
            ppos += 1


def pack_silence_packet(
    setup: dict,
    channels: int,
    mode: int = 0,
    prev_window: int = 0,
    next_window: int = 0,
) -> bytes:
    """
    Silence audio packet: floor nonzero=0 for all channels → no residue.

    Short (mode0): 1 + channels bits (typically 7 → 1 byte).
    Long (mode1): 1 + channels bits.
    """
    op = OggPack(16)
    pack_audio_header(op, setup, mode, prev_window, next_window)
    for _ in range(channels):
        op.write(0, 1)
    return op.get_buffer()


def pack_floor_only_packet(
    setup: dict,
    books: Sequence[Codebook],
    channels: int,
    mode: int,
    curves: list[list[int] | None],
    *,
    prev_window: int = 0,
    next_window: int = 0,
    silent_residue: bool = True,
    absolute_posts: bool = False,
    n_spectrum: int | None = None,
) -> bytes:
    """
    Pack mode + floor1 bodies; optional silent residue (class0 only).

    curves[ch] = None ⇒ nonzero 0.
    If absolute_posts=False (default): curves are *wrapped residuals*.
    If absolute_posts=True: curves are absolute posts (fit_simple output);
    they are wrapped with floor1_wrap before packing.
    """
    op = OggPack(512)
    pack_audio_header(op, setup, mode, prev_window, next_window)
    md = setup["modes"][mode]
    mapping = setup["maps"][md["mapping"]]
    any_nz = False
    ch_count = 0
    for ch in range(channels):
        sub = mapping["chmux"][ch] if mapping["submaps"] > 1 else 0
        floor = setup["floors"][mapping["floors"][sub]]
        Y = curves[ch] if ch < len(curves) else None
        if Y is None:
            op.write(0, 1)
        else:
            op.write(1, 1)
            if absolute_posts:
                pl = postlist_from_floor(floor)
                rng = FLOOR1_RANGES[floor["multiplier"]]
                Y = floor1_wrap(Y, pl, rng)
            pack_floor1_body(op, floor, books, Y)
            any_nz = True
            ch_count += 1
    if any_nz and silent_residue:
        res = setup["residues"][mapping["residues"][0]]
        pack_residue_silent(
            op, res, books, ch_count, n_spectrum=n_spectrum
        )
    return op.get_buffer()


@dataclass(frozen=True)
class BlockPacketResult:
    """Encoded packet and the exact integer residue rows supplied to VQ."""

    packet: bytes
    quantized_residue: tuple[tuple[int, ...], ...]


def pack_block_packet_details(
    setup: dict,
    books: Sequence[Codebook],
    channels: int,
    mode: int,
    absolute_posts: list[list[int] | None],
    mdct: list[list[float]],
    *,
    prev_window: int = 0,
    next_window: int = 0,
    residue_vq: bool = True,
    posts_are_10bit: bool = False,
 ) -> BlockPacketResult:
    """
    Full short/long audio packet: floor1 + residue VQ (or silent).

    absolute_posts[ch]: packet-domain fit output or None (floor zero).
        Set ``posts_are_10bit`` when these are raw Wwise fit values from
        ``floor1_fit_wwise``; the packet encoder then applies the format's
        multiplier shift before wrapping.
    mdct[ch]: MDCT spectrum for residual = mdct/floor_amp.
    """
    op = OggPack(8192)
    pack_audio_header(op, setup, mode, prev_window, next_window)
    md = setup["modes"][mode]
    mapping = setup["maps"][md["mapping"]]
    n_spectrum = len(mdct[0]) if mdct else (
        1024 if md["blockflag"] else 128
    )

    ch_used: list[bool] = []
    residuals: list[list[float]] = []
    for ch in range(channels):
        sub = mapping["chmux"][ch] if mapping["submaps"] > 1 else 0
        floor = setup["floors"][mapping["floors"][sub]]
        posts = absolute_posts[ch] if ch < len(absolute_posts) else None
        if posts is None:
            op.write(0, 1)
            ch_used.append(False)
            residuals.append([0.0] * n_spectrum)
            continue
        op.write(1, 1)
        pl = postlist_from_floor(floor)
        rng = FLOOR1_RANGES[floor["multiplier"]]
        packet_posts = (
            floor1_quantize_posts(posts, floor["multiplier"])
            if posts_are_10bit
            else posts
        )
        raster_posts, Y = floor1_wrap_with_posts(packet_posts, pl, rng)
        pack_floor1_body(op, floor, books, Y)
        ch_used.append(True)
        # amplitude floor curve for residual
        # use unwrapped absolute posts (fit output already absolute)
        curve = floor1_curve_from_posts(
            raster_posts, pl, n_spectrum, floor["multiplier"]
        )
        residuals.append(mdct_to_residue(mdct[ch], curve))

    quantized_residue = [[0] * n_spectrum for _ in range(channels)]
    if any(ch_used):
        res = setup["residues"][mapping["residues"][0]]
        # Materialize the integer residue handoff once. The residue packer
        # accepts numeric rows and its own integer normalization is
        # idempotent, so these exact rows feed both classification and VQ.
        begin = int(res["begin"])
        end = min(int(res["end"]), n_spectrum)
        quantized_residue = [
            [
                quantize_residue_value(value)
                if used and begin <= index < end
                else 0
                for index, value in enumerate(row)
            ]
            for row, used in zip(residuals, ch_used)
        ]
        if residue_vq:
            pack_residue_vq(
                op,
                res,
                books,
                quantized_residue,
                ch_used,
                n_spectrum=n_spectrum,
            )
        else:
            pack_residue_silent(
                op, res, books, sum(1 for u in ch_used if u), n_spectrum=n_spectrum
            )
    return BlockPacketResult(
        op.get_buffer(),
        tuple(tuple(row) for row in quantized_residue),
    )


def pack_block_packet(
    setup: dict,
    books: Sequence[Codebook],
    channels: int,
    mode: int,
    absolute_posts: list[list[int] | None],
    mdct: list[list[float]],
    *,
    prev_window: int = 0,
    next_window: int = 0,
    residue_vq: bool = True,
    posts_are_10bit: bool = False,
) -> bytes:
    """Encode one audio block and return only its packet bytes."""
    return pack_block_packet_details(
        setup,
        books,
        channels,
        mode,
        absolute_posts,
        mdct,
        prev_window=prev_window,
        next_window=next_window,
        residue_vq=residue_vq,
        posts_are_10bit=posts_are_10bit,
    ).packet


@dataclass(frozen=True)
class EncodedPacket:
    """Packet bytes plus unwrapped 10-bit floor posts used to create them."""

    frame: PsyFrame
    posts: tuple[tuple[int, ...] | None, ...]
    packet: bytes
    quantized_residue: tuple[tuple[int, ...], ...]


def pack_analysis_frame(
    setup: dict,
    books: Sequence[Codebook],
    analysis: PsyFrame,
    *,
    channels: int,
) -> EncodedPacket:
    """Fit floor1 curves and pack floor/residue for one analysis frame."""
    if len(analysis.post) != channels or len(analysis.side) != channels:
        raise ValueError("analysis channel count differs from packet mapping")
    mode = analysis.window.current
    mapping = setup["maps"][setup["modes"][mode]["mapping"]]
    posts: list[list[int] | None] = []
    for channel, (post_curve, raw_curve) in enumerate(
        zip(analysis.post, analysis.raw_mdct)
    ):
        submap = mapping["chmux"][channel] if mapping["submaps"] > 1 else 0
        floor = setup["floors"][mapping["floors"][submap]]
        posts.append(
            floor1_fit_wwise(post_curve, raw_curve, floor, n=len(raw_curve))
        )
    packet_result = pack_block_packet_details(
        setup,
        books,
        channels,
        mode,
        posts,
        [list(row) for row in analysis.side],
        posts_are_10bit=True,
    )
    return EncodedPacket(
        analysis,
        tuple(None if row is None else tuple(row) for row in posts),
        packet_result.packet,
        packet_result.quantized_residue,
    )
