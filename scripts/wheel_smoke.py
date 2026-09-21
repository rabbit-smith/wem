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
  encodes the golden input byte-exactly through the embedded kernel and
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
ALLOWLIST = ROOT / "tests" / "contract" / "distribution_allowlist.json"
GOLDEN_INPUT = ROOT / "tests" / "fixtures" / "input.wav"
GOLDEN_REFERENCE = ROOT / "tests" / "fixtures" / "reference.wem"


def _distribution_contract() -> dict[str, object]:
    return json.loads(ALLOWLIST.read_text(encoding="utf-8"))


def _expected_extension_artifacts(contract: dict[str, object]) -> list[str]:
    """Platform artifact names for the allowlisted native extensions."""
    extension = ".pyd" if os.name == "nt" else ".so"
    return [
        f"{module.replace('.', '/')}.abi3{extension}"
        for module in contract["native_extensions"]
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


def _verify_wheel_contents(wheel: Path, contract: dict[str, object]) -> None:
    expected = sorted(
        _canonical_wheel_entry(name)
        for name in [
            *contract["modules"],
            *contract["resources"],
            *_expected_extension_artifacts(contract),
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
    extension_artifacts = {
        _canonical_wheel_entry(name) for name in _expected_extension_artifacts(contract)
    }
    if not extension_artifacts & set(actual):
        raise RuntimeError(
            f"wheel does not carry the native extension {sorted(extension_artifacts)}; "
            "the single wheel must embed its engine"
        )


def _facade_metadata_smoke(contract: dict[str, object]) -> str:
    """Metadata-path smoke shared by the zip and installed import paths.

    The profile is resolved by key (generation, channels, sample rate) over
    the bundle inventory the index declares, never by a profile name; the
    digest chain (payload -> manifest SHA -> index SHA) is verified by
    ``load_profile_bundle`` plus ``verify_all``.  The native selector types
    are deliberately out of reach here: an abi3 extension cannot be imported
    from inside a zip, so this smoke never touches ``wwise_wem._core``.
    """
    return (
        "import wwise_wem; "
        f"assert wwise_wem.__all__=={contract['root_exports']!r}; "
        "from wwise_wem.profiles.bundle import installed_profile_names, "
        "load_profile_bundle; "
        "installed=[load_profile_bundle(profile=n, verify_all=False) "
        "for n in installed_profile_names()]; "
        "matches=[b for b in installed "
        "if (b.key.generation,b.key.channels,b.key.sample_rate)"
        "==('2013.2',6,44100)]; "
        "assert len(matches)==1, matches; "
        "b=matches[0]; b.verify_all(); "
        "assert b.runtime_manifest.resource('vorbis.setup')==b.setup; "
        "assert set(b.runtime_manifest.resources)=={"
        "'vorbis.setup','vorbis.codebooks.t97','vorbis.codebooks.t219',"
        "'transform.mdct','analysis.transient',"
        "'psychoacoustics.short-profiles','psychoacoustics.short-seed',"
        "'psychoacoustics.long-base','psychoacoustics.long-modes',"
        "'analysis.frozen-tables'}; "
        "assert len(b.setup_packet())==201; "
        "print(b.key.generation, b.key.channels, b.key.sample_rate, "
        "len(b.setup_packet()), len(b.runtime_manifest.resources))"
    )


def _clean_venv_core_smoke() -> str:
    """Installed facade must encode byte-exactly through the embedded kernel."""
    return (
        "import hashlib\n"
        "import sys\n"
        "from wwise_wem import encode\n"
        "import wwise_wem._core as core\n"
        "assert hasattr(core, 'Encoder'), core\n"
        "result = encode(sys.argv[1])\n"
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
        # NOTE: shutil.ignore_patterns matches path *components* (basenames),
        # so a slashed pattern like "crates/target" never fires; match the
        # build caches by basename and drop the top-level corpus scratch
        # tree (not part of the wheel contract, multi-GB).
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
                _clean_venv_core_smoke(),
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
