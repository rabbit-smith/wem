"""LONG mode and seed materialization; registered values serve verification only."""

from . import long_base, short_seed


def build(d, key, geo):
    if tuple(geo) != tuple(key):
        raise ValueError("descriptor geometry does not match requested key")
    if tuple(key) not in long_base.SAMPLE_RATE:
        raise ValueError("unsupported LONG geometry key")
    rate = long_base.SAMPLE_RATE[tuple(key)]
    out = {}
    for mode in (2, 3):
        base = long_base.build_mode(d, key, geo, mode)
        out[f"variants[{mode}].curves"] = [
            word for i in range(3) for word in base[f"analysis.curves[{i}]"]
        ]
        for field in ("field_19_curve", "interval_u32"):
            out[f"variants[{mode}].{field}"] = base[f"analysis.{field}"]
    # 100161a1..10016486: same 87-segment ATH writer and f32 tail.
    out["seed.base_curve"] = [short_seed.f32_bits(v) for v in short_seed.ath(1024, rate)]
    # 1001671f..10016785: bin log-coordinate, converted toward zero.
    out["seed.group_labels_u32"] = [v & 0xFFFFFFFF for v in short_seed.octave(1024, rate)]
    return out
