#!/usr/bin/env python3
"""Vorbis residue type 0/1 packing and consumption for Wwise 2013.2.

The encoder may stop mid-residue; decoding treats EOF as end of residue.
"""
from __future__ import annotations

from typing import List, Sequence

from .bitio import BitReader, OggPack
from .codebook import Codebook
from .._tmath import log10_f64


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
    e = log10_f64(peak + 1e-12)
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


def _vv_add_slots(offset: int, part: int, n_channels: int, dim: int):
    """Yield per-entry (bin, channel) slot lists in the libvorbis
    decodevv_add flat-domain order (bin-major, channel round-robin)."""
    i = offset // n_channels
    chptr = 0
    m = (offset + part) // n_channels
    while i < m:
        slots = []
        for _ in range(dim):
            if i >= m:
                break
            slots.append((i, chptr))
            chptr += 1
            if chptr == n_channels:
                chptr = 0
                i += 1
        yield slots


#: Encoder-only class metrics for the type-2 (flat-domain) residue template,
#: read from the paired build `.data`: the type-2 configuration record at
#: the build's code carries its magnitude metrics at the build's code and its angle
#: metrics at the build's code (the golden's type-1 pair sits the same way at
#: the build's code / the build's code relative to the build's code).  A negative angle metric
#: disables that class's angle gate, the same convention as the validated
#: type-1 metric table.
WWISE_RESIDUE_TYPE2_CLASS_METRICS: tuple[tuple[int, ...], tuple[int, ...]] = (
    (0, 1, 1, 2, 2, 4, 4, 16, 60),
    (-1, 30, -1, 50, -1, 80, -1, -1, -1),
)

#: Channel count of the type-2 flat classification domain. The reference
#: classifier de-interleaves ``bin * channels + channel``; every installed
#: profile is stereo.
_TYPE2_CLASSIFY_CHANNELS = 2


def _classify_partition_type2(
    samples_flat: Sequence[float],
    nclass: int,
    channels: int = _TYPE2_CLASSIFY_CHANNELS,
    long_block: bool = False,
) -> int:
    """Classify one type-2 partition (reference ``_2class``).

    The reference classifier takes two peaks over the partition's flat
    ``(bin * channels + channel)`` domain: the maximum absolute coefficient of
    channel 0, and the maximum over every other channel.  It scans the class
    metrics in order and takes the first class whose magnitude and angle gates
    both hold, otherwise the last class.  ``long_block`` is unused.
    """
    del long_block  # the reference type-2 classifier has no block dependence
    if not samples_flat or channels <= 0:
        return 0
    values = [quantize_residue_value(sample) for sample in samples_flat]
    magnitude_peak = 0
    angle_peak = 0
    for index, value in enumerate(values):
        magnitude = abs(value)
        if index % channels == 0:
            if magnitude > magnitude_peak:
                magnitude_peak = magnitude
        elif magnitude > angle_peak:
            angle_peak = magnitude
    max_metrics, angle_metrics = WWISE_RESIDUE_TYPE2_CLASS_METRICS
    for classification, (max_metric, angle_metric) in enumerate(
        zip(max_metrics, angle_metrics)
    ):
        if magnitude_peak <= max_metric and (
            angle_metric < 0 or angle_peak <= angle_metric
        ):
            return classification
    return nclass - 1


def _pack_classbook_entry_type2(
    op: OggPack, cb: Codebook, classes: Sequence[int], nclass: int
) -> None:
    """Mixed-radix classbook entry for the type-2 phrasebook.

    entry = c0 * nclass^(ppw-1) + ... + c_{ppw-1}; the decoder unpacks the
    same mixed radix.  A classbook only codes a subset of the mixed-radix
    domain (e.g. the 2ch short classbook carries classes 4..9 only); an
    uncoded combination falls back to the smallest coded entry, matching
    the calibrated 2ch behavior.
    """
    entry = 0
    for c in classes:
        entry = entry * nclass + (int(c) % nclass)
    if entry >= cb.entries or int(cb.lengthlist[entry]) <= 0:
        entry = next(
            e for e, length in enumerate(cb.lengthlist) if int(length) > 0
        )
    cb.encode(op, entry)


def pack_residue_type2(
    op: OggPack,
    residue: dict,
    books: Sequence[Codebook],
    residuals: Sequence[Sequence[float]],
    ch_used: Sequence[bool],
    *,
    n_spectrum: int,
    n_channels: int,
    long_block: bool = False,
) -> dict:
    """Pack residue type 2 (the libvorbis res2_inverse flat-domain layout).

    Type 2 codes the residue on the flat (bin * n_channels + channel)
    domain: one class per partition shared across all channels, classword
    written once per partition-group, VQ vectors slot-major over
    (bin, channel). Mirrors the decoder's res2_inverse bit schedule
    (scripts/decode_wem.py::decode_residue_type2), so encoded packets
    close strictly against that published decoder.

    residuals[ch][bin]: coupled residue rows (floor-divided MDCT, mapping0
    coupling already applied).
    """
    max_end = n_spectrum * n_channels
    begin = int(residue["begin"])
    end = int(residue["end"]) if int(residue["end"]) < max_end else max_end
    span = end - begin
    part = int(residue["partition_size"])
    partvals = span // part if span > 0 else 0
    if partvals <= 0 or not any(ch_used):
        return {"status": "empty", "partvals": 0, "vq_count": 0}

    nclass = int(residue["classifications"])
    cascades = residue["cascades"]
    stage_books = residue["books"]  # [class][stage] -> book or -1
    cb = books[int(residue["classbook"])]
    ppw = int(cb.dim)
    if ppw <= 0:
        ppw = 1
    partwords = (partvals + ppw - 1) // ppw

    # working residual copies (mutable), quantized as the forward path does
    work: List[List[float]] = []
    for ch in range(n_channels):
        if ch_used[ch]:
            work.append(
                [float(quantize_residue_value(v)) for v in residuals[ch]]
            )
        else:
            work.append([])

    # classify each partition (class shared across channels), gathering in
    # flat (bin*ch + ch) order so the metric sees the values the VQ stages
    # will write (mirror of the _vv_add_slots layout)
    partword: List[List[int]] = [[0] * ppw for _ in range(partwords)]
    for lw in range(partwords):
        for k in range(ppw):
            p = lw * ppw + k
            if p >= partvals:
                break
            off = begin + p * part
            flat = []
            for f in range(off, off + part):
                bin_idx = f // n_channels
                ch_idx = f % n_channels
                if ch_used[ch_idx] and bin_idx < len(work[ch_idx]):
                    flat.append(work[ch_idx][bin_idx])
            partword[lw][k] = _classify_partition_type2(
                flat, nclass, n_channels, long_block
            )

    vq_count = 0
    for s in range(8):
        for lw in range(partwords):
            if s == 0:
                classes = partword[lw][:ppw]
                while len(classes) < ppw:
                    classes = classes + [0]
                _pack_classbook_entry_type2(op, cb, classes, nclass)
            for k in range(ppw):
                p = lw * ppw + k
                if p >= partvals:
                    break
                cls = partword[lw][k]
                if not (int(cascades[cls]) & (1 << s)):
                    continue
                book_id = int(stage_books[cls][s])
                if book_id < 0:
                    continue
                book = books[book_id]
                dim = int(book.dim)
                off = begin + p * part
                for slots in _vv_add_slots(off, part, n_channels, dim):
                    target = [0.0] * dim
                    for j, (bin_idx, ch_idx) in enumerate(slots[:dim]):
                        if ch_idx < len(work) and bin_idx < len(work[ch_idx]):
                            target[j] = work[ch_idx][bin_idx]
                        else:
                            target[j] = 0.0
                    entry = book.best_vq(target)
                    book.encode(op, entry)
                    vq_vec = book.vq_values(entry)
                    for (bin_idx, ch_idx), val in zip(slots[:dim], vq_vec):
                        if ch_idx < len(work) and bin_idx < len(work[ch_idx]):
                            work[ch_idx][bin_idx] -= val
                    vq_count += 1

    return {
        "status": "ok",
        "partvals": partvals,
        "vq_count": vq_count,
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
