#!/usr/bin/env python3
"""Build the wheel and verify packaged runtime resources outside the repo."""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ALLOWLIST = ROOT / "tests" / "contract" / "distribution_allowlist.json"


def _distribution_contract() -> dict[str, object]:
    return json.loads(ALLOWLIST.read_text(encoding="utf-8"))


def _verify_wheel_contents(wheel: Path, contract: dict[str, object]) -> None:
    expected = sorted([*contract["modules"], *contract["resources"]])
    with zipfile.ZipFile(wheel) as archive:
        actual = sorted(
            name
            for name in archive.namelist()
            if name.startswith("wwise2013_wem/")
        )
        content_markers = tuple(
            str(value).encode("ascii").lower()
            for value in contract["forbidden_content_markers"]
        )
        content_violations = [
            name
            for name in actual
            if name.endswith((".py", ".json"))
            and any(marker in archive.read(name).lower() for marker in content_markers)
        ]
    if actual != expected:
        missing = sorted(set(expected) - set(actual))
        unexpected = sorted(set(actual) - set(expected))
        raise RuntimeError(
            "wheel package contents differ from distribution allowlist; "
            f"missing={missing}, unexpected={unexpected}"
        )
    markers = tuple(
        str(value).lower() for value in contract["forbidden_path_markers"]
    )
    violations = [
        name for name in actual if any(marker in name.lower() for marker in markers)
    ]
    if violations:
        raise RuntimeError(f"forbidden wheel paths: {violations}")
    if content_violations:
        raise RuntimeError(f"forbidden wheel content: {content_violations}")


def main() -> None:
    contract = _distribution_contract()
    with tempfile.TemporaryDirectory(prefix="wwise2013-wheel-") as directory:
        work = Path(directory)
        wheels = work / "wheels"
        installed = work / "installed"
        source = work / "source"
        wheels.mkdir()
        installed.mkdir()
        shutil.copytree(
            ROOT,
            source,
            ignore=shutil.ignore_patterns(
                ".git", "build", "dist", "*.egg-info", "__pycache__", "*.pyc"
            ),
        )
        subprocess.run(
            [
                sys.executable,
                "-m",
                "pip",
                "wheel",
                str(source),
                "--no-deps",
                "--no-cache-dir",
                "--wheel-dir",
                str(wheels),
            ],
            check=True,
        )
        wheel = next(wheels.glob("wwise2013_wem-*.whl"))
        _verify_wheel_contents(wheel, contract)
        subprocess.run(
            [
                sys.executable,
                "-m",
                "pip",
                "install",
                "--quiet",
                "--no-deps",
                "--target",
                str(installed),
                str(wheel),
            ],
            check=True,
        )
        smoke = (
            "import wwise2013_wem; "
            f"assert wwise2013_wem.__all__=={contract['root_exports']!r}; "
            "from wwise2013_wem import resolve_wem_profile; "
            "from wwise2013_wem.profiles.bundle import load_profile_bundle; "
            "from wwise2013_wem.profiles.book_ids import load_book_table; "
            "from wwise2013_wem.profiles.transient import load_transient_tables; "
            "from wwise2013_wem.profiles.psychoacoustics.long_tables import load_long_psy_tables; "
            "from wwise2013_wem.profiles.psychoacoustics.short_tables import load_short_psy_profiles; "
            "p=resolve_wem_profile(6,44100); "
            "b=load_profile_bundle(); b.verify_all(); "
            "assert b.runtime_manifest.resource('vorbis.setup')==b.setup; "
            "assert set(b.runtime_manifest.resources)=={'vorbis.setup','vorbis.codebooks.t97','vorbis.codebooks.t219','transform.mdct','analysis.transient','psychoacoustics.short-profiles','psychoacoustics.short-seed','psychoacoustics.long-base','psychoacoustics.long-modes'}; "
            "assert p.name=='wwise2013-6ch-44100'; "
            "assert len(p.setup_packet())==201; "
            "assert len(load_book_table('t97',b.runtime_manifest.resource('vorbis.codebooks.t97')))==97; "
            "assert load_transient_tables(b.runtime_manifest.resource('analysis.transient')).n==128; "
            "assert load_long_psy_tables(b.runtime_manifest.resource('psychoacoustics.long-base')).n==1024; "
            "assert len(load_short_psy_profiles(b.runtime_manifest.resource('psychoacoustics.short-profiles')))==2; "
            "print(p.name, len(p.setup_packet()), len(b.runtime_manifest.resources))"
        )
        outputs = []
        for label, python_path in (("installed", installed), ("zip", wheel)):
            environment = dict(os.environ)
            environment["PYTHONPATH"] = str(python_path)
            result = subprocess.run(
                [sys.executable, "-c", smoke],
                cwd=work,
                env=environment,
                capture_output=True,
                text=True,
            )
            if result.returncode:
                raise RuntimeError(
                    f"{label} wheel smoke failed:\n{result.stderr}"
                )
            outputs.append(f"{label}={result.stdout.strip()}")
        print("wheel smoke OK", *outputs)


if __name__ == "__main__":
    main()
