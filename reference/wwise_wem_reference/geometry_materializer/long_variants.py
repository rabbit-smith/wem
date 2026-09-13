"""LONG mode and seed materialization; registered values are gate-only inputs."""

from . import long_base, short_seed


def build(d, key, geo):
    if tuple(key) != (6, 40000, 70000) or tuple(geo) != tuple(key):
        raise ValueError("the round supports only the registered 6ch LONG geometry")
    out = {}
    for mode in (2, 3):
        base = long_base.build_mode(d, key, geo, mode)
        out[f"variants[{mode}].curves"] = [
            word for i in range(3) for word in base[f"analysis.curves[{i}]"]
        ]
        for field in ("field_19_curve", "interval_u32"):
            out[f"variants[{mode}].{field}"] = base[f"analysis.{field}"]
    # 100161a1..10016486: same 87-segment ATH writer and f32 tail.
    out["seed.base_curve"] = [short_seed.f32_bits(v) for v in short_seed.ath(1024, 44100)]
    # 1001671f..10016785: bin log-coordinate, converted toward zero.
    out["seed.group_labels_u32"] = [v & 0xFFFFFFFF for v in short_seed.octave(1024, 44100)]
    return out
