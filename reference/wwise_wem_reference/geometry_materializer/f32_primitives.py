"""Deterministic f32 / x87 primitives for the analysis-geometry builder.

The paired 2013.2 build is 32-bit x86. All surface computation goes through the
x87 FPU (double) with explicit f32 re-rounds wherever a segment ends in an x87
store or in a call to the build's round helper. To reproduce registered bytes
exactly, every such re-round must be modeled with Python struct '<f' (IEEE-754
round-to-nearest-even), matching the x86 default.

No clocks, no randomness. Pure functions.
"""

from __future__ import annotations

import math
import struct


def f32(x: float) -> float:
    """Round a value to single precision (x86 fstps semantics)."""
    return to_f32_store(x)


def f32_bits(x: float) -> int:
    """f32 as an unsigned little-endian bit pattern."""
    return to_f32_bits(x)


def f64(x: float) -> float:
    return to_float(x)


# ---- the paired-build x87 constants (name -> value)
# These are the literals the builders reference. Kept here so a ported builder
# reads the SAME constants the paired build reads (no second source).
F64_C = {
    "C_127": 127.0,
    "PI_PREC": 3.1415927410125732,  # "Vorbis pi" (the f32 pi widened to f64)
    "HALF": 0.5,
    "LOG_L": 7.177114298428933e-07,  # wwise_float_log L
    "LOG_M": 764.6162109375,  # wwise_float_log M
    "WWISE_LOG_ADD": 0.345,
    "FOUR": 4.0,
    "F15": 15.0,
    "F00625": 0.0625,
    "F8": 8.0,
    "F2": 2.0,
    "F02": 0.2,  # multiplies in the seed-curve builder
    "F07": 0.7,  # multiplies in the seed-curve builder
}

# f32 constants
F32_C = {
    "NEG130": -130.0,  # specmax peak floor
    "POS140": 140.0,  # seed curve level distance
    "DECAY045": 0.45,  # specmax decay (stored as f64 in the paired build)
    "HALF_F32": 0.5,
    "NEG1": -1.0,
}

# Static sub-band geometry LUTs (consumed as u32 and stored into the seed context)
LUT50 = [2, 4, 6, 9, 13, 17, 22, 12, 8, 3, 2, 1]  # 12 u32
LUT80 = [4, 5, 6, 8, 8, 8, 8, 4, 4, 3, 2, 4]  # 12 u32, sum=64=n/2


# NOTE (verified against the paired build's registered surfaces):
# The paired 2013.2 build's wwise_float_log is a BIT-MANIPULATION trick, not
# math.log(x). It reads the f32 bit pattern (sign cleared), treats it as a large
# integer/float, then *LOG_L - LOG_M. This yields ~6.0206*log2(x).
# math.log(x)*L - M (the earlier implementation) is WRONG here and has been
# replaced. Regression: 6ch registered 1024-bin curves sit in [-14,+7]/[-7,+69]/
# [0,+30]; the math.log form would produce ~-764, so the bit-trick is the
# historical true user.


def wwise_float_log(x: float) -> float:
    """Core wwise_float_log(x) = f32( float(absbits(f32(x))) * LOG_L - LOG_M ).

    This is the bit-manipulation log used by the seed and analysis paths. It does
    NOT include the +0.345 (WWISE_LOG_ADD); that is added at the call site AFTER
    this f32 rounding (see offset_for_n).
    """
    L = F64_C["LOG_L"]
    M = F64_C["LOG_M"]
    bits = f32_bits(f32(x)) & 0x7FFFFFFF  # clear sign bit
    return f32(to_float(bits) * L - M)


def offset_for_n(n: int) -> float:
    """Seed/analysis offset construction, byte-exact:
        offset = f32( f32(float(absbits(f32(4.0/n))) * LOG_L - LOG_M) + 0.345 )
    The 4.0 numerator is the paired build's f64 constant. Two f32 roundings: one
    in wwise_float_log, one after adding WWISE_LOG_ADD.
    """
    val = f32(F64_C["FOUR"] / to_float(n))
    ADD = F64_C["WWISE_LOG_ADD"]
    return f32(wwise_float_log(val) + ADD)


# Interval-only x87 operations: 64 significant bits, nearest/even.
# Fractions retain register precision until the caller's explicit fstp store.
def _round80(value):
    from fractions import Fraction

    value = Fraction(value)
    if not value:
        return value
    sign = -1 if value < 0 else 1
    value = abs(value)
    n, d = value.numerator, value.denominator
    exponent = n.bit_length() - d.bit_length()
    scale = Fraction(2) ** exponent
    if value < scale:
        exponent -= 1
    quantum = Fraction(2) ** (exponent - 63)
    units = value / quantum
    whole, remainder = divmod(units.numerator, units.denominator)
    if 2 * remainder > units.denominator or (2 * remainder == units.denominator and whole & 1):
        whole += 1
    return sign * whole * quantum


def mul80(left, right):
    """Finite normal-range x87 multiply; returns an exact register value."""
    from fractions import Fraction

    return _round80(Fraction(left) * Fraction(right))


def add80(left, right):
    """Finite normal-range x87 add; returns an exact register value."""
    from fractions import Fraction

    return _round80(Fraction(left) + Fraction(right))


# Table-relative origins for the three extracted read-only constant regions.
# Every coefficient lookup below is written `TABLE + byte_offset`, where the
# offset counts from the first byte of that region's blob (`_CIATAN_DATA`,
# `_CIEXP_DATA`, `_CILOG_DATA`); the names keep the two same-shaped exp/log
# regions apart at the call sites. These are table origins, not addresses.
CIATAN = 0
CIEXP = 0
CILOG = 0

# Supplied CRT _CIatan: the SSE2 value implementation the paired build links.
# Constants are extracted from the supplied runtime library and indexed by their
# offset within that region (see the table origins above).
_CIATAN_DATA = bytes.fromhex(
    "e2652f227f2b7a3c075c143326a6813cbdcbf07a8807703c075c143326a6913c"
    "4fbb610567acdd3f182d4454fb21e93f9bf681d20b73ef3f182d4454fb21f93f"
    "182d4454fb21f93f182d4454fb21f9bf000000000000f03f0000000000000040"
    "000000000000f83f000000000000f0bf11da22e33aad903feb0d76244b7ba93f"
    "513dd0a0660db13f6e204cc5cd45b73fff8300922449c23f0d5555555555d53f"
    "2f6c6a2c44b4a2bf9afdde522ddead3f6d9a74aff2b0b33f711623fec671bc3f"
    "c4eb98999999c93f"
)


def ciatan(x):
    """Value semantics of supplied CRT _CIatan; SSE2 rounds each operation.

    Models all numeric branches, including signed zero, subnormal, infinity,
    and NaN. Does not emulate FPU status flags or the wrapper's stack save.
    This is the supplied runtime's implementation, not proof of another CRT
    version.
    """
    x = to_float(x)  # wrapper fstpl.
    bits = struct.unpack("<Q", struct.pack("<d", x))[0]
    high = (bits >> 32) & 0x7FFFFFFF
    negative = bits >> 63

    def c(off):
        return struct.unpack_from("<d", _CIATAN_DATA, off)[0]

    # Large, NaN and tiny argument classes.
    if high > 0x440FFFFF:
        if (bits & 0x7FFFFFFFFFFFFFFF) > 0x7FF0000000000000:
            return x
        return c(CIATAN + 0x48 if negative else CIATAN + 0x40)
    if high <= 0x3E3FFFFF:
        return x
    region = -1
    if high > 0x3FDBFFFF:
        x = abs(x)  # fabs and fstpl.
        if high <= 0x3FF2FFFF:
            if high <= 0x3FE5FFFF:
                # Preserve each SSE2 operation, in order.
                numerator = x + x
                denominator = x + c(CIATAN + 0x58)
                numerator = numerator - c(CIATAN + 0x50)
                x = numerator / denominator
                region = 0
            else:
                x = (x - c(CIATAN + 0x50)) / (x + c(CIATAN + 0x50))
                region = 1
        elif high <= 0x40037FFF:
            denominator = x * c(CIATAN + 0x60)
            numerator = x - c(CIATAN + 0x60)
            denominator = denominator + c(CIATAN + 0x50)
            x = numerator / denominator
            region = 2
        else:
            x = c(CIATAN + 0x68) / x
            region = 3
    # Even polynomial in x^4, times x^2.
    z = x * x
    w = z * z
    even = c(CIATAN + 0x70) * w
    for off in range(0x78, 0xA0, 8):
        even = even + c(CIATAN + off)
        if off != 0x98:
            even = even * w
    even = even * z
    # Odd polynomial; subtraction signs are literal.
    odd = c(CIATAN + 0xA0) * w
    for off in range(0xA8, 0xC8, 8):
        odd = odd - c(CIATAN + off)
        if off != 0xC0:
            odd = odd * w
    odd = w * odd
    correction = (even + odd) * x
    if region == -1:
        return x - correction
    # Low part, reduced argument, high part, sign.
    correction = correction - c(CIATAN + 8 * region)
    correction = correction - x
    result = c(CIATAN + 0x20 + 8 * region) - correction
    return -result if negative else result


# Constants from the same read-only CRT region as _CIATAN_DATA.
_CILOG_DATA = bytes.fromhex(
    "000000000000f03f000000000000f0bf000000000000a0410000000000003043"
    "6c6f673130000000000000000000f0bf0000000000005043362bf111f3fe593d"
    "00609f501344d33f000000000000f03f000000000000e03f0000000000000040"
    "44523edf12f1c23fde03cb966446c73f599322942449d23f935555555555e53f"
    "9fc678d0099ac33faf788e1dc571cc3f04fa97999999d93f000020157bcbdb3f"
    "d5ad9aca3894bb3d000000000000000000000000000000000038fafe422ee63f"
    "3067c79357f32e3d010000000000e0bf5b3051555555d53f9045ebffffffcfbf"
    "1101f124b399c93f9fc806e57555c5bf000000000000e0bf775555555555d53f"
    "cbfdffffffffcfbf0cdd95999999c93fa74567555555c5bf30de44a32449c23f"
    "653d42a4ffffbfbfcad62a288471bc3fff68b043eb99b9bf85d0aff78281b73f"
    "cd45d1751352b5bf9fdee0c3f034f73f0090e6797fccd7bf1fe92c6a7813f73f"
    "00000dc2ee6fd7bfa0b5fa0860f2f63f00e05113e313d7bf7d8c131fa6d1f63f"
    "007828385bb8d6bfd1b4c50b49b1f63f00788090555dd6bfba0c2f334791f63f"
    "00001876d002d6bf234222189f71f63f00909086caa8d5bfd91ea5994f52f63f"
    "00500356434fd5bfc4248faa5633f63f00406bc337f6d4bf14dc9d6bb314f63f"
    "0050a8fda79dd4bf4c5cc65264f6f53f00a889399245d4bf4f2c91b567d8f53f"
    "00b8b039f4edd3bfde905bcbbcbaf53f00708f44ce96d3bf781ad9f2619df53f"
    "00a0bd171e40d3bf875646125680f53f008046efe2e9d2bfd36be7ce9763f53f"
    "00e030381b94d2bf937fa7e22547f53f0088da8cc53ed2bf83450642ff2af53f"
    "00902729e1e9d1bfdfbdb2db220ff53f00f8482b6d95d1bfd7de34478ff3f43f"
    "00f8b99a6741d1bf4028decf43d8f43f0098ef94d0edd0bfc8a378c03ebdf43f"
    "0010db18a59ad0bf8a25e0c37fa2f43f00b86352e647d0bf3484d4240588f43f"
    "00f0864522ebcfbf0b2d191bce6df43f00b017754a47cfbf541839d3d953f43f"
    "0030103d44a4cebf5a84b444273af43f00b0e9440d02cebffbf81541b520f43f"
    "00f07729a260cdbfb1f43eda8207f43f0090950401c0ccbf8ffe575d8feef33f"
    "001089562920ccbfe94c0ba0d9d5f33f0010818d1781cbbf2bc110c060bdf33f"
    "00d0d3ccc9e2cabfb8da752b24a5f33f0090122e4045cabf02d09fcd228df33f"
    "00f01d6877a8c9bf1c7a84c55b75f33f003048696d0cc9bfe236ad49ce5df33f"
    "00c045a62071c8bf40d44d987946f33f003014b48fd6c7bf24cbffce5c2ff33f"
    "0070623cb83cc7bf490da1757718f33f0060379b9aa3c6bf90393e37c801f33f"
    "00a0b754310bc6bf41f895bb4eebf23f003024767d73c5bfd1a919020ad5f23f"
    "0030c28f7bdcc4bf2afdb7a8f9bef23f0000d2512c46c4bfab1b0c7a1ca9f23f"
    "000083bc8ab0c3bf30b514607293f23f0000496b991bc3bff5a15757fa7df23f"
    "0040a4905487c2bfbf3b1d9bb368f23f00a079f8b9f3c1bfbdf58f839d53f23f"
    "00a02c25c860c1bf3b08c9aab73ef23f0020f7577fcec0bfb640a92b012af23f"
    "00a0fe49dc3cc0bf3241cc967915f23f00804bbcbd57bfbf9bfcd21d2001f23f"
    "004040960837bebf0b484d49f4ecf13f0040f93e9817bdbf69658f52f5d8f13f"
    "00a0d84e67f9bbbf7c7e571123c5f13f00602f2079dcbabfe926cb747cb1f13f"
    "008028e7c3c0b9bfb61a2c0c019ef13f00c072b346a6b8bfbd70b67bb08af13f"
    "0000acb3018db7bfb6bcef258a77f13f00003845f174b6bfda314c358d64f13f"
    "0080876d0e5eb5bfdd5f2790b951f13f00e0a1de5c48b4bf4cd232a40e3ff13f"
    "00a06a4dd933b3bfdaf910728b2cf13f0060c5f87920b2bf31b5ec28301af13f"
    "00206298460eb1bfaf3484dafb07f13f0000d26a6cfaafbfb36b4e0feef5f03f"
    "0040774a8ddaadbfce9f2a5d06e4f03f000085e4ecbcabbf21a52c6344d2f03f"
    "00c0124089a1a9bf1a98e27ca7c0f03f00c002335888a7bfd136c6832faff03f"
    "0080d6675e71a5bf3913a098db9df03f008065498a5ca3bfdfe752afab8cf03f"
    "00401564e349a1bffb284e2f9f7bf03f0080eb82c0729ebf198f358cb56af03f"
    "00805252f1559abf2cf9eca5ee59f03f008081cf623d96bf902cd1cd4949f03f"
    "0000aa8cfb2892bfa9adf0c6c638f03f0000f9207b318cbfa93279136528f03f"
    "0000aa5d351984bf4873ea272418f03f0000ecc2031278bf95b114060408f03f"
    "00002479090460bf1afa26f71fe0ef3f00009084f3ef6f3f74ea61c21ca1ef3f"
    "00003d3541dc873f2e9981b01063ef3f0080c2c4a3ce933fcdadee3cf625ef3f"
    "00008914c19f9b3fe7139103c8e9ee3f000011ced8b0a13fabb1cb7880aeee3f"
    "00c001d05b8aa53f9b0c9da21a74ee3f0080d840835ca93fb5990a83913aee3f"
    "008057ef6a27ad3f569a6009e001ee3f00c098e59875b03f98bb77e501caed3f"
    "00200de3f553b23f03917c0bf292ed3f0000388bdd2eb43fce5cfb66ac5ced3f"
    "00c057875906b63f9dde5eaa2c27ed3f00006a3576dab73fcd2c6b3e6ef2ec3f"
    "00601c4e43abb93f0279a7a26dbeec3f00600dbbc778bb3f6d08376d268bec3f"
    "0020e7321343bd3f04585dbd9458ec3f0060de71310abf3f8c9fbb33b526ec3f"
    "0040912b1567c03f3fe7ecee83f5eb3f00b092828547c13fc196db75fdc4eb3f"
    "0030cacd6e26c23f284a860c1e95eb3f0050c5a6d703c33f2c3eefc5e265eb3f"
    "0010333cc3dfc33f8b88c9674837eb3f00807a6b36bac43f4a301d214b09eb3f"
    "00f0d1283993c53f7eeff285e8dbea3f00f01824cd6ac63fa23d60311dafea3f"
    "009066ecf840c73fa758d33fe682ea3f00f01af5c015c83f8b7309ef4057ea3f"
    "0080f65429e9c83f274bab902a2cea3f0040f80236bbc93fd1f29313a001ea3f"
    "00002c1ced8bca3f1b3cdb249fd7e93f00d0015c515bcb3f90b1c70525aee93f"
    "00c0bccc6729cc3f2fce97f22e85e93f006048d535f6cc3f754ba4eeba5ce93f"
    "00c04634bdc1cd3f3848e79dc634e93f00e0cfb8018cce3fe652672f4f0de93f"
    "009017c00955cf3f9dd7ff8e52e6e83f00b81f126c0ed03f7c00cc9fcebfe83f"
    "00d0930eb871d03f0ec3bedac099e83f0070869e6bd4d03ffb1723aa2774e83f"
    "00d04b338736d13f089ab3ac004fe83f004823670d98d13f553e65e8492ae83f"
    "0080cce0fff8d13f6002f4950106e83f006863d75f59d23f29a3e06325e2e73f"
    "00a8140930b9d23fadb5dc77b3bee73f006043107218d33fc2259767aa9be73f"
    "0018ec6d2677d33f570617f20779e73f0030affb4fd5d33f0c13d6dbca56e73f"
    "00e02fe3ee32d43f6bb64f010010e63f3c5b42916c027e3c95b44d030030e63f"
    "415d0048eabf8d3c78d4940d0050e63fb7a5d686a77f8e3cad6f4e070070e63f"
    "4c25546beafc613cae0fdffeff8fe63ffd0e594c277e7cbcbcc5630700b0e63f"
    "01dadc4868c18abcf6c15c1e00d0e63f1193499d1c3f833c3ef605ebffefe63f"
    "532de21a04807ebc8097860e0010e73f5279097166ff7b3c12e967fcff2fe73f"
    "2487bd26e2008c3c6a1181dfff4fe73fd201f16e91026ebc909c670f0070e73f"
    "749c54cd71fc67bc35c87efaff8fe73f8304f59ec1be813ce6c220feffafe73f"
    "6564cc29177e70bc00c93fedffcfe73f1c8b7b08728080bc761a26e9ffefe73f"
    "aef99d6d28c08d3ce8a39c040010e83f334ce551d27f893c8f2c93170030e83f"
    "81f330b6e9fe8abc9c7333060050e83fbc35656bbfbf893cc68942200070e83f"
    "757b11f365bf8bbc0479f5ebff8fe83f57cb3da26e0089bcdf04bc2200b0e83f"
    "0a4be038df007dbc8a1b0ce5ffcfe83f059fff46710088bc438e91fcffefe83f"
    "38707ad07b81833cc75ffa1e0010e93f03b4df76913e893cb97b46130030e93f"
    "7602984b4e807f3c6f07eee6ff4fe93f2e62ffd9f07e8fbcd1123cdeff6fe93f"
    "ba382696aa8270bc0d8a45f4ff8fe93fefa864911b8087bc3e2e98ddffafe93f"
    "37935a8ae04087bc66fb49edffcfe93f00e09bc108ce3f3c519cf12000f0e93f"
    "0a5b8827aa3f8abc06b045110010ea3f56da589948ff743cfaf6bb070030ea3f"
    "186d2b8aabbe8c3c791d97100050ea3f307978ddcafe883c482ef51d0070ea3f"
    "dbabd83d76418fbc5233591c0090ea3f1276c28402bf8ebc4b3e4f2a00b0ea3f"
    "5f3fff3c04fd69bcd11eaed7ffcfea3fb4709012e73e82bc780451eeffefea3f"
    "a3de0ee03e066a3c5b0d65dbff0feb3fb90a1f38c8065a3c57caaafeff2feb3f"
    "1d3c23741e0179bcdcba95d9ff4feb3f9f2a866810ff79bc9c659e240070eb3f"
    "3e4f86d045ff8a3c401687f9ff8feb3ff9c3c29677fe7c3c4fcb04d2ffafeb3f"
    "c42bf2ee27ff63bc455c41d2ffcfeb3f21ea3beeb7ff6cbcdf0963f8ffefeb3f"
    "5c0b2e97034181bc5376b5e1ff0fec3f196ab79464c18b3ce357faf1ff2fec3f"
    "edc6308deffe64bc24e4bfdcff4fec3f7547ecbc683f84bcf7b954edff6fec3f"
    "ece053f0a37e843cd58f99ebff8fec3ff192f98d0683733c9a21252100b0ec3f"
    "040e18648efd68bc9c4694ddffcfec3f72eac71cbe7e8e3c76c4fdeaffefec3f"
    "fe889fad39be8e3c2bf89a160010ed3f715ab9a8917d753c1df70f0d0030ed3f"
    "dac7706990c1893cc40f79eaff4fed3f0cfe58c5370e58bce587dc2e0070ed3f"
    "440fc14dd6807fbcaa82dc210090ed3f5c5cfd948f7c74bc83026bd8ffafed3f"
    "7e6121c51d7f8c3c39476c2900d0ed3f53b1ffb29e01883cf59044e5ffefed3f"
    "89cc52c6d2006e3c94f6abcdff0fee3fd2692d2040837fbcddc852dbff2fee3f"
    "64081bcac1007b3cef1642f2ff4fee3f51ab94b0a8ff723c115e8ae8ff6fee3f"
    "59beefb173f657bc0dff9e110090ee3f01c80b5e8d8084bc4417a5dfffafee3f"
    "b52043d50600783ca17f121a00d0ee3f925c5660f80250bcc4bcba0700f0ee3f"
    "11e6355d444085bc028d7af5ff0fef3f0591ef3931fb4fbcc78ae51e0030ef3f"
    "551173f2ac818a3c943482f5ff4fef3f43c7d7d4413f8a3c6b4ca9fcff6fef3f"
    "7578981cf40262bc41c4f9e1ff8fef3f4be777f4d17d773c7ee3e0d2ffafef3f"
    "31a37c9a19016fbc9ee4771c00d0ef3fb1acce4bee81713c31c3e0f7ffefef3f"
    "5a87700137056ebc6e6065f4ff0ff03fda0a1c49ad7e8abc587a86f3ff2ff03f"
    "e0b2fcc3697f97bc170dfcfdff4ff03f5b94cb34febf973c824dcd030070f03f"
    "cb56e4c08300823ce8cbf2f9ff8ff03f1a7537bedfff6dbc65da0c0100b0f03f"
    "eb26e6ae7f3f91bc38d3a40100d0f03ff79f4879fa7d803cfdfddafaffeff03f"
    "c06bd670050477bc96fdba0b0010f13f620b6d84d4808e3c5df4e5faff2ff13f"
    "ef36fd64fabf9d3cd99ad50d0050f13fae50127077009a3c9a55210f0070f13f"
    "eedee3e2f9fd8d3c265427fcff8ff13f73723bdc3000913c593c3d1200b0f13f"
    "88010380797f993cb79e29f8ffcff13f678c9fab32f965bc00d48af4ffeff13f"
    "eb5ba79dbf7f933ca4868b0c0010f23f225bfd916b809f3c034385030030f23f"
    "33bf9febc2ff933c84f6bcffff4ff23f722e2e7ee701763cd92129f5ff6ff23f"
    "610c7f76bbfc7f3c3c3a93140090f23f2b41023cca0272bc1363551400b0f23f"
    "021ff233828092bc3b52feebffcff23ff2dc4f387eff88bc96adb80b00f0f23f"
    "c541305051ff85bcafe27afbff0ff33f9d285e88710081bc7f5facfeff2ff33f"
    "15b7b73f5dff91bc5667a60c0050f33fbd828b22827f953c21f7fb110070f33f"
    "ccd50dc4ba00803cb92f59f9ff8ff33f51a7b22d9d3f94bc42d2dd0400b0f33f"
    "e13876706b7f853c57c9b2f5ffcff33f3112bf103a027a3c18b4b0eaffeff33f"
    "b052b1666d7f983cf4af32150010f43f2485195f37f8673c298b47170030f43f"
    "4351dc72e601833c63b495e7ff4ff43f5a89b2b869ff893ce07504e8ff6ff43f"
    "54f2c29bb1c095bce7c16fefff8ff43f722a3af209409b3c04a7bee5ffaff43f"
    "457d0dbfb7ff94bcde27101700d0f43f3d6adc7164c099bce23ef00f00f0f43f"
    "1c53850b897f973cd14bdc120010f53f36a466716504603c7a2705160030f53f"
    "093223cecebf96bc4c70dbecff4ff53fd7a10505720289bca9545fefff6ff53f"
    "1264c90ee6bf9b3c1210e6170090f53f90efaf81c57e883c923ec90300b0f53f"
    "c00cbf0a08419fbcbc19491d00d0f53f294725fb2a8198bc897ab8e7ffeff53f"
    "0469ed80b77e94bc"
)
_CIEXP_DATA = bytes.fromhex(
    "000000000000f03f0000000000001000ffffffffffffef7f000000000000007f"
    "00000000000000000000000000000000fe822b65471567400000000000003843"
    "0000fafe422e76bf3a3b9ebc9af70cbdbdfdffffffffdf3f3c5455555555c53f"
    "912b17cf5555a53f17d0a4671111813f000000000000c842ef39fafe422ee63f"
    "24c482ffbdbfce3fb5f40cd7086bac3fcc5046d2abb2833f843a4e9be0d7553f"
    "0000000000000000000000000000f03f6ebf881a4f3b9b3c3533fba93df6ef3f"
    "5ddcd89c136071bc6180773e9aecef3fd16687107a5e90bc857f6ee815e3ef3f"
    "13f6673552d28c3c748515d3b0d9ef3ffa8ef92380ce8bbcdef6dd296bd0ef3f"
    "61c8e6614ef7603cc89b751845c7ef3f99d3335be4a3903c83f3c6ca3ebeef3f"
    "6d7b835da69a973c0f89f96c58b5ef3ffceffd921ab58e3cf747722b92acef3f"
    "d19c2f703dbe3e3ca2d1d332eca3ef3f0b6e908934036abc1bd3feaf669bef3f"
    "0ebd2f2a525695bc515b12d00193ef3f55ea4e8cef8050bccc316cc0bd8aef3f"
    "16f4d5b923c991bce02da9ae9a82ef3faf555ce9e3d3803c518ea5c8987aef3f"
    "4893a5ea151b80bc7b517d3cb872ef3f3d32de55f01f8fbcea8d8c38f96aef3f"
    "bf53133f8c898b3c75cb6feb5b63ef3f26eb11769cd996bcd45c0484e05bef3f"
    "602f3a3ef7ec9a3caab968318754ef3f9d3886cb82e78fbc1dd9fc22504def3f"
    "8dc3a644416f8a3cd68c62883b46ef3f7d04e4b0057a803c96dc7d91493fef3f"
    "94a8a8e3fd8e963c3862756e7a38ef3f7d4874f2185e873c3fa6b24fce31ef3f"
    "f2e71f982b47803cdd7ce265452bef3f5e08713f7bb896bc8163f5e1df24ef3f"
    "31ab096de1f7823ce1de1ff59d1eef3ffabf6f1a9b213dbc90d9dad07f18ef3f"
    "b40a0c7282378b3c0b03e4a68512ef3f8fcbce8992146e3c562f3ea9af0cef3f"
    "b6abb04d754d833c15b7310afe06ef3f4c74ace20142863c31d84cfc7001ef3f"
    "4af8d35d39dd8f3cff1664b208fcee3f045b8e3b80a386bcf19f925fc5f6ee3f"
    "68504bcced4a92bccba93a37a7f1ee3f8e2d511bf80799bc66d8056daeecee3f"
    "d236943ee8d171bcf79fe534dbe7ee3f151bceb3191999bce5a813c32de3ee3f"
    "6d4c2aa7489f853c2234124ca6deee3f8a69287a601293bc1c80ac0445daee3f"
    "5b8917488fa758bc2a2ef7210ad6ee3f1b9a49679b2c7cbc97a850d9f5d1ee3f"
    "11acc260ed63433c2d89616008ceee3fef64063b0966963c57001ded41caee3f"
    "7903a1dae1cc6e3cd03cc1b5a2c6ee3f30120f3f8eff933cded3d7f02ac3ee3f"
    "b0af7abbce90763c272a36d5dabfee3f77e054ebbd1d933c0dddfd99b2bcee3f"
    "8ea3710034948fbca72c9d76b2b9ee3f49a393dcccde87bc4266cfa2dab6ee3f"
    "5f380fbdc6de78bc824f9d562bb4ee3ff65c7bec461286bc0f925dcaa4b1ee3f"
    "8ed7fd180535933cda27b53647afee3f059b8a2fb7987b3cfdc797d412adee3f"
    "09541ce2e163903c295448dd07abee3feac6195085c7343cb746598a26a9ee3f"
    "35c0642be632943c4821ad156fa7ee3f9f7699614ae48cbc09dc76b9e1a5ee3f"
    "a84def3bc5338cbc85553ab07ea4ee3faee92b89785384bc20c3cc3446a3ee3f"
    "58585678ddce93bc2522558238a2ee3f64197e80aa10573c73a94cd455a1ee3f"
    "28225ebfefb393bccd3b7f669ea0ee3f82b93487ad126abcbfda0b7512a0ee3f"
    "eea96db8ef6763bc2f1a653cb29fee3f5188e0543ddc80bc849451f97d9fee3f"
    "cf3e5a7e641f78bc745fece8759fee3fb07d8bc04aee86bc7481a5489a9fee3f"
    "8ae6551e321986bcc9674256eb9fee3fd3d4095ecb9c903c3f5dde4f69a0ee3f"
    "1da54db9dc327bbc8701eb7314a1ee3f6bc06754fdec943c32c13001eda1ee3f"
    "556cd6abe1eb653c624ecf36f3a2ee3f42cfb32fc5a188bc121a3e5427a4ee3f"
    "34373bf1b66993bc13ce4c9989a5ee3f1eff193a845e80bcadc723461aa7ee3f"
    "6e5772d850d494bced92449bd9a8ee3f008a0e5b67ad903c99668ad9c7aaee3f"
    "b4eaf0c12fb78d3cdba02a42e5acee3fffe7c59c60b665bc8c44b51632afee3f"
    "445ff35983f67b3c36771599aeb1ee3f833d1ea71f0993bcc6ff910b5bb4ee3f"
    "291e6c8bb8a95dbce5c5cdb037b7ee3f59b9907cf9236cbc0f52c8cb44baee3f"
    "aaf9f422434392bc504ede9f82bdee3f4b8e66d76cca85bcba07ca70f1c0ee3f"
    "27ce912bfcaf713c90f0a38291c4ee3fbb730ae135d26d3c2323e31963c8ee3f"
    "6322622204c587bc65e55d7b66ccee3fd531e2e3861c8b3c332d4aec9bd0ee3f"
    "15bbbcd3d1bb91bc5d253eb203d5ee3fd231ee9c31cc903c58b330139ed9ee3f"
    "b35a736e8469843cbffd79556bdeee3fb49d8e97cddf82bc7af3d3bf6be3ee3f"
    "8733cb92771a8c3cadd35a999fe8ee3ffad9d14a8f7b90bc66b68d2907eeee3f"
    "baaedc56d9c355bcfb154fb8a2f3ee3f40f6a63d0ea490bc3a59e58d72f9ee3f"
    "3493ad38f4d668bc475efbf276ffee3f358a586be2ee91bc4a06a130b005ef3f"
    "cddd5f0ad7ff743cd2c14b901e0cef3fac9892fafbbd91bc091ed75bc212ef3f"
    "b30caf30ae6e733c9c5285dd9b19ef3f94fd9f5c32e38e3c7ad0ff5fab20ef3f"
    "ac5909d18fe0843c4bd1572ef127ef3f671a4e38afcd633cb5e706946d2fef3f"
    "6819926c2c6b673c6990efdc2037ef3fd2b5cc83188a80bcfac35d550b3fef3f"
    "6ffaff3f5dad8fbc7c89074a2d47ef3f49a97538ae0d90bcf2890d08874fef3f"
    "a7073da685a3743c87a4fbdc1858ef3f0f2240209e9182bc9883c916e360ef3f"
    "ac92c1d5505a8e3c8532db03e669ef3f4b6b01ac593a843c60b401f32173ef3f"
    "1f3eb40721d582bc5f9b7b33977cef3fc90d473bb92a89bc29a1f5144686ef3f"
    "d3883a6004b6743cf63f8be72e90ef3f71729d51ecc5833c834cc7fb519aef3f"
    "f091d38f12f78fbcda90a4a2afa4ef3f7d7423e298ae8dbcf1678e2d48afef3f"
    "0820aa41bcc38e3c275a61ee1bbaef3f32eba9c3942b843c97ba6b372bc5ef3f"
    "ee85d131a9648a3c40456e5b76d0ef3fede33be4ba378ebc14be9cadfddbef3f"
    "9dcd914d3b89773cd8909e81c1e7ef3f89cc6041c105533cf1718f2bc2f3ef3f"
)


def _bits64(x):
    return struct.unpack("<Q", struct.pack("<d", x))[0]


def _from_bits64(bits):
    return struct.unpack("<d", struct.pack("<Q", bits & 0xFFFFFFFFFFFFFFFF))[0]


def ciexp(x):
    """Supplied _CIexp, default CRT value semantics.

    Both wrapper fstpl stores round to binary64. The callee uses SSE2;
    its x87 loads/stores and fisttp carry already-integral binary64 values.
    No host transcendental, FPU flags, errno or matherr callback execution.
    """
    x = to_float(x)  # wrapper fstpl; the return store is the same shape.
    bits = _bits64(x)
    exponent = (bits >> 52) & 0x7FF

    def c(off):
        return struct.unpack_from("<d", _CIEXP_DATA, off)[0]

    if (bits & 0x7FFFFFFFFFFFFFFF) > 0x7FF0000000000000:
        # Default callback class: returns the argument, the load quiets it.
        return _from_bits64(bits | 0x8000000000000)
    if exponent < 0x3C9:  # 1+x, including both zeros and subnormals
        return x + c(CIEXP + 0x00)
    if exponent > 0x408:
        if bits == 0xFFF0000000000000:  # -inf: fldz
            return 0.0
        if exponent == 0x7FF:  # +inf takes the same 1+x path
            return x + c(CIEXP + 0x00)
        # Default error result from the overflow/underflow multiply.
        huge_or_tiny = c(CIEXP + 0x08 if bits >> 63 else CIEXP + 0x10)
        return huge_or_tiny * huge_or_tiny
    # Bitwise round to nearest, ties AWAY from zero.
    scaled = x * c(CIEXP + 0x30)
    scaled_bits = _bits64(scaled)
    e = ((scaled_bits >> 52) & 0x7FF) - 0x3FF
    if e < -1:
        rounded = scaled * 0.0
    elif e == -1:
        rounded = -1.0 if scaled_bits >> 63 else 1.0
    elif e <= 51:
        mask = (1 << (52 - e)) - 1
        rounded = _from_bits64((scaled_bits + (1 << (51 - e))) & ~mask)
    else:
        rounded = scaled
    # The fstl result and the fisttp consumer see the same integral value.
    k = to_int(rounded)
    v0 = c(CIEXP + 0x40) * rounded
    v1 = rounded * c(CIEXP + 0x48)
    v0 = v0 + x
    v0 = v0 + v1
    j = 2 * (k & 127)
    v1 = c(CIEXP + 0x58) * v0
    v1 = v1 + c(CIEXP + 0x50)
    v2 = v0 * v0
    table_bits = _bits64(c(CIEXP + 0x30 + (j + 15) * 8))
    scale_bits = (table_bits + ((k & 0xFFFFFFFF) << 45)) & 0xFFFFFFFFFFFFFFFF
    v3 = c(CIEXP + 0x30 + (j + 14) * 8)
    v1 = v1 * v2
    v3 = v3 + v0
    v2 = v2 * v2
    v0 = v0 * c(CIEXP + 0x68)
    v0 = v0 + c(CIEXP + 0x60)
    v1 = v1 + v3
    v0 = v0 * v2
    v1 = v1 + v0
    if exponent <= 0x407:  # normal range: scale by the table word
        scale = _from_bits64(scale_bits)
        v1 = v1 * scale
        return v1 + scale
    if k >= 0:  # split overflow scaling
        scale = _from_bits64(scale_bits - (0x3F100000 << 32))
        v1 = v1 * scale
        v1 = v1 + scale
        return v1 * c(CIEXP + 0x18)
    # Split underflow scaling; the compensation runs before the store.
    scale = _from_bits64(scale_bits + (0x3FE00000 << 32))
    one = c(CIEXP + 0x00)
    v1 = v1 * scale
    v0 = scale + v1
    if v0 < one:
        v4 = v0 + one
        v2 = scale - v0
        v1 = v1 + v2
        v2 = one - v4
        v0 = v0 + v2
        v0 = v0 + v1
        v0 = v0 + v4  # binary64 spill/reload
        v0 = v0 - one
    return v0 * c(CIEXP + 0x08)


def cilog(x):
    """Supplied _CIlog value path, default rounding and no matherr override.

    The wrapper fstpl stores bound SSE2 binary64 operations. There are no
    extended-precision arithmetic instructions inside this callee. Status
    flags, errno and user-installed CRT callbacks are not emulated.
    """
    x = to_float(x)
    bits = _bits64(x)
    high = bits >> 32

    def c(off):
        return struct.unpack_from("<d", _CILOG_DATA, off)[0]

    if ((high - 0x3FEE0000) & 0xFFFFFFFF) <= 0x308FF:
        if bits == 0x3FF0000000000000:  # fldz
            return 0.0
        v0 = x
        v0 = v0 - c(CILOG + 0x00)
        v4 = c(CILOG + 0x138)
        v1 = c(CILOG + 0x130)
        v3 = v0
        v2 = v0
        v5 = c(CILOG + 0x120)
        v6 = c(CILOG + 0xF0)
        v3 = v3 * v0
        v1 = v1 * v0
        v1 = v1 + c(CILOG + 0x128)
        v4 = v4 * v3
        v2 = v2 * v3
        v5 = v5 * v3
        v3 = v3 * c(CILOG + 0x108)
        v1 = v1 + v4
        v4 = c(CILOG + 0x140)
        v4 = v4 * v2
        v1 = v1 + v4
        v4 = c(CILOG + 0x118)
        v4 = v4 * v0
        v4 = v4 + c(CILOG + 0x110)
        v1 = v1 * v2
        v4 = v4 + v5
        v5 = v0
        v1 = v1 + v4
        v4 = c(CILOG + 0x100)
        v4 = v4 * v0
        v4 = v4 + c(CILOG + 0xF8)
        v1 = v1 * v2
        v3 = v3 + v4
        v4 = v0
        v1 = v1 + v3
        v3 = v0
        v1 = v1 * v2
        v2 = c(CILOG + 0x10)
        v2 = v2 * v0
        v5 = v5 + v2
        v5 = v5 - v2
        v2 = v5
        v2 = v2 * v5
        v2 = v2 * v6
        v3 = v3 + v2
        v4 = v4 - v3
        v4 = v4 + v2
        v2 = v0
        v2 = v2 - v5
        v0 = v0 + v5
        v2 = v2 * v6
        v0 = v0 * v2
        v0 = v0 + v4
        v0 = v0 + v1
        v3 = v3 + v0
        return v3
    # Classify before normalizing subnormals.
    absolute = bits & 0x7FFFFFFFFFFFFFFF
    if absolute == 0:
        return -math.inf  # +/-0: signed reciprocal, default error return
    if absolute > 0x7FF0000000000000:
        return _from_bits64(bits | 0x8000000000000)  # the load/store quiets sNaN
    if bits == 0x7FF0000000000000:
        return x
    if bits >> 63:
        return _from_bits64(0xFFF8000000000000)  # SSE invalid indefinite
    if high < 0x100000:
        bits = (_bits64(x * c(CILOG + 0x18)) + (0xFCC00000 << 32)) & 0xFFFFFFFFFFFFFFFF
        high = bits >> 32
    # Integer exponent/index and split table reduction.
    delta = (high - 0x3FE60000) & 0xFFFFFFFF
    index = (delta >> 13) & 0x7F
    signed = delta if delta < 0x80000000 else delta - 0x100000000
    v2 = to_float(signed >> 20)
    v1 = _from_bits64(bits - ((delta & 0xFFF00000) << 32))
    v0 = c(CILOG + 0xE8)
    v3 = c(CILOG + 0xB8)
    v6 = c(CILOG + 0xD8)
    at = CILOG + 0xB8 + (index + 0x89) * 16
    v1 = v1 - c(at)
    v1 = v1 - c(at + 8)
    at = CILOG + 0xB8 + (index + 9) * 16
    v3 = v3 * v2
    v1 = v1 * c(at)
    v3 = v3 + c(at + 8)
    v2 = v2 * c(CILOG + 0xC0)
    v0 = v0 * v1
    v5 = v1
    v4 = v1
    v5 = v5 * v1
    v4 = v4 + v3
    v0 = v0 + c(CILOG + 0xE0)
    v6 = v6 * v1
    v6 = v6 + c(CILOG + 0xD0)
    v3 = v3 - v4
    v0 = v0 * v5
    v3 = v3 + v1
    v2 = v2 + v3
    v0 = v0 + v6
    v6 = v1
    v6 = v6 * v5
    v5 = v5 * c(CILOG + 0xC8)
    v0 = v0 * v6
    v2 = v2 + v5
    v0 = v0 + v2
    v0 = v0 + v4
    return v0


# --- public safe conversions (fail-loud: domain errors raise ValueError with
# the offending input; numerics are the plain struct/float/int operations) ---


def to_float(x):
    try:
        return float(x)
    except (TypeError, ValueError, OverflowError) as exc:
        raise ValueError(f"float conversion failed for {x!r}") from exc


def to_int(x):
    try:
        return int(x)
    except (TypeError, ValueError, OverflowError) as exc:
        raise ValueError(f"int conversion failed for {x!r}") from exc


def to_f32_store(x):
    try:
        return struct.unpack("<f", struct.pack("<f", float(x)))[0]
    except (TypeError, ValueError, OverflowError) as exc:
        raise ValueError(f"f32 store failed for {x!r}") from exc


def to_f32_bits(x):
    try:
        return struct.unpack("<I", struct.pack("<f", float(x)))[0]
    except (TypeError, ValueError, OverflowError) as exc:
        raise ValueError(f"f32 bit extraction failed for {x!r}") from exc
