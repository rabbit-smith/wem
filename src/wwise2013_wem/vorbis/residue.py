#!/usr/bin/env python3
"""Vorbis residue type 0/1 packing and consumption for Wwise 2013.2.

The encoder may stop mid-residue; decoding treats EOF as end of residue.
"""
from __future__ import annotations

import math
from typing import List, Sequence

from .bitio import BitReader, OggPack
from .codebook import Codebook


class ResidueEOP(Exception):
    """End of packet while reading residue (libvorbis returns 0)."""


def _decode_eop(book: Codebook, br: BitReader) -> int:
    try:
        if br.bits_left() <= 0:
            raise ResidueEOP()
        return book.decode(br)
    except (EOFError, ValueError) as e:
        raise ResidueEOP() from e


def partitions_to_read(residue: dict, n_spectrum: int | None = None) -> int:
    """Number of partitions from begin/end (optionally clipped by spectrum n)."""
    begin = residue["begin"]
    end = residue["end"]
    part = residue["partition_size"]
    if n_spectrum is not None and end > n_spectrum:
        end = n_spectrum
    span = end - begin
    if span <= 0:
        return 0
    return span // part


def consume_residue(
    br: BitReader,
    residue: dict,
    books: Sequence[Codebook],
    ch_used: Sequence[bool],
    *,
    n_spectrum: int | None = None,
) -> dict:
    """
    Consume residue bits for type 0/1 (identical bitstream).

    ch_used[i] False ⇒ do_not_decode (floor zero).
    Returns status: complete | eop | empty.
    """
    start = br.tell_bits()
    npart = partitions_to_read(residue, n_spectrum)
    if npart <= 0 or not any(ch_used):
        return {
            "status": "empty",
            "npart": npart,
            "bit_start": start,
            "bit_end": start,
            "bits_left": br.bits_left(),
        }

    part = residue["partition_size"]
    cb = books[residue["classbook"]]
    classwords = cb.dim
    nclass = residue["classifications"]
    nch = len(ch_used)
    partword = [[0] * npart for _ in range(nch)]

    try:
        # The phrasebook is interleaved with stage 0: for each group of
        # ``classwords`` partitions, every active channel emits one phrase
        # entry and the group is immediately processed.  It is not legal to
        # read every phrase entry up front and then process every stage.
        #
        # A phrase entry stores its first partition in the *highest* radix
        # position (the inverse of the old ``temp % nclass`` assumption).
        # This is the schedule used by libvorbis _01inverse/_01forward.
        vq = 0
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
                        nvec = part // book.dim
                        for _ in range(nvec):
                            _decode_eop(book, br)
                            vq += 1

        return {
            "status": "complete",
            "npart": npart,
            "vq_reads": vq,
            "bit_start": start,
            "bit_end": br.tell_bits(),
            "bits_left": br.bits_left(),
            "partword0": partword[0][: min(8, npart)],
        }
    except ResidueEOP:
        return {
            "status": "eop",
            "npart": npart,
            "bit_start": start,
            "bit_end": br.tell_bits(),
            "bits_left": br.bits_left(),
            "partword0": partword[0][: min(8, npart)],
        }


def pack_residue_silent(
    op: OggPack,
    residue: dict,
    books: Sequence[Codebook],
    n_channels: int,
    *,
    n_spectrum: int | None = None,
) -> None:
    """
    Pack a silent residue: all partitions class 0.

    Requires cascade[0] == 0 (no stage books) so no VQ follows.
    The installed profile's silent class has an empty cascade.
    """
    if residue["cascades"][0] != 0:
        raise ValueError("silent residue requires cascade class0 empty")
    npart = partitions_to_read(residue, n_spectrum)
    cb = books[residue["classbook"]]
    classwords = cb.dim
    for i in range(0, npart, classwords):
        for _ch in range(n_channels):
            cb.encode(op, 0)


WWISE_RESIDUE_44_LOW_UN_METRICS = (
    # Encoder-only maximum-coefficient thresholds for the 44.1-kHz
    # uncoupled low-residue template. This table is deliberately absent from
    # the serialized Vorbis setup packet.
    (0, 1, 1, 2, 2, 4, 28),
    # Encoder-only mean-absolute-coefficient thresholds. A negative value
    # disables the average-absolute-value gate for that class.
    (-1, 25, -1, 45, -1, -1, -1),
)


def quantize_residue_value(value: float) -> int:
    """Quantize a residue coefficient using nearest-even rounding."""
    # Python's round implements the required nearest-even rule. Cast to int
    # to keep later arithmetic and codebook selection explicitly integral.
    return int(round(float(value)))


def _classify_partition_heuristic(samples: Sequence[float], nclass: int) -> int:
    """Bounded fallback for a setup with no installed class-metric table."""
    if not samples:
        return 0
    peak = 0.0
    energy = 0.0
    for s in samples:
        a = abs(s)
        if a > peak:
            peak = a
        energy += s * s
    if peak < 1e-5 and energy < 1e-8:
        return 0
    # log peak → 1..nclass-1
    e = math.log10(peak + 1e-12)
    # peak ~1e-4 → low class, peak ~1 → high
    t = (e + 5.0) / 5.0  # roughly 0..1 for e in [-5,0]
    c = 1 + int(t * (nclass - 2))
    if c < 1:
        c = 1
    if c >= nclass:
        c = nclass - 1
    # prefer classes that have cascade bits in typical Wwise tables
    return c


def classify_partition(samples: Sequence[float], nclass: int = 8) -> int:
    """Select the Wwise residue class for one partition.

    Classification operates on the already integer-quantized residue, scans
    class 0 through
    ``nclass - 2``, and selects the first class satisfying both its maximum
    absolute coefficient and (when enabled) mean-absolute coefficient gates.
    The 44.1-kHz uncoupled low template has eight classes and the
    installed metrics above. Other setups retain the bounded fallback until
    their encoder-only metric table is available.
    """
    if nclass != 8:
        return _classify_partition_heuristic(samples, nclass)

    values = [quantize_residue_value(sample) for sample in samples]
    if not values:
        return 0
    peak = max(abs(value) for value in values)
    average_x100 = (100 * sum(abs(value) for value in values)) // len(values)
    max_metrics, avg_metrics = WWISE_RESIDUE_44_LOW_UN_METRICS
    for classification, (max_metric, avg_metric) in enumerate(
        zip(max_metrics, avg_metrics)
    ):
        if peak <= max_metric and (
            avg_metric < 0 or average_x100 < avg_metric
        ):
            return classification
    return nclass - 1


def _pack_classbook_entry(
    op: OggPack,
    cb: Codebook,
    classes: Sequence[int],
    nclass: int,
) -> None:
    """Pack classwords classifications as mixed-radix classbook entry."""
    # libvorbis phrasebook values are most-significant-first:
    # entry = (...((c0 * nclass) + c1) * nclass + c2)... .  Consequently
    # decode maps the highest radix digit to the earliest partition.
    entry = 0
    for c in classes:
        entry = entry * nclass + (c % nclass)
    if entry >= cb.entries or cb.lengthlist[entry] <= 0:
        # clamp to valid: use all-zero if broken
        entry = 0
        if cb.lengthlist[0] <= 0:
            # nearest used
            for e, L in enumerate(cb.lengthlist):
                if L > 0:
                    entry = e
                    break
    cb.encode(op, entry)


def pack_residue_vq(
    op: OggPack,
    residue: dict,
    books: Sequence[Codebook],
    residuals: Sequence[Sequence[float]],
    ch_used: Sequence[bool],
    *,
    n_spectrum: int | None = None,
) -> dict:
    """
    Pack residue type 0/1 with greedy multi-stage VQ.

    residuals[ch][bin]: full spectrum residual (floor-divided MDCT).
    Only bins [begin, end) are coded; ch_used gates channels.

    Returns stats: npart, classes histogram, vq_count, bits written estimate.
    """
    begin = residue["begin"]
    end = residue["end"]
    part = residue["partition_size"]
    npart = partitions_to_read(residue, n_spectrum)
    if npart <= 0 or not any(ch_used):
        return {"status": "empty", "npart": 0, "vq_count": 0}

    nclass = residue["classifications"]
    cascades = residue["cascades"]
    stage_books = residue["books"]  # [class][stage] -> book or -1
    cb = books[residue["classbook"]]
    classwords = cb.dim
    nch = len(ch_used)

    # working residual copies (mutable)
    work: List[List[float]] = []
    for ch in range(nch):
        if ch_used[ch]:
            # Wwise's mapping-forward path converts the floor-divided MDCT to
            # integer residue before both classing and VQ.  Keep the mutable
            # work surface numeric for ``best_vq`` while preserving those
            # exact integral inputs.
            row = [float(quantize_residue_value(value)) for value in residuals[ch]]
            # ensure length
            if len(row) < end:
                row = row + [0.0] * (end - len(row))
            work.append(row)
        else:
            work.append([])

    # classify each partition per used channel
    partword = [[0] * npart for _ in range(nch)]
    class_hist = [0] * nclass
    for j in range(nch):
        if not ch_used[j]:
            continue
        for i in range(npart):
            off = begin + i * part
            seg = work[j][off : off + part]
            c = classify_partition(seg, nclass)
            # if cascade empty for that class, fall back
            if cascades[c] == 0 and c != 0:
                # try nearest class with cascade
                for alt in range(nclass - 1, 0, -1):
                    if cascades[alt]:
                        c = alt
                        break
            partword[j][i] = c
            class_hist[c] += 1

    # Interleave the phrasebook with stage 0 exactly as _01forward does.
    # This matters for bit identity: phrase entries are not a contiguous
    # prefix followed by all VQ vectors.
    vq_count = 0
    for s in range(8):
        for i in range(0, npart, classwords):
            if s == 0:
                for j in range(nch):
                    if not ch_used[j]:
                        continue
                    group = [
                        partword[j][i + k] if i + k < npart else 0
                        for k in range(classwords)
                    ]
                    _pack_classbook_entry(op, cb, group, nclass)

            for k in range(classwords):
                partition = i + k
                if partition >= npart:
                    break
                for j in range(nch):
                    if not ch_used[j]:
                        continue
                    pclass = partword[j][partition]
                    book_id = stage_books[pclass][s]
                    if book_id < 0:
                        continue
                    book = books[book_id]
                    dim = book.dim
                    off = begin + partition * part
                    for v in range(0, part, dim):
                        vec = work[j][off + v : off + v + dim]
                        if len(vec) < dim:
                            vec = list(vec) + [0.0] * (dim - len(vec))
                        entry = book.best_vq(vec)
                        book.encode(op, entry)
                        vq_vec = book.vq_values(entry)
                        for d in range(dim):
                            work[j][off + v + d] -= vq_vec[d]
                        vq_count += 1

    return {
        "status": "ok",
        "npart": npart,
        "vq_count": vq_count,
        "class_hist": class_hist,
        "partword0": partword[0][: min(8, npart)] if nch else [],
    }


def mdct_to_residue(
    mdct: Sequence[float],
    floor_amp: Sequence[float],
    *,
    floor_eps: float = 1e-8,
) -> list[float]:
    """Multiplicative floor: residue = mdct / floor_amp (vorbis convention)."""
    n = min(len(mdct), len(floor_amp))
    out = []
    for i in range(n):
        f = floor_amp[i]
        if abs(f) < floor_eps:
            out.append(0.0)
        else:
            out.append(mdct[i] / f)
    if len(mdct) > n:
        out.extend(mdct[n:])
    return out
