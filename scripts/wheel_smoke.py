#!/usr/bin/env python3
"""Build the wheel and verify the distribution facade outside the repo.

The wheel ships only the native-first facade, its DTOs, and the profile
data; the reference implementation tree is a development-tree artifact and
must be absent from the package.  This script verifies, from installed and
zip-import targets:

- the wheel inventory matches the distribution allowlist exactly;
- the package root imports, exports its exact supported surface, and keeps
  the profile metadata path (bundle, setup packet) fully functional;
- without the native kernel and without the reference tree, an encode call
  fails with a clear ImportError instead of running a half-built path;
- when the native kernel is importable in the runner environment, the
  installed facade encodes the golden input byte-exactly through it.
"""
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
GOLDEN_SHA256 = (
    "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247"
)


def _distribution_contract() -> dict[str, object]:
    return json.loads(ALLOWLIST.read_text(encoding="utf-8"))


def _verify_wheel_contents(wheel: Path, contract: dict[str, object]) -> None:
    expected = sorted([*contract["modules"], *contract["resources"]])
    with zipfile.ZipFile(wheel) as archive:
        actual = sorted(
            name
            for name in archive.namelist()
            if name.startswith("wwise_wem/")
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


def _facade_metadata_smoke(contract: dict[str, object]) -> str:
    return (
        "import wwise_wem; "
        f"assert wwise_wem.__all__=={contract['root_exports']!r}; "
        "from wwise_wem import resolve_wem_profile; "
        "from wwise_wem.profiles.bundle import load_profile_bundle; "
        "p=resolve_wem_profile(6,44100); "
        "b=load_profile_bundle(); b.verify_all(); "
        "assert b.runtime_manifest.resource('vorbis.setup')==b.setup; "
        "assert set(b.runtime_manifest.resources)=={"
        "'vorbis.setup','vorbis.codebooks.t97','vorbis.codebooks.t219',"
        "'transform.mdct','analysis.transient',"
        "'psychoacoustics.short-profiles','psychoacoustics.short-seed',"
        "'psychoacoustics.long-base','psychoacoustics.long-modes',"
        "'analysis.frozen-tables'}; "
        "assert p.name=='wwise2013-6ch-44100'; "
        "assert len(p.setup_packet())==201; "
        "print(p.name, len(p.setup_packet()), len(b.runtime_manifest.resources))"
    )


def _no_kernel_no_reference_smoke() -> str:
    return """
import sys

class _Blocker:
    def find_spec(self, name, path=None, target=None):
        if name.startswith("_wwise_wem_native"):
            raise ImportError("native kernel blocked for this smoke")
        return None

sys.meta_path.insert(0, _Blocker())
import wwise_wem
try:
    import wwise_wem_reference
    raise SystemExit("reference tree leaked into the distribution")
except ImportError:
    pass
from wwise_wem import Encoder, load_wem_profile
from wwise_wem.model import PcmBuffer
profile = load_wem_profile("wwise2013-6ch-44100")
pcm = PcmBuffer(44100, tuple(tuple(0.0 for _ in range(4096)) for _ in range(6)))
try:
    Encoder(profile).encode_pcm(pcm)
    raise SystemExit("expected ImportError without kernel or reference tree")
except ImportError as error:
    message = str(error)
    assert "_wwise_wem_native" in message, message
    print("import-error-ok")
"""


def _native_facade_smoke() -> str:
    return (
        "import hashlib; "
        "import sys; "
        "import _wwise_wem_native; "
        "from wwise_wem import encode_wav; "
        "result=encode_wav(sys.argv[1]); "
        f"assert result.sha256=={GOLDEN_SHA256!r}, result.sha256; "
        "assert result.stats.engine=='native'; "
        "print('native-encode-ok')"
    )


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
        wheel = next(wheels.glob("wwise_wem-*.whl"))
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

        outputs: list[str] = []
        for label, python_path in (("installed", installed), ("zip", wheel)):
            environment = dict(os.environ)
            environment["PYTHONPATH"] = str(python_path)

            result = subprocess.run(
                [sys.executable, "-c", _facade_metadata_smoke(contract)],
                cwd=work,
                env=environment,
                capture_output=True,
                text=True,
            )
            if result.returncode:
                raise RuntimeError(
                    f"{label} facade metadata smoke failed:\n{result.stderr}"
                )
            outputs.append(f"{label}=metadata:{result.stdout.strip()}")

            result = subprocess.run(
                [sys.executable, "-c", _no_kernel_no_reference_smoke()],
                cwd=work,
                env=environment,
                capture_output=True,
                text=True,
            )
            if result.returncode or "import-error-ok" not in result.stdout:
                raise RuntimeError(
                    f"{label} no-kernel encode smoke failed "
                    f"(rc={result.returncode}):\n{result.stdout}{result.stderr}"
                )
            outputs.append(f"{label}=no-kernel:import-error-ok")

        if _native_available():
            environment = dict(os.environ)
            environment["PYTHONPATH"] = str(installed)
            result = subprocess.run(
                [
                    sys.executable,
                    "-c",
                    _native_facade_smoke(),
                    str(ROOT / "tests" / "fixtures" / "input.wav"),
                ],
                cwd=work,
                env=environment,
                capture_output=True,
                text=True,
            )
            if result.returncode or "native-encode-ok" not in result.stdout:
                raise RuntimeError(
                    "installed native facade smoke failed "
                    f"(rc={result.returncode}):\n{result.stdout}{result.stderr}"
                )
            outputs.append("installed=native:native-encode-ok")
        else:
            outputs.append("installed=native:skipped(no kernel)")

        print("wheel smoke OK", *outputs)


def _native_available() -> bool:
    result = subprocess.run(
        [sys.executable, "-c", "import _wwise_wem_native"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    return result.returncode == 0


if __name__ == "__main__":
    main()
