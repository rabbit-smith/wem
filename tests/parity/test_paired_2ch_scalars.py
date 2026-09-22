"""Registered 2ch/48k container and seed scalars equal the paired build.

Both groups below were read from the paired build rather than derived, and both
were previously wrong because the 2ch registration carried the 6ch values:

* Aux fmt fields.  Thirteen 2ch/48k reference streams (different sources,
  lengths, and both the bank and external-source conversion routes) carry the
  identical triple ``16080 / 16560 / 0xec69cb18`` with ``0x24 = 0x32 = 0``, so
  they are per-layout constants, not per-sample data.  The 6ch profile carries
  its own triple (18180 / 18636 / 0xb3dea448), which the byte-exact 6ch reference
  already pins.

* ``seed.outer_u32`` rate-dependent scalars.  The serialised array holds two
  record copies; a live read of the running 2ch/48k build, anchored on the long
  geometry 5-tuple at ``[7..11]``, gives the values below for both copies.  The
  pointer-bearing words differ only by load-time relocation and must stay as
  registered, so only the scalars are asserted here.

With the aux fields pinned, every self-describing fmt field of the 2ch output
matches the reference; the remaining header differences are the three
content-derived ones (``dwDataPayloadSize``, ``nAvgBytesPerSec``,
``uMaxPacketSize``), which follow the coded audio.
"""

from __future__ import annotations

import unittest
from typing import Any, ClassVar

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem_reference.profiles.artifact import resolve_selection


#: The compiled 2ch/48000 profile: every value below is read off the carrier
#: its selection resolves to, never re-typed here.
PROFILE = resolve_selection(WwiseProfile(WwiseVersion.WWISE2013, 2, 48000))

AUX_FIELDS = {
    "dwUnknown_0x24": 0,
    "uUnknown_0x32": 0,
    "dwUnknown_0x34": 16080,
    "dwUnknown_0x38": 16560,
    "dwUnknown_0x3C": 0xEC69CB18,
}

#: index -> value, both record copies (stride 30) share the same lookup group.
OUTER_SCALARS = {
    16: 1067072881,
    17: 664,
    20: 640,
    23: 560,
    37: 0xFFFFFF1E,  # -226 as u32
    38: 5,
    39: 8,
    40: 776,
    41: 48000,
    46: 1067072881,
    47: 664,
    50: 640,
    53: 560,
}


class Paired2chScalarParityTests(unittest.TestCase):
    container_metadata: ClassVar[dict[str, Any]]
    outer_u32: ClassVar[list[int]]

    @classmethod
    def setUpClass(cls) -> None:
        cls.container_metadata = PROFILE.container_metadata.to_fmt_dict()
        cls.outer_u32 = list(PROFILE.table("long_base.seed_outer_u32"))

    def test_aux_container_fields_are_the_paired_layout_constants(self) -> None:
        for field, expected in AUX_FIELDS.items():
            with self.subTest(field=field):
                self.assertEqual(self.container_metadata[field], expected)

    def test_seed_outer_rate_dependent_scalars_come_from_the_read(self) -> None:
        for index, expected in OUTER_SCALARS.items():
            with self.subTest(index=index):
                self.assertEqual(self.outer_u32[index], expected)

    def test_both_record_copies_agree(self) -> None:
        """The array holds two copies at stride 30; the lookup group must match."""
        for index in (16, 17, 20, 23):
            with self.subTest(index=index):
                self.assertEqual(self.outer_u32[index], self.outer_u32[index + 30])


if __name__ == "__main__":
    unittest.main()
