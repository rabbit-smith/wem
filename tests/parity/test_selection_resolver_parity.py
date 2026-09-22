"""One selection, two readers of one carrier, one answer.

The kernel resolves a ``WwiseProfile`` against the profile tables compiled
into the native extension, and it is the only authority on what a selection
denotes. The oracle reads the same carrier *through* that authority — the
extension's own ``profile_tables()`` stream — so this suite pins that the two
agree rather than that two independent registries do:

* the resolved identity and the recorded setup digest are the kernel's own
  seq-0 packet for the same selection;
* every installed profile is distinct, so the comparison has teeth;
* an unsatisfiable selection is rejected on both sides.

The profiles are enumerated from the carrier itself; that enumeration is
deliberately here rather than in the package's public surface.
"""

from __future__ import annotations

import hashlib
import unittest

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem import _core
from wwise_wem_reference.profiles.artifact import compiled_profiles, resolve_selection

MIN_FRAMES = 4096


def _installed() -> list[tuple[str, WwiseProfile]]:
    """Every installed profile as ``(label, selection)``."""
    selections = []
    for profile in compiled_profiles():
        selections.append(
            (
                profile.label(),
                WwiseProfile(
                    WwiseVersion.from_generation(profile.key.generation),
                    profile.key.channels,
                    profile.key.sample_rate,
                ),
            )
        )
    return selections


def _kernel_setup_digest(selection: WwiseProfile) -> str:
    """The setup packet the kernel emits for this selection (stream seq 0)."""
    session = _core.StreamSession.for_selection(selection)
    chunk = bytes(MIN_FRAMES * int(selection.channels) * 2)
    packets = session.push(chunk)
    if not packets:
        raise AssertionError(f"no seq-0 packet for {selection}")
    return hashlib.sha256(bytes(packets[0].data)).hexdigest()


class SelectionResolverParityTests(unittest.TestCase):
    def test_every_installed_selection_resolves_identically_on_both_sides(self):
        installed = _installed()
        self.assertGreaterEqual(len(installed), 1, "no installed profile")
        digests = {}
        for key, selection in installed:
            with self.subTest(profile=key):
                resolved = resolve_selection(selection)
                self.assertEqual(
                    (
                        resolved.key.generation,
                        resolved.channels,
                        resolved.sample_rate,
                    ),
                    (
                        selection.version.generation,
                        selection.channels,
                        selection.sample_rate,
                    ),
                    "the carrier resolved a different identity than the selection",
                )
                kernel = _kernel_setup_digest(selection)
                self.assertEqual(
                    resolved.setup_sha256,
                    kernel,
                    "the carrier's setup digest differs from the kernel's "
                    "seq-0 packet for the same selection",
                )
                digests[key] = kernel
        self.assertEqual(
            len(set(digests.values())),
            len(digests),
            f"installed profiles share a setup digest: {digests}",
        )

    def test_unsatisfiable_selection_is_rejected_on_both_sides(self):
        # An installed channel count at an uninstalled rate, and a channel
        # count nothing installs. Neither side may fall back to a neighbour.
        installed_geometries = {
            (selection.channels, selection.sample_rate)
            for _, selection in _installed()
        }
        self.assertTrue(installed_geometries, "no installed profile")
        channels = sorted({c for c, _ in installed_geometries})[0]
        rates = {r for _, r in installed_geometries}
        unsatisfiable = WwiseProfile(
            WwiseVersion.WWISE2013, channels, (max(rates) + 1000)
        )
        self.assertNotIn(
            (unsatisfiable.channels, unsatisfiable.sample_rate),
            installed_geometries,
        )

        with self.assertRaises(ValueError):
            resolve_selection(unsatisfiable)
        with self.assertRaisesRegex(
            _core.WemEncoderError, "no installed profile|PROFILE_NOT_FOUND"
        ):
            _core.StreamSession.for_selection(unsatisfiable)

    def test_unknown_generation_is_rejected_by_the_version_table(self):
        with self.assertRaises(ValueError):
            WwiseVersion.from_generation("1999.1")
        with self.assertRaises(ValueError):
            WwiseVersion.from_code(7)


if __name__ == "__main__":
    unittest.main()
