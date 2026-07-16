"""Pure Vorbis Huffman and maptype-1 VQ codebook core."""

from __future__ import annotations

import math
import struct
from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Sequence, Tuple

from .bitio import BitReader, OggPack


def ilog(v: int) -> int:
    r = 0
    x = v
    while x:
        r += 1
        x >>= 1
    return r


@dataclass
class StaticCodebook:
    dim: int
    entries: int
    lengthlist: List[int]
    maptype: int = 0
    q_min: int = 0
    q_delta: int = 0
    q_quant: int = 0
    q_sequencep: int = 0
    quantlist: Optional[List[int]] = None

    def is_ordered(self) -> bool:
        n = self.entries
        lengths = self.lengthlist
        if n <= 1:
            return True
        if lengths[0] == 0:
            return False
        i = 1
        while i < n:
            if lengths[i] < lengths[i - 1]:
                return False
            i += 1
        return True

    def pack(self) -> bytes:
        """Pack the Wwise static-codebook representation."""
        op = OggPack()
        op.write(self.dim, 4)
        op.write(self.entries, 14)
        lengths = self.lengthlist
        n = self.entries
        max_len = max(lengths) if lengths else 0
        nob = ilog(max_len)

        if self.is_ordered():
            op.write(1, 1)
            op.write(lengths[0] - 1, 5)
            this = 0
            i = 1
            while i < n:
                if lengths[i] > lengths[i - 1]:
                    num = lengths[i] - lengths[i - 1]
                    while num > 0:
                        op.write(i - this, ilog(n - this))
                        this = i
                        num -= 1
                i += 1
            op.write(i - this, ilog(n - this))
        else:
            op.write(0, 1)
            op.write(nob, 3)
            first_unused = 0
            while first_unused < n and lengths[first_unused] != 0:
                first_unused += 1
            if first_unused == n:
                op.write(0, 1)
                for L in lengths:
                    op.write(L - 1, nob)
            else:
                op.write(1, 1)
                for L in lengths:
                    if L:
                        op.write(1, 1)
                        op.write(L - 1, nob)
                    else:
                        op.write(0, 1)

        # Wwise stores maptype as one bit: zero or non-zero.
        op.write(1 if self.maptype else 0, 1)
        if self.maptype == 0:
            return op.get_buffer()

        op.write(self.q_min & 0xFFFFFFFF, 32)
        op.write(self.q_delta & 0xFFFFFFFF, 32)
        op.write((self.q_quant - 1) & 0xF, 4)
        op.write(self.q_sequencep & 1, 1)
        if self.quantlist:
            for v in self.quantlist:
                av = -v if v < 0 else v
                bits = self.q_quant
                mask = (1 << bits) - 1 if bits < 32 else 0xFFFFFFFF
                op.write(av & mask, bits)
        return op.get_buffer()


def float32_unpack(val: int) -> float:
    """Vorbis non-IEEE float32_unpack (spec / libvorbis sharedbook)."""
    v = val & 0xFFFFFFFF
    mant = v & 0x1FFFFF
    sign = v & 0x80000000
    exp = (v & 0x7FE00000) >> 21
    if sign:
        mant = -mant
    return math.ldexp(float(mant), exp - 20 - 768)


def ieee_float32_bits(val: int) -> float:
    """Interpret uint32 bit pattern as IEEE float32 (not used by Vorbis q_*)."""
    return struct.unpack("<f", struct.pack("<I", val & 0xFFFFFFFF))[0]


def book_maptype1_quantvals(entries: int, dim: int) -> int:
    """Largest r such that r**dim <= entries (libvorbis _book_maptype1_quantvals)."""
    if dim < 1:
        raise ValueError("dim must be >= 1")
    if entries < 1:
        return 0
    vals = int(math.floor(entries ** (1.0 / dim)))
    # Float pow can be off by one; walk until r^dim <= n < (r+1)^dim
    while True:
        acc = 1
        acc1 = 1
        for _ in range(dim):
            acc *= vals
            acc1 *= vals + 1
        if acc <= entries and acc1 > entries:
            return vals
        if acc > entries:
            vals -= 1
            if vals < 0:
                return 0
        else:
            vals += 1


def make_codewords(lengthlist: Sequence[int]) -> List[int]:
    """
    Assign Huffman codewords from lengths (libvorbis _make_words, sparsecount=0).

    Returns one codeword per entry (0 if unused / length 0). Words are
    bit-reversed for LSB-first pack/unpack.
    """
    n = len(lengthlist)
    marker = [0] * 33
    # First pass: MSB-first tree codes into r[0..used)
    r: List[int] = [0] * n
    count = 0
    for i in range(n):
        length = int(lengthlist[i])
        if length > 0:
            entry = marker[length]
            if length < 32 and (entry >> length):
                raise ValueError(
                    f"overpopulated Huffman tree at entry {i} length {length}"
                )
            r[count] = entry
            count += 1

            for j in range(length, 0, -1):
                if marker[j] & 1:
                    if j == 1:
                        marker[1] += 1
                    else:
                        marker[j] = marker[j - 1] << 1
                    break
                marker[j] += 1

            for j in range(length + 1, 33):
                if (marker[j] >> 1) == entry:
                    entry = marker[j]
                    marker[j] = marker[j - 1] << 1
                else:
                    break
        else:
            # unused: still advance dense slot so second pass indexes align
            count += 1

    # Bit-reverse into dense slots (all entries when sparsecount=0)
    out = [0] * n
    count = 0
    for i in range(n):
        length = int(lengthlist[i])
        temp = 0
        for j in range(length):
            temp <<= 1
            temp |= (r[count] >> j) & 1
        out[count] = temp
        count += 1
    return out


def _build_decode_tree(
    codelist: Sequence[int], lengthlist: Sequence[int]
) -> Dict[int, Any]:
    """
    Binary tree for LSB-first bit walk.
    Internal nodes: {0: child, 1: child}; leaves: entry index (int).
    Represented as nested dicts; leaf is int.
    """
    root: Dict[int, Any] = {}
    for entry, (code, length) in enumerate(zip(codelist, lengthlist)):
        length = int(length)
        if length <= 0:
            continue
        node: Any = root
        for b in range(length):
            bit = (code >> b) & 1
            if b == length - 1:
                if bit in node and not isinstance(node[bit], int):
                    raise ValueError(f"code prefix collision at entry {entry}")
                node[bit] = entry
            else:
                child = node.get(bit)
                if child is None:
                    child = {}
                    node[bit] = child
                elif isinstance(child, int):
                    raise ValueError(
                        f"code embeds earlier leaf (entry {child}) at {entry}"
                    )
                node = child
    return root


def _book_unquantize_maptype1(
    entries: int,
    dim: int,
    quantlist: Sequence[int],
    q_min: int,
    q_delta: int,
    q_sequencep: int,
    lengthlist: Optional[Sequence[int]] = None,
) -> List[Optional[List[float]]]:
    """
    libvorbis _book_unquantize for maptype 1 (full dense valuallist).

    Unused entries (length 0) get None. Sequential adds last to each step.
    value = |quantlist[index]| * delta + mindel [+ last]
    """
    quantvals = book_maptype1_quantvals(entries, dim)
    if quantvals <= 0:
        raise ValueError("quantvals must be positive for maptype1")
    if len(quantlist) < quantvals:
        raise ValueError(
            f"quantlist length {len(quantlist)} < quantvals {quantvals}"
        )
    mindel = float32_unpack(q_min)
    delta = float32_unpack(q_delta)
    out: List[Optional[List[float]]] = [None] * entries
    for j in range(entries):
        if lengthlist is not None and int(lengthlist[j]) <= 0:
            continue
        last = 0.0
        vals: List[float] = []
        indexdiv = 1
        for _k in range(dim):
            index = (j // indexdiv) % quantvals
            q = quantlist[index]
            val = abs(q) * delta + mindel + last
            if q_sequencep:
                last = val
            vals.append(val)
            indexdiv *= quantvals
        out[j] = vals
    return out


@dataclass
class Codebook:
    """Runtime Huffman + optional VQ lattice codebook."""

    static: StaticCodebook
    book_id: Optional[int] = None
    table: Optional[str] = None
    index: Optional[int] = None
    codelist: List[int] = field(default_factory=list)
    # decode tree root
    _tree: Dict[int, Any] = field(default_factory=dict, repr=False)
    # maptype1: per-entry VQ vector or None if unused
    valuallist: Optional[List[Optional[List[float]]]] = None
    quantvals: int = 0

    @property
    def dim(self) -> int:
        return self.static.dim

    @property
    def entries(self) -> int:
        return self.static.entries

    @property
    def maptype(self) -> int:
        return self.static.maptype

    @property
    def lengthlist(self) -> List[int]:
        return self.static.lengthlist

    def used_entries(self) -> List[int]:
        return [i for i, L in enumerate(self.lengthlist) if L > 0]

    def encode(self, op: OggPack, entry: int) -> None:
        """Write Huffman code for entry index (maptype 0 or 1 index)."""
        if entry < 0 or entry >= self.entries:
            raise IndexError(f"entry {entry} out of range 0..{self.entries - 1}")
        length = int(self.lengthlist[entry])
        if length <= 0:
            raise ValueError(f"entry {entry} is unused (length 0)")
        op.write(self.codelist[entry], length)

    def decode(self, br: BitReader) -> int:
        """Decode one Huffman entry index from the bit stream."""
        node: Any = self._tree
        if not node:
            raise ValueError("empty codebook (no used entries)")
        while True:
            bit = br.read(1)
            if bit not in node:
                raise ValueError("invalid Huffman code (no matching entry)")
            nxt = node[bit]
            if isinstance(nxt, int):
                return nxt
            node = nxt

    def decode_vq(self, br: BitReader) -> List[float]:
        """Decode entry and return VQ vector (maptype 1). maptype 0 raises."""
        if self.maptype != 1:
            raise ValueError(f"decode_vq requires maptype 1, got {self.maptype}")
        entry = self.decode(br)
        if self.valuallist is None:
            raise ValueError("no valuallist")
        vec = self.valuallist[entry]
        if vec is None:
            raise ValueError(f"entry {entry} has no VQ vector (unused)")
        return list(vec)

    def vq_values(self, entry: int) -> List[float]:
        """Return unquantized VQ vector for entry (maptype 1)."""
        if self.maptype != 1 or self.valuallist is None:
            raise ValueError("vq_values requires maptype 1 with valuallist")
        vec = self.valuallist[entry]
        if vec is None:
            raise ValueError(f"entry {entry} unused")
        return list(vec)

    def _ensure_vq_cache(self) -> None:
        """Build list of (entry, vector) for used maptype1 entries."""
        if getattr(self, "_vq_cache", None) is not None:
            return
        cache: List[Tuple[int, List[float]]] = []
        if self.valuallist is None:
            self._vq_cache = cache  # type: ignore[attr-defined]
            return
        for e, L in enumerate(self.lengthlist):
            if L <= 0:
                continue
            vec = self.valuallist[e]
            if vec is None:
                continue
            cache.append((e, list(vec)))
        self._vq_cache = cache  # type: ignore[attr-defined]
        # optional numpy matrix (entries × dim)
        try:
            import numpy as np

            if cache:
                self._vq_mat = np.asarray([v for _, v in cache], dtype=np.float64)  # type: ignore[attr-defined]
                self._vq_ids = [e for e, _ in cache]  # type: ignore[attr-defined]
            else:
                self._vq_mat = None  # type: ignore[attr-defined]
                self._vq_ids = []  # type: ignore[attr-defined]
        except ImportError:
            self._vq_mat = None  # type: ignore[attr-defined]
            self._vq_ids = []  # type: ignore[attr-defined]

    def best_vq(self, target: Sequence[float]) -> int:
        """Nearest used maptype1 entry (squared Euclidean)."""
        if self.maptype != 1 or self.valuallist is None:
            raise ValueError("best_vq requires maptype 1 with valuallist")
        dim = self.dim
        if len(target) < dim:
            raise ValueError(f"target len {len(target)} < dim {dim}")
        self._ensure_vq_cache()
        mat = getattr(self, "_vq_mat", None)
        ids = getattr(self, "_vq_ids", None)
        if mat is not None and ids:
            import numpy as np

            t = np.asarray(target[:dim], dtype=np.float64)
            # squared distances
            d = mat - t
            err = np.einsum("ij,ij->i", d, d)
            return int(ids[int(np.argmin(err))])
        best_e = -1
        best_err = float("inf")
        for e, vec in self._vq_cache:  # type: ignore[attr-defined]
            err = 0.0
            for i in range(dim):
                d = target[i] - vec[i]
                err += d * d
            if err < best_err:
                best_err = err
                best_e = e
                if err == 0.0:
                    break
        if best_e < 0:
            raise ValueError("no usable VQ entries")
        return best_e


def static_from_entry_dict(b: dict) -> StaticCodebook:
    """Build StaticCodebook from a decoded JSON book entry."""
    lengthlist = list(b.get("lengthlist") or [])
    entries = int(b["entries"])
    if len(lengthlist) != entries:
        raise ValueError(
            f"lengthlist len {len(lengthlist)} != entries {entries}"
        )
    quantlist = b.get("quantlist")
    return StaticCodebook(
        dim=int(b["dim"]),
        entries=entries,
        lengthlist=[int(x) for x in lengthlist],
        maptype=int(b.get("maptype") or 0),
        q_min=int(b.get("q_min") or 0),
        q_delta=int(b.get("q_delta") or 0),
        q_quant=int(b.get("q_quant") or 0),
        q_sequencep=int(b.get("q_sequencep") or 0),
        quantlist=[int(x) for x in quantlist] if quantlist is not None else None,
    )


def codebook_from_static(
    sc: StaticCodebook,
    *,
    book_id: Optional[int] = None,
    table: Optional[str] = None,
    index: Optional[int] = None,
) -> Codebook:
    """Build runtime Codebook (Huffman tree + optional VQ) from static fields."""
    codelist = make_codewords(sc.lengthlist)
    tree = _build_decode_tree(codelist, sc.lengthlist)
    valuallist: Optional[List[Optional[List[float]]]] = None
    quantvals = 0
    if sc.maptype == 1:
        if not sc.quantlist:
            raise ValueError("maptype 1 requires quantlist")
        quantvals = book_maptype1_quantvals(sc.entries, sc.dim)
        valuallist = _book_unquantize_maptype1(
            sc.entries,
            sc.dim,
            sc.quantlist,
            sc.q_min,
            sc.q_delta,
            sc.q_sequencep,
            sc.lengthlist,
        )
    return Codebook(
        static=sc,
        book_id=book_id,
        table=table,
        index=index,
        codelist=codelist,
        _tree=tree,
        valuallist=valuallist,
        quantvals=quantvals,
    )
