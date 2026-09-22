#!/usr/bin/env python3
"""Build the single wheel and verify the distribution end-to-end.

The wheel ships the native facade and its abi3 kernel together
(``wwise_wem._core``, built from ``crates/wem-python`` by the maturin
build backend); the reference implementation tree is a development-tree
artifact and must be absent from the package.  This script verifies:

- the wheel inventory matches the distribution allowlist exactly
  (modules + resources + native extension artifact);
- the wheel carries no reference-tree or forbidden paths/content;
- the package root imports, exports its exact supported surface, and keeps
  the profile metadata path (bundle, setup packet) fully functional, both
  from the installed wheel and a zip-import target;
- in a clean venv (no development tree on the path), the installed facade
  encodes the reference input byte-exactly through the embedded kernel and
  the reference package is not importable there.

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
ALLOWLIST = ROOT / "tests" / "parity" / "distribution_allowlist.json"
SMOKE_INPUT = ROOT / "tests" / "fixtures" / "input.wav"
SMOKE_REFERENCE = ROOT / "tests" / "fixtures" / "reference.wem"


def _distribution_allowlist() -> dict[str, object]:
    return json.loads(ALLOWLIST.read_text(encoding="utf-8"))


def _expected_extension_artifacts(allowlist: dict[str, object]) -> list[str]:
    """Platform artifact names for the allowlisted native extensions."""
    extension = ".pyd" if os.name == "nt" else ".so"
    return [
        f"{module.replace('.', '/')}.abi3{extension}"
        for module in allowlist["native_extensions"]
    ]


_WHEEL_EXTENSION_SUFFIXES = (".so", ".pyd", ".dylib")


def _canonical_wheel_entry(name: str) -> str:
    """Map a native extension file to its logical allowlist identity.

    maturin names abi3 extensions ``_core.abi3.so`` on POSIX but ``_core.pyd``
    on Windows (the abi3 tag lives in the wheel filename, not the artifact).
    Inventory comparison must not depend on that platform spelling.
    """
    for suffix in _WHEEL_EXTENSION_SUFFIXES:
        if name.endswith(suffix):
            stem = name[: -len(suffix)]
            if stem.endswith(".abi3"):
                stem = stem[: -len(".abi3")]
            return stem + ".ext"
    return name


def _verify_wheel_contents(wheel: Path, allowlist: dict[str, object]) -> None:
    expected = sorted(
        _canonical_wheel_entry(name)
        for name in [
            *allowlist["modules"],
            *allowlist["resources"],
            *_expected_extension_artifacts(allowlist),
        ]
    )
    with zipfile.ZipFile(wheel) as archive:
        actual = sorted(
            _canonical_wheel_entry(name)
            for name in archive.namelist()
            if name.startswith("wwise_wem/")
        )
        content_markers = tuple(
            str(value).encode("ascii").lower()
            for value in allowlist["forbidden_content_markers"]
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
        str(value).lower() for value in allowlist["forbidden_path_markers"]
    )
    violations = [
        name for name in actual if any(marker in name.lower() for marker in markers)
    ]
    if violations:
        raise RuntimeError(f"forbidden wheel paths: {violations}")
    if content_violations:
        raise RuntimeError(f"forbidden wheel content: {content_violations}")
    extension_artifacts = {
        _canonical_wheel_entry(name) for name in _expected_extension_artifacts(allowlist)
    }
    if not extension_artifacts & set(actual):
        raise RuntimeError(
            f"wheel does not carry the native extension {sorted(extension_artifacts)}; "
            "the single wheel must embed its engine"
        )


def _facade_metadata_smoke(allowlist: dict[str, object]) -> str:
    """Metadata-path smoke shared by the zip and installed import paths.

    The wheel is a facade plus its native engine: profile data is compiled
    into the engine, so the pure-Python subset must carry the public surface
    and *no* profile resource tree — no index, no manifest, no payload, and
    no packaged ``data/`` directory. The identity type stays importable
    without the extension (it is a facade DTO the reference tree reads by
    absolute name). The native selector types are deliberately out of reach
    here: an abi3 extension cannot be imported from inside a zip, so this
    smoke never touches ``wwise_wem._core``.
    """
    zeros = "0" * 64
    return (
        "import wwise_wem; "
        f"assert wwise_wem.__all__=={allowlist['root_exports']!r}; "
        "from wwise_wem.profiles.key import ProfileKey, profile_label; "
        f"k=ProfileKey(6,44100,'2013.2','5.1','sha256:{zeros}'); "
        "assert k.label()=='6ch/44100Hz/2013.2', k.label(); "
        "assert profile_label(2,48000,'2013.2')=='2ch/48000Hz/2013.2'; "
        "import importlib.util as u; "
        "assert u.find_spec('wwise_wem.data') is None, 'packaged data tree'; "
        "assert u.find_spec('wwise_wem.profiles.bundle') is None; "
        "assert u.find_spec('wwise_wem.profiles.registry') is None; "
        "assert u.find_spec('wwise_wem.profiles.resources') is None; "
        "assert u.find_spec('wwise_wem.profiles.model') is None; "
        "print('facade-metadata-ok', k.label())"
    )


def _clean_venv_core_smoke() -> str:
    """Installed facade must encode byte-exactly through the embedded kernel."""
    return (
        "import hashlib\n"
        "import sys\n"
        "from wwise_wem import RawPcm, WwiseWemError, encode\n"
        "import wwise_wem._core as core\n"
        "assert hasattr(core, 'Encoder'), core\n"
        "result = encode(sys.argv[1])\n"
        "ref = hashlib.sha256(open(sys.argv[2], 'rb').read()).hexdigest()\n"
        "assert result.sha256 == ref, (result.sha256, ref)\n"
        # The shipped entry point carries the kernel's stable error code:
        # 2ch/44100 is not an installed configuration, so the kernel rejects
        # the selection and the facade must hand the code to the caller.
        "try:\n"
        "    encode(RawPcm(b'\\x00' * (2 * 2 * 4096), 44100, 2, 's16le'))\n"
        "except WwiseWemError as error:\n"
        "    assert error.code == 'PROFILE_NOT_FOUND', error.code\n"
        "else:\n"
        "    raise SystemExit('uninstalled 2ch/44100 selection was accepted')\n"
        "try:\n"
        "    import wwise_wem_reference\n"
        "except ImportError:\n"
        "    print('native-encode-ok')\n"
        "else:\n"
        "    raise SystemExit('reference tree importable from clean install')\n"
    )


def main() -> None:
    allowlist = _distribution_allowlist()
    with tempfile.TemporaryDirectory(prefix="wwise2013-wheel-") as directory:
        work = Path(directory)
        wheels = work / "wheels"
        source = work / "source"
        wheels.mkdir()
        # A stale development-tree .so must not ride into the wheel.
        # NOTE: shutil.ignore_patterns matches path *components* (basenames),
        # so a slashed pattern like "crates/target" never fires; match the
        # build caches by basename and drop the top-level corpus scratch
        # tree (not part of the wheel contents, multi-GB).
        def _ignore(dir_path: str, names: list[str]) -> set[str]:
            ignored = {
                name
                for name in names
                if name
                in {
                    ".git",
                    ".venv",
                    "venv",
                    "build",
                    "dist",
                    "target",
                    "__pycache__",
                    ".mypy_cache",
                    ".ruff_cache",
                    ".pytest_cache",
                }
                or name.endswith((".egg-info", ".pyc", ".so", ".dylib", ".pyd", ".whl"))
            }
            if Path(dir_path) == ROOT and "corpus" in names:
                ignored.add("corpus")
            return ignored

        shutil.copytree(
            ROOT,
            source,
            ignore=_ignore,
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
        _verify_wheel_contents(wheel, allowlist)

        # --- zip-import metadata path (pure Python subset) ---------------
        environment = dict(os.environ)
        environment["PYTHONPATH"] = str(wheel)
        result = subprocess.run(
            [sys.executable, "-c", _facade_metadata_smoke(allowlist)],
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
            [str(venv_python), "-c", _facade_metadata_smoke(allowlist)],
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
                _clean_venv_core_smoke(),
                str(SMOKE_INPUT),
                str(SMOKE_REFERENCE),
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
