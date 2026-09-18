"""Vendored geometry materializer (the round promotion of the corpus builder).

Source of truth: corpus/paired-build/the round (per-instruction port of
analysis_geometry_builder on the paired build, carrier the paired build md5
d21427e9c35271d579205bc6fda8e325). Inputs come from
data/materializer_inputs.json (gen_r20 extracts them); 6ch outputs are
locked against the registered profile by tests/contract/
test_geometry_materializer_contract.py. Keep numerics byte-identical to
the corpus; regenerate, do not retype.
"""

import json
from pathlib import Path

try:
    _DATA = json.loads(
        (Path(__file__).resolve().parent / "data" / "materializer_inputs.json").read_text()
    )
except (OSError, ValueError) as exc:  # package data is a build artifact: fail loud, never silent
    raise RuntimeError(
        "materializer_inputs.json missing/corrupt; regenerate via corpus gen_r20"
    ) from exc


def _s32(x: int) -> int:
    return x - 2**32 if x >= 2**31 else x


def load(decode_path=None):
    """Corpus-API shim: descriptor dict backed by the extracted bundle.

    Both descriptor copies of the paired build carry byte-identical mask
    pools, so one record serves every geometry key.
    """
    slots = {
        "mask_pool_0": {"raw_u32": _DATA["mask_pool_0_u32"]},
        "mask_pool_1": {"raw_u32": _DATA["mask_pool_1_u32"]},
    }
    rec = {"slots": slots}
    return {"descriptor_6ch": rec, "descriptor_2ch": rec}


def record_key(d, key):
    return "descriptor_6ch" if key[0] == 6 else "descriptor_2ch"


def slot_u32(d, key, slot):
    return list(d[record_key(d, key)]["slots"][slot]["raw_u32"])


def constant_bytes(name):
    raw = bytes.fromhex(_DATA[name])
    return raw


def list_profiles():
    """Vendored bundle exposes exactly the two paired geometries."""
    return [
        ("wwise2013-6ch-44100", (6, 40000, 70000), (6, 40000, 70000)),
        ("wwise2013-2ch-48000", (2, 45000, 50000), (2, 45000, 50000)),
    ]
