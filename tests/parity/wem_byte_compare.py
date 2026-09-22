"""Shared failure localization for whole-WEM byte comparisons."""

from __future__ import annotations

import hashlib
import itertools
import unittest

from wwise_wem_reference.container.wem import load_wem_parts_bytes


def assert_wem_equal(
    case: unittest.TestCase,
    expected: bytes,
    actual: bytes,
    label: str,
) -> None:
    """Assert byte identity and identify the first differing packet on failure."""
    if actual == expected:
        return
    expected_packets = load_wem_parts_bytes(expected)["packets"]
    actual_packets = load_wem_parts_bytes(actual)["packets"]
    sentinel = object()
    first_difference = next(
        (
            index
            for index, pair in enumerate(
                itertools.zip_longest(
                    expected_packets,
                    actual_packets,
                    fillvalue=sentinel,
                )
            )
            if pair[0] != pair[1]
        ),
        None,
    )
    packet_label = (
        "none"
        if first_difference is None
        else "setup"
        if first_difference == 0
        else f"audio {first_difference - 1}"
    )
    case.fail(
        f"{label}: WEM differs; first differing packet={packet_label}; "
        f"expected/actual packet counts={len(expected_packets)}/{len(actual_packets)}; "
        f"expected/actual bytes={len(expected)}/{len(actual)}; "
        f"expected/actual SHA-256={hashlib.sha256(expected).hexdigest()}/"
        f"{hashlib.sha256(actual).hexdigest()}"
    )
