#!/usr/bin/env python3
"""Pure-Python Vorbis floor1 algorithms (libvorbis floor1.c).

Packet Y values are *residuals* (wrapped predictions), not absolute post heights.
Call floor1_wrap(absolute_posts, postlist, range_) before pack_floor1_body;
decode_floor1_body returns residuals — call floor1_unwrap to recover absolutes.

References (xiph/vorbis lib/floor1.c):
  floor1_look          → neighbor tables
  render_point         → linear prediction
  floor1_inverse1      → unwrap residuals → absolute Y
  floor1_encode        → wrap absolute Y → residuals
  render_line/line0    → Bresenham floor curve
  floor1_inverse2      → piecewise linear × FLOOR1_fromdB_LOOKUP
"""
from __future__ import annotations

from typing import Sequence

# multiplier (1..4) → quant range  (libvorbis look->quant_q)
FLOOR1_RANGES: dict[int, int] = {1: 256, 2: 128, 3: 86, 4: 64}

# libvorbis FLOOR1_fromdB_LOOKUP[256]: quant 0..255 → linear amplitude.
# Index is post_Y * multiplier, clamped to [0, 255].
# Formula approx: amp = 10**((q * (140/255) - 140) / 20)  (0 → ~-140 dB, 255 → 0 dB)
FLOOR1_fromdB_LOOKUP: list[float] = [
    1.0649863e-07, 1.1341951e-07, 1.2079015e-07, 1.2863978e-07,
    1.3699951e-07, 1.4590251e-07, 1.5538408e-07, 1.6548181e-07,
    1.7623575e-07, 1.8768855e-07, 1.9988561e-07, 2.128753e-07,
    2.2670913e-07, 2.4144197e-07, 2.5713223e-07, 2.7384213e-07,
    2.9163793e-07, 3.1059021e-07, 3.3077411e-07, 3.5226968e-07,
    3.7516214e-07, 3.9954229e-07, 4.2550680e-07, 4.5315863e-07,
    4.8260743e-07, 5.1396998e-07, 5.4737065e-07, 5.8294187e-07,
    6.2082472e-07, 6.6116941e-07, 7.0413592e-07, 7.4989464e-07,
    7.9862701e-07, 8.5052630e-07, 9.0579828e-07, 9.6466216e-07,
    1.0273513e-06, 1.0941144e-06, 1.1652161e-06, 1.2409384e-06,
    1.3215816e-06, 1.4074654e-06, 1.4989305e-06, 1.5963394e-06,
    1.7000785e-06, 1.8105592e-06, 1.9282195e-06, 2.0535261e-06,
    2.1869758e-06, 2.3290978e-06, 2.4804557e-06, 2.6416497e-06,
    2.8133190e-06, 2.9961443e-06, 3.1908506e-06, 3.3982101e-06,
    3.6190449e-06, 3.8542308e-06, 4.1047004e-06, 4.3714470e-06,
    4.6555282e-06, 4.9580707e-06, 5.2802740e-06, 5.6234160e-06,
    5.9888572e-06, 6.3780469e-06, 6.7925283e-06, 7.2339451e-06,
    7.7040476e-06, 8.2047000e-06, 8.7378876e-06, 9.3057248e-06,
    9.9104632e-06, 1.0554501e-05, 1.1240392e-05, 1.1970856e-05,
    1.2748789e-05, 1.3577278e-05, 1.4459606e-05, 1.5399272e-05,
    1.6400004e-05, 1.7465768e-05, 1.8600792e-05, 1.9809576e-05,
    2.1096914e-05, 2.2467911e-05, 2.3928002e-05, 2.5482978e-05,
    2.7139006e-05, 2.8902651e-05, 3.0780908e-05, 3.2781225e-05,
    3.4911534e-05, 3.7180282e-05, 3.9596466e-05, 4.2169667e-05,
    4.4910090e-05, 4.7828601e-05, 5.0936773e-05, 5.4246931e-05,
    5.7772202e-05, 6.1526565e-05, 6.5524908e-05, 6.9783085e-05,
    7.4317983e-05, 7.9147585e-05, 8.4291040e-05, 8.9768747e-05,
    9.5602426e-05, 0.00010181521, 0.00010843174, 0.00011547824,
    0.00012298267, 0.00013097477, 0.00013948625, 0.00014855085,
    0.00015820453, 0.00016848555, 0.00017943469, 0.00019109536,
    0.00020351382, 0.00021673929, 0.00023082423, 0.00024582449,
    0.00026179955, 0.00027881276, 0.00029693158, 0.00031622787,
    0.00033677814, 0.00035866388, 0.00038197188, 0.00040679456,
    0.00043323036, 0.00046138411, 0.00049136745, 0.00052329927,
    0.00055730621, 0.00059352311, 0.00063209358, 0.00067317058,
    0.00071691700, 0.00076350630, 0.00081312324, 0.00086596457,
    0.00092223983, 0.00098217216, 0.0010459992, 0.0011139742,
    0.0011863665, 0.0012634633, 0.0013455702, 0.0014330129,
    0.0015261382, 0.0016253153, 0.0017309374, 0.0018434235,
    0.0019632195, 0.0020908006, 0.0022266726, 0.0023713743,
    0.0025254795, 0.0026895994, 0.0028643847, 0.0030505286,
    0.0032487691, 0.0034598925, 0.0036847358, 0.0039241906,
    0.0041792066, 0.0044507950, 0.0047400328, 0.0050480668,
    0.0053761186, 0.0057254891, 0.0060975636, 0.0064938176,
    0.0069158225, 0.0073652516, 0.0078438871, 0.0083536271,
    0.0088964928, 0.009474637, 0.010090352, 0.010746080,
    0.011444421, 0.012188144, 0.012980198, 0.013823725,
    0.014722068, 0.015678791, 0.016697687, 0.017782797,
    0.018938423, 0.020169149, 0.021479854, 0.022875735,
    0.024362330, 0.025945531, 0.027631618, 0.029427276,
    0.031339626, 0.033376252, 0.035545228, 0.037855157,
    0.040315199, 0.042935108, 0.045725273, 0.048696758,
    0.051861348, 0.055231591, 0.058820850, 0.062643361,
    0.066714279, 0.071049749, 0.075666962, 0.080584227,
    0.085821044, 0.091398179, 0.097337747, 0.10366330,
    0.11039993, 0.11757434, 0.12521498, 0.13335215,
    0.14201813, 0.15124727, 0.16107617, 0.17154380,
    0.18269168, 0.19456402, 0.20720788, 0.22067342,
    0.23501402, 0.25028656, 0.26655159, 0.28387361,
    0.30232132, 0.32196786, 0.34289114, 0.36517414,
    0.38890521, 0.41417847, 0.44109412, 0.46975890,
    0.50028648, 0.53279791, 0.56742212, 0.60429640,
    0.64356699, 0.68538959, 0.72993007, 0.77736504,
    0.82788260, 0.88168307, 0.9389798, 1.0,
]


def postlist_from_floor(floor: dict) -> list[int]:
    """Full post X coordinates: [0, 1<<rangebits] + x_list."""
    return [0, 1 << floor["rangebits"], *floor["x_list"]]


def floor1_neighbor_tables(postlist: Sequence[int]) -> tuple[list[int], list[int]]:
    """loneighbor / hineighbor for posts i=2..n-1 (libvorbis floor1_look).

    For each post i in packet order (2..n-1), find closest lower/higher X among
    posts already "looked at" (indices 0..i-1). Returns lists of length n-2
    indexed by (i-2).
    """
    n = len(postlist)
    if n < 2:
        return [], []
    hx_limit = postlist[1]  # 1<<rangebits
    loneighbor: list[int] = []
    hineighbor: list[int] = []
    for i in range(n - 2):
        lo = 0
        hi = 1
        lx = 0
        hx = hx_limit
        currentx = postlist[i + 2]
        for j in range(i + 2):
            x = postlist[j]
            if x > lx and x < currentx:
                lo = j
                lx = x
            if x < hx and x > currentx:
                hi = j
                hx = x
        loneighbor.append(lo)
        hineighbor.append(hi)
    return loneighbor, hineighbor


def render_point(x0: int, x1: int, y0: int, y1: int, x: int) -> int:
    """Linear interpolation prediction (libvorbis render_point; masks 0x8000)."""
    y0 &= 0x7FFF
    y1 &= 0x7FFF
    dy = y1 - y0
    adx = x1 - x0
    if adx == 0:
        return y0
    ady = abs(dy)
    err = ady * (x - x0)
    off = err // adx
    if dy < 0:
        return y0 - off
    return y0 + off


def floor1_unwrap(
    fit_value: Sequence[int], postlist: Sequence[int], range_: int
) -> list[int]:
    """Inverse1 second half: packet residuals → absolute posts.

    fit_value[i]==0 for i>=2 means “use predicted” (result flagged 0x8000).
    Endpoints fit_value[0]/[1] are absolute quant values.
    """
    posts = len(fit_value)
    if posts != len(postlist):
        raise ValueError(f"fit_value len {posts} != postlist len {len(postlist)}")
    loneighbor, hineighbor = floor1_neighbor_tables(postlist)
    out = list(fit_value)
    for i in range(2, posts):
        predicted = render_point(
            postlist[loneighbor[i - 2]],
            postlist[hineighbor[i - 2]],
            out[loneighbor[i - 2]],
            out[hineighbor[i - 2]],
            postlist[i],
        )
        hiroom = range_ - predicted
        loroom = predicted
        room = (hiroom if hiroom < loroom else loroom) << 1
        val = out[i]
        if val:
            if val >= room:
                if hiroom > loroom:
                    val = val - loroom
                else:
                    val = -1 - (val - hiroom)
            else:
                if val & 1:
                    val = -((val + 1) >> 1)
                else:
                    val >>= 1
            out[i] = (val + predicted) & 0x7FFF
            out[loneighbor[i - 2]] &= 0x7FFF
            out[hineighbor[i - 2]] &= 0x7FFF
        else:
            out[i] = predicted | 0x8000
    return out


def floor1_wrap(
    posts: Sequence[int], postlist: Sequence[int], range_: int
) -> list[int]:
    """Encode residual wrap: absolute posts → packet Y (libvorbis floor1_encode).

    posts may carry 0x8000 “unused / predicted” flags from fit; such posts and
    any post equal to its prediction become residual 0.
    """
    _, out = floor1_wrap_with_posts(posts, postlist, range_)
    return out


def floor1_wrap_with_posts(
    posts: Sequence[int], postlist: Sequence[int], range_: int
) -> tuple[list[int], list[int]]:
    """Return both floor1's normalized absolute posts and packet residuals.

    ``floor1_encode`` modifies its quantized post vector while producing the
    coded residuals: predicted posts are replaced by their prediction and
    marked unused, while neighbors needed by a coded post have their unused
    flag cleared.  The modified vector is also the input to the integer floor
    rasterizer used by the following residue quantizer. Returning
    it here prevents the packet path from incorrectly rasterizing the
    pre-wrap fit flags (which first becomes observable after several frames).
    """
    n = len(posts)
    if n != len(postlist):
        raise ValueError(f"posts len {n} != postlist len {len(postlist)}")
    loneighbor, hineighbor = floor1_neighbor_tables(postlist)
    # Work on a mutable copy; encode clears flags on used neighbors.
    post = list(posts)
    out = [0] * n
    out[0] = post[0] & 0x7FFF
    out[1] = post[1] & 0x7FFF
    for i in range(2, n):
        ln = loneighbor[i - 2]
        hn = hineighbor[i - 2]
        predicted = render_point(
            postlist[ln], postlist[hn], post[ln], post[hn], postlist[i]
        )
        pi = post[i]
        if (pi & 0x8000) or predicted == pi:
            post[i] = predicted | 0x8000
            out[i] = 0
        else:
            headroom = range_ - predicted if range_ - predicted < predicted else predicted
            val = (pi & 0x7FFF) - predicted
            if val < 0:
                if val < -headroom:
                    val = headroom - val - 1
                else:
                    val = -1 - (val * 2)
            else:
                if val >= headroom:
                    val = val + headroom
                else:
                    val <<= 1
            out[i] = val
            post[ln] &= 0x7FFF
            post[hn] &= 0x7FFF
    return post, out


def floor1_render_line(
    x0: int,
    x1: int,
    y0: int,
    y1: int,
    out: list[float],
    base: int = 0,
) -> None:
    """Bresenham line into *integer* quant domain (libvorbis render_line0).

    Writes y values as floats into out[x] for x in [x0, min(x1, len(out))).
    ``base`` is added to each written y (default 0). Does not apply fromdB.
    """
    n = len(out)
    if x0 >= x1 or x0 >= n:
        return
    dy = y1 - y0
    adx = x1 - x0
    ady = abs(dy)
    # C integer division truncates toward zero
    base_step = int(dy / adx) if adx else 0
    sy = base_step - 1 if dy < 0 else base_step + 1
    x = x0
    y = y0
    err = 0
    ady -= abs(base_step * adx)
    limit = x1 if n > x1 else n
    if x < limit:
        out[x] = float(y + base)
    while True:
        x += 1
        if x >= limit:
            break
        err = err + ady
        if err >= adx:
            err -= adx
            y += sy
        else:
            y += base_step
        out[x] = float(y + base)


def _render_line_fromdB(
    n: int, x0: int, x1: int, y0: int, y1: int, d: list[float]
) -> None:
    """libvorbis render_line: multiply d[x] by FLOOR1_fromdB_LOOKUP[y]."""
    if x0 >= x1:
        return
    dy = y1 - y0
    adx = x1 - x0
    ady = abs(dy)
    base_step = int(dy / adx) if adx else 0
    sy = base_step - 1 if dy < 0 else base_step + 1
    x = x0
    y = y0
    err = 0
    ady -= abs(base_step * adx)
    limit = x1 if n > x1 else n
    if x < limit:
        yi = 0 if y < 0 else (255 if y > 255 else y)
        d[x] *= FLOOR1_fromdB_LOOKUP[yi]
    while True:
        x += 1
        if x >= limit:
            break
        err = err + ady
        if err >= adx:
            err -= adx
            y += sy
        else:
            y += base_step
        yi = 0 if y < 0 else (255 if y > 255 else y)
        d[x] *= FLOOR1_fromdB_LOOKUP[yi]


def floor1_curve_from_posts(
    posts: Sequence[int],
    postlist: Sequence[int],
    n: int,
    multiplier: int,
) -> list[float]:
    """Piecewise-linear floor curve, n spectrum bins (libvorbis floor1_inverse2).

    Absolute posts (0x8000 = skip / predicted-only) × multiplier → quant 0..255,
    then amp = FLOOR1_fromdB_LOOKUP[q]. Output is linear amplitude (starts as 1.0
    and is multiplied along active segments; inactive tail holds final level).
    """
    nposts = len(posts)
    if nposts != len(postlist):
        raise ValueError("posts/postlist length mismatch")
    # Sort order by X (forward_index)
    order = sorted(range(nposts), key=lambda i: postlist[i])
    out = [1.0] * n
    ly = (posts[0] & 0x7FFF) * multiplier
    ly = 0 if ly < 0 else (255 if ly > 255 else ly)
    lx = 0
    hx = 0
    for j in range(1, nposts):
        current = order[j]
        hy = posts[current] & 0x7FFF
        # active if no 0x8000 flag (libvorbis: hy == fit_value[current])
        if hy == posts[current]:
            hx = postlist[current]
            hy = hy * multiplier
            hy = 0 if hy < 0 else (255 if hy > 255 else hy)
            _render_line_fromdB(n, lx, hx, ly, hy, out)
            lx = hx
            ly = hy
    for j in range(hx, n):
        out[j] *= FLOOR1_fromdB_LOOKUP[ly]
    return out
