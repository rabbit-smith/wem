#!/usr/bin/env python3
"""Build the single wheel and verify the distribution end-to-end.

The wheel ships the native-first facade and its abi3 kernel together
(``wwise_wem._native``, built from ``crates/wem-python`` by the maturin
build backend); the reference implementation tree is a development-tree
artifact and must be absent from the package.  This script verifies:

- the wheel inventory matches the distribution allowlist exactly
  (modules + resources + native extension artifact);
- the wheel carries no reference-tree or forbidden paths/content;
- the package root imports, exports its exact supported surface, and keeps
  the profile metadata path (bundle, setup packet) fully functional, both
  from the installed wheel and a zip-import target;
- in a clean venv (no development tree on the path), the installed facade
  encodes the golden input byte-exactly through the native kernel and the
  reference package is not importable there.

Building the wheel requires a Rust toolchain (the maturin backend invokes
cargo); a missing one fails this smoke with an install hint.
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
GOLDEN_INPUT = ROOT / "tests" / "fixtures" / "input.wav"
GOLDEN_REFERENCE = ROOT / "tests" / "fixtures" / "reference.wem"


def _distribution_contract() -> dict[str, object]:
    return json.loads(ALLOWLIST.read_text(encoding="utf-8"))


def _expected_native_artifacts(contract: dict[str, object]) -> list[str]:
    """Platform artifact names for the allowlisted native extensions."""
    extension = ".pyd" if os.name == "nt" else ".so"
    return [
        f"{module.replace('.', '/')}.abi3{extension}"
        for module in contract["native_extensions"]
    ]


def _verify_wheel_contents(wheel: Path, contract: dict[str, object]) -> None:
    expected = sorted(
        [*contract["modules"], *contract["resources"], *
         _expected_native_artifacts(contract)]
    )
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
    if any(name.startswith("wwise_wem_reference") for name in actual):
        raise RuntimeError("reference tree leaked into the wheel")
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
    native = _expected_native_artifacts(contract)
    if not any(name in actual for name in native):
        raise RuntimeError(
            f"wheel does not carry the native extension {native}; "
            "the single wheel must embed its engine"
        )


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


def _clean_venv_native_smoke() -> str:
    """Installed facade must encode byte-exactly through the embedded kernel."""
    return (
        "import hashlib\n"
        "import sys\n"
        "from wwise_wem import encode_wav, _engine\n"
        "assert _engine.active_engine() == 'native', _engine.active_engine()\n"
        "result = encode_wav(sys.argv[1])\n"
        "ref = hashlib.sha256(open(sys.argv[2], 'rb').read()).hexdigest()\n"
        "assert result.sha256 == ref, (result.sha256, ref)\n"
        "try:\n"
        "    import wwise_wem_reference\n"
        "except ImportError:\n"
        "    print('native-encode-ok')\n"
        "else:\n"
        "    raise SystemExit('reference tree importable from clean install')\n"
    )


def main() -> None:
    contract = _distribution_contract()
    with tempfile.TemporaryDirectory(prefix="wwise2013-wheel-") as directory:
        work = Path(directory)
        wheels = work / "wheels"
        source = work / "source"
        wheels.mkdir()
        # A stale development-tree .so must not ride into the wheel.
        shutil.copytree(
            ROOT,
            source,
            ignore=shutil.ignore_patterns(
                ".git", ".venv", "venv", "build", "dist", "crates/target",
                "*.egg-info", "__pycache__", "*.pyc", "*.so",
                "*.dylib", "*.pyd", "*.whl",
            ),
        )
        try:
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
        except subprocess.CalledProcessError as error:
            raise RuntimeError(
                "wheel build failed; the maturin backend needs a Rust "
                "toolchain (cargo) on PATH; see AGENTS.md for the layout"
            ) from error
        wheel = next(wheels.glob("wwise_wem-*.whl"))
        _verify_wheel_contents(wheel, contract)

        # --- zip-import metadata path (pure Python subset) ---------------
        environment = dict(os.environ)
        environment["PYTHONPATH"] = str(wheel)
        result = subprocess.run(
            [sys.executable, "-c", _facade_metadata_smoke(contract)],
            cwd=work,
            env=environment,
            capture_output=True,
            text=True,
        )
        if result.returncode:
            raise RuntimeError(
                f"zip facade metadata smoke failed:\n{result.stderr}"
            )

        # --- clean-venv end-to-end path (facade + embedded kernel) -------
        venv = work / "venv"
        subprocess.run(
            [sys.executable, "-m", "venv", str(venv)],
            check=True,
        )
        venv_python = (
            venv / "Scripts" / "python.exe"
            if os.name == "nt"
            else venv / "bin" / "python"
        )
        subprocess.run(
            [
                str(venv_python),
                "-m",
                "pip",
                "install",
                "--quiet",
                "--no-deps",
                str(wheel),
            ],
            check=True,
        )
        result = subprocess.run(
            [str(venv_python), "-c", _facade_metadata_smoke(contract)],
            cwd=work,
            env=os.environ.copy(),
            capture_output=True,
            text=True,
        )
        if result.returncode:
            raise RuntimeError(
                f"installed facade metadata smoke failed:\n{result.stderr}"
            )
        result = subprocess.run(
            [
                str(venv_python),
                "-c",
                _clean_venv_native_smoke(),
                str(GOLDEN_INPUT),
                str(GOLDEN_REFERENCE),
            ],
            cwd=work,
            env=os.environ.copy(),
            capture_output=True,
            text=True,
        )
        if result.returncode or "native-encode-ok" not in result.stdout:
            raise RuntimeError(
                f"clean-venv native encode smoke failed "
                f"(rc={result.returncode}):\n{result.stdout}{result.stderr}"
            )
        print(
            "wheel smoke OK",
            f"wheel={wheel.name}",
            "zip=metadata-ok",
            "installed=metadata:ok",
            "clean-venv=native-encode-ok",
        )


if __name__ == "__main__":
    main()
