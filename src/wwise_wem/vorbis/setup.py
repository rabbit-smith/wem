"""Pure Wwise Vorbis setup packet syntax codec."""

from __future__ import annotations

from .bitio import BitReader, OggPack


def ilog(n: int) -> int:
    """Bits needed to store values in [0, n] inclusive when n>=0 (Vorbis-style)."""
    r = 0
    v = n
    while v:
        r += 1
        v >>= 1
    return r


def parse_floor1(br: BitReader) -> dict:
    """Parse a floor1 setup body; Wwise omits the floor type field."""
    start = br.tell_bits()
    partitions = br.read(5)
    partition_classes = [br.read(4) for _ in range(partitions)]
    max_class = max(partition_classes) if partition_classes else -1

    class_dims: list[int] = []
    class_subs: list[int] = []
    class_masterbooks: list[int | None] = []
    subclass_books: list[list[int]] = []

    for _c in range(max_class + 1):
        dim = br.read(3) + 1
        subs = br.read(2)
        class_dims.append(dim)
        class_subs.append(subs)
        master = br.read(8) if subs else None
        class_masterbooks.append(master)
        sbooks = [br.read(8) - 1 for _ in range(1 << subs)]
        subclass_books.append(sbooks)

    multiplier = br.read(2) + 1
    rangebits = br.read(4)
    n_x = sum(class_dims[partition_classes[p]] for p in range(partitions))
    x_list = [br.read(rangebits) for _ in range(n_x)]

    return {
        "type": 1,
        "partitions": partitions,
        "partition_classes": partition_classes,
        "max_class": max_class,
        "class_dims": class_dims,
        "class_subs": class_subs,
        "class_masterbooks": class_masterbooks,
        "subclass_books": subclass_books,
        "multiplier": multiplier,
        "rangebits": rangebits,
        "x_list": x_list,
        "bit_start": start,
        "bit_end": br.tell_bits(),
    }


def parse_residue(br: BitReader) -> dict:
    """Parse a residue setup body; types 1 and 2 share this layout."""
    start = br.tell_bits()
    rtype = br.read(2)
    begin = br.read(24)
    end = br.read(24)
    partition_size = br.read(24) + 1
    classifications = br.read(6) + 1
    classbook = br.read(8)

    cascades: list[int] = []
    for _ in range(classifications):
        # Writer: if bitlen(cascade)>3 → low3 + flag1 + high5; else → 4-bit cascade.
        # Reader-compatible form (cascade < 8 keeps flag=0 as bit3 of the 4-bit write):
        low = br.read(3)
        flag = br.read(1)
        high = br.read(5) if flag else 0
        cascades.append(high * 8 + low)

    books: list[list[int]] = []
    for cascade in cascades:
        row: list[int] = []
        for k in range(8):
            if cascade & (1 << k):
                row.append(br.read(8))
            else:
                row.append(-1)
        books.append(row)

    return {
        "type": rtype,
        "begin": begin,
        "end": end,
        "partition_size": partition_size,
        "classifications": classifications,
        "classbook": classbook,
        "cascades": cascades,
        "books": books,
        "bit_start": start,
        "bit_end": br.tell_bits(),
    }


def parse_mapping0(br: BitReader, channels: int) -> dict:
    """Parse mapping type 0; Wwise omits the mapping type field."""
    start = br.tell_bits()
    if br.read(1):
        submaps = br.read(4) + 1
    else:
        submaps = 1

    coupling: list[dict] = []
    if br.read(1):
        steps = br.read(8) + 1
        chbits = ilog(channels - 1)
        for _ in range(steps):
            coupling.append({"mag": br.read(chbits), "ang": br.read(chbits)})
    reserved = br.read(2)

    if submaps > 1:
        chmux = [br.read(4) for _ in range(channels)]
    else:
        chmux = [0] * channels

    floors: list[int] = []
    residues: list[int] = []
    for _ in range(submaps):
        _unused = br.read(8)  # always 0 in pack
        floors.append(br.read(8))
        residues.append(br.read(8))

    return {
        "type": 0,
        "submaps": submaps,
        "coupling": coupling,
        "reserved": reserved,
        "chmux": chmux,
        "floors": floors,
        "residues": residues,
        "bit_start": start,
        "bit_end": br.tell_bits(),
    }


def parse_mode(br: BitReader) -> dict:
    """blockflag u1 + mapping u8 (window/transform omitted, always 0)."""
    start = br.tell_bits()
    blockflag = br.read(1)
    mapping = br.read(8)
    return {
        "blockflag": blockflag,
        "windowtype": 0,
        "transformtype": 0,
        "mapping": mapping,
        "bit_start": start,
        "bit_end": br.tell_bits(),
    }


def pack_floor1(op: OggPack, fl: dict) -> None:
    op.write(fl["partitions"], 5)
    for c in fl["partition_classes"]:
        op.write(c, 4)
    for i in range(fl["max_class"] + 1):
        op.write(fl["class_dims"][i] - 1, 3)
        subs = fl["class_subs"][i]
        op.write(subs, 2)
        if subs:
            op.write(fl["class_masterbooks"][i], 8)
        for b in fl["subclass_books"][i]:
            op.write(b + 1, 8)
    op.write(fl["multiplier"] - 1, 2)
    op.write(fl["rangebits"], 4)
    for x in fl["x_list"]:
        op.write(x, fl["rangebits"])


def pack_residue(op: OggPack, rs: dict) -> None:
    op.write(rs["type"], 2)
    op.write(rs["begin"], 24)
    op.write(rs["end"], 24)
    op.write(rs["partition_size"] - 1, 24)
    op.write(rs["classifications"] - 1, 6)
    op.write(rs["classbook"], 8)
    for cascade in rs["cascades"]:
        bitlen = 0
        v = cascade
        while v:
            bitlen += 1
            v >>= 1
        if bitlen > 3:
            op.write(cascade & 7, 3)
            op.write(1, 1)
            op.write(cascade >> 3, 5)
        else:
            op.write(cascade, 4)
    for row, cascade in zip(rs["books"], rs["cascades"]):
        for k in range(8):
            if cascade & (1 << k):
                op.write(row[k], 8)


def pack_mapping0(op: OggPack, mp: dict, channels: int) -> None:
    if mp["submaps"] <= 1:
        op.write(0, 1)
    else:
        op.write(1, 1)
        op.write(mp["submaps"] - 1, 4)
    if not mp["coupling"]:
        op.write(0, 1)
    else:
        op.write(1, 1)
        op.write(len(mp["coupling"]) - 1, 8)
        chbits = ilog(channels - 1)
        for step in mp["coupling"]:
            op.write(step["mag"], chbits)
            op.write(step["ang"], chbits)
    op.write(mp.get("reserved", 0), 2)
    if mp["submaps"] > 1:
        for m in mp["chmux"]:
            op.write(m, 4)
    for i in range(mp["submaps"]):
        op.write(0, 8)
        op.write(mp["floors"][i], 8)
        op.write(mp["residues"][i], 8)


def pack_mode(op: OggPack, md: dict) -> None:
    op.write(md["blockflag"], 1)
    op.write(md["mapping"], 8)


def pack_setup(info: dict) -> bytes:
    """Repack a parse_setup() result to Wwise setup packet bytes."""
    op = OggPack(max(256, info.get("setup_size", 256) + 16))
    channels = info.get("channels", 6)
    op.write(info["nbooks"] - 1, 8)
    for bid in info["book_ids"]:
        op.write(bid, 10)
    op.write(info["nfloors"] - 1, 6)
    for fl in info["floors"]:
        pack_floor1(op, fl)
    op.write(info["nresidues"] - 1, 6)
    for rs in info["residues"]:
        pack_residue(op, rs)
    op.write(info["nmaps"] - 1, 6)
    for mp in info["maps"]:
        pack_mapping0(op, mp, channels)
    op.write(info["nmodes"] - 1, 6)
    for md in info["modes"]:
        pack_mode(op, md)
    # trailing zero bits already in buffer; round up to byte
    return op.get_buffer()


def parse_setup(
    data: bytes,
    channels: int = 6,
) -> dict:
    br = BitReader(data)
    bits_total = len(data) * 8

    nbooks = br.read(8) + 1
    book_ids = [br.read(10) for _ in range(nbooks)]
    pos_after_books = br.tell_bits()

    nfloors = br.read(6) + 1
    pos_after_floor_count = br.tell_bits()
    floors = [parse_floor1(br) for _ in range(nfloors)]
    pos_after_floors = br.tell_bits()

    nresidues = br.read(6) + 1
    residues = [parse_residue(br) for _ in range(nresidues)]
    pos_after_residues = br.tell_bits()

    nmaps = br.read(6) + 1
    maps = [parse_mapping0(br, channels) for _ in range(nmaps)]
    pos_after_maps = br.tell_bits()

    nmodes = br.read(6) + 1
    modes = [parse_mode(br) for _ in range(nmodes)]
    pos_after_modes = br.tell_bits()

    pad_bits = br.bits_left()
    pad_value = br.read(pad_bits) if pad_bits else 0

    info: dict = {
        "setup_size": len(data),
        "bits_total": bits_total,
        "channels": channels,
        "nbooks": nbooks,
        "book_ids": book_ids,
        "unique_book_ids": sorted(set(book_ids)),
        "nfloors": nfloors,
        "floors": floors,
        "nresidues": nresidues,
        "residues": residues,
        "nmaps": nmaps,
        "maps": maps,
        "nmodes": nmodes,
        "modes": modes,
        "bit_positions": {
            "after_books": pos_after_books,
            "after_floor_count": pos_after_floor_count,
            "after_floors": pos_after_floors,
            "after_residues": pos_after_residues,
            "after_maps": pos_after_maps,
            "after_modes": pos_after_modes,
            "end": br.tell_bits(),
        },
        "trailing_pad_bits": pad_bits,
        "trailing_pad_value": pad_value,
        "parse_complete": br.tell_bits() == bits_total and pad_value == 0,
        "book_id_assignment": "t97:0-96, t219:97-315, t282:316-597",
    }
    return info
