#!/usr/bin/env python3
"""Fail a commit that stages an asset, a development-tree file, or a root entry it should not.

Three rules, each one an accident this repository has already had. This script is
the cheap early feedback that keeps the accident out of history; it is not the
verdict. CI is (see `AGENTS.md`, "Verification ladder"). It reads the git index
and the tracked list, runs no build, needs no kernel, and touches no network.

1. Media and bank payloads (`.wem`, `.bnk`, `.pck`, `.wav`, `.ogg`, `.mp3`,
   `.flac`) are versioned test data and live under `tests/` only, where
   `tests/AGENTS.md` sanctions them. One staged at the repository root or under
   `docs/` is a download artefact, not an asset.
2. Nothing from the untracked development trees (`corpus/`, `local/`) is ever
   committed. `.gitignore` already lists both, so this is the belt-and-braces
   check for a `git add -f`.
3. A file at the repository root must be a name somebody wrote down on purpose.
   Two stray download artefacts -- an HTML captcha page and a saved JSON report
   -- once sat in the root, unignored and one `git add -A` from being published.

Usage:
    python3 scripts/check_staged_assets.py                # the staged change set
    python3 scripts/check_staged_assets.py --all-tracked  # the whole tracked tree

Exit status: 0 clean, 1 for any violation, 2 when git cannot be read.
"""

from __future__ import annotations

import argparse
import subprocess
import sys

from collections.abc import Sequence
from pathlib import Path
from typing import NamedTuple

# `.gitignore` lists the untracked development trees; this check catches the
# `git add -f` that bypasses the ignore rule and would publish them anyway.
DEVELOPMENT_TREES = ("corpus", "local")

# Payload kinds a suite reads as data and a download leaves behind as a file.
ASSET_SUFFIXES = (".wem", ".bnk", ".pck", ".wav", ".ogg", ".mp3", ".flac")

# `tests/AGENTS.md` heads the sanctioned set with "Versioned assets
# (`tests/data/`, `tests/fixtures/`)": the suites generate their PCM in process
# and hold the paired build's containers under those two trees.
SANCTIONED_ASSET_PREFIX = "tests/"

# The expected repository-root entries. Derived from `git ls-files` at HEAD plus
# the literal root-level names in `.gitignore`, then written out here so that a
# new one has to be added deliberately.
#
# UPDATE THIS SET DELIBERATELY. An entry added here widens what may be published
# at the root; it is not a formality. A name that is absent is a stray file --
# the two download artefacts found in the root were exactly that -- and the
# commit fails until the path is unstaged or this set is edited by the change
# that intends to publish the name.
ROOT_ALLOWED_ENTRIES = frozenset(
    {
        # Files tracked at HEAD.
        ".gitattributes",
        ".gitignore",
        ".pi-lens.json",
        "AGENTS.md",
        "CONTRIBUTING.md",
        "LICENSE",
        "LICENSE-APACHE",
        "LICENSE-MIT",
        "Makefile",
        "PROVENANCE.md",
        "pyproject.toml",
        "pyrightconfig.json",
        "README.md",
        "THIRD_PARTY_NOTICES.md",
        # Trees tracked at HEAD.
        ".github",
        "crates",
        "docs",
        "examples",
        "include",
        "js",
        "reference",
        "scripts",
        "src",
        "tests",
        # The versioned hook directory (`core.hooksPath`), added with the hooks
        # themselves -- see `docs/guides/hooks.md`.
        ".githooks",
        # Literal root-level names `.gitignore` declares: build output, tool
        # caches, the untracked development trees, local machine noise. Expected
        # to exist in a working checkout, never published.
        ".DS_Store",
        ".claude",
        ".coverage",
        ".mypy_cache",
        ".pi",
        ".pytest_cache",
        ".ruff_cache",
        ".understand-anything",
        ".venv",
        ".workflow",
        "__pycache__",
        "build",
        "coverage.xml",
        "corpus",
        "dist",
        "htmlcov",
        "local",
        "native-wheel",
        "output.wem",
    }
)

# The reason a rule exists, and where the offending path belongs instead. Printed
# once per rule that a run tripped, beneath the one-line violations.
RULE_HELP = {
    "media-outside-tests": (
        f"Media and bank payloads may only be committed under {SANCTIONED_ASSET_PREFIX}.\n"
        'tests/AGENTS.md, "Versioned assets (`tests/data/`, `tests/fixtures/`)":\n'
        "  `fixtures/input.wav` is the input the encoding suites read; a suite that needs\n"
        "  different PCM builds it in process. `fixtures/reference.wem` is the paired\n"
        "  build's output for that input.\n"
        "Move the payload under tests/data/ or tests/fixtures/, or unstage it."
    ),
    "development-tree": (
        "The development trees are untracked and are never published. AGENTS.md,\n"
        '"Provenance hygiene":\n'
        "  `corpus/` and `local/` are untracked and are never published: `corpus/` holds\n"
        "  unverified extraction and development material, `local/` holds working material\n"
        "  that is never a packaged asset.\n"
        "Unstage it (git restore --staged <path>); publishing it is a decision, not an add."
    ),
    "unexpected-root-entry": (
        "The repository root is a published surface, and every entry in it is deliberate:\n"
        "a tracked file or tree, or a name `.gitignore` declares. A file nobody named -- a\n"
        "saved web page, a scanner report, an export -- is a stray artefact one `git add -A`\n"
        "from being published. Unstage it, or add the entry to ROOT_ALLOWED_ENTRIES in this\n"
        "script in the change that intends to publish it."
    ),
}


class Violation(NamedTuple):
    """One rule tripped by one path, said in one line."""

    path: str
    rule: str
    detail: str

    def line(self) -> str:
        return f"{self.path}: {self.rule}: {self.detail}"


def _repo_root() -> Path:
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        capture_output=True,
        encoding="utf-8",
        errors="replace",
    )
    if result.returncode != 0:
        raise SystemExit("check_staged_assets: not inside a git working tree.")
    return Path(result.stdout.strip())


def _git(repo_root: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo_root), *args],
        capture_output=True,
        encoding="utf-8",
        errors="replace",
    )
    if result.returncode != 0:
        raise SystemExit(
            f"check_staged_assets: git {' '.join(args)} failed "
            f"({result.returncode}): {result.stderr.strip()}"
        )
    return result.stdout


def _listed_paths(repo_root: Path, *, all_tracked: bool) -> list[str]:
    """The paths to check, NUL-separated by git so no name is ever misread.

    `-z` is what makes a path containing a space, a quote or a newline survive
    intact; `--diff-filter=ACMR` keeps a deletion out of the set, because nothing
    enters the tree by deleting something.
    """
    if all_tracked:
        listing = _git(repo_root, "ls-files", "-z")
    else:
        listing = _git(
            repo_root, "diff", "--cached", "--name-only", "-z", "--diff-filter=ACMR"
        )
    return [name for name in listing.split("\0") if name]


def _check_assets(paths: Sequence[str]) -> list[Violation]:
    violations: list[Violation] = []
    for path in paths:
        suffix = Path(path).suffix.lower()
        if suffix not in ASSET_SUFFIXES or path.startswith(SANCTIONED_ASSET_PREFIX):
            continue
        violations.append(
            Violation(
                path,
                "media-outside-tests",
                f"{suffix} is a media or bank payload outside the sanctioned asset tree "
                f"({SANCTIONED_ASSET_PREFIX})",
            )
        )
    return violations


def _check_development_trees(paths: Sequence[str]) -> list[Violation]:
    violations: list[Violation] = []
    for path in paths:
        root_entry = path.split("/", 1)[0]
        if root_entry not in DEVELOPMENT_TREES:
            continue
        violations.append(
            Violation(
                path,
                "development-tree",
                f"{root_entry}/ is untracked development material and is never committed "
                "(.gitignore already lists it; only a force-add gets here)",
            )
        )
    return violations


def _check_root_entries(paths: Sequence[str]) -> list[Violation]:
    """One violation per unexpected root entry, naming the first path under it.

    Deduplicated because a stray directory can contribute thousands of staged
    paths, and one line per entry is the actionable statement.
    """
    unexpected: dict[str, list[str]] = {}
    for path in paths:
        root_entry = path.split("/", 1)[0]
        if root_entry in ROOT_ALLOWED_ENTRIES:
            continue
        unexpected.setdefault(root_entry, []).append(path)

    violations: list[Violation] = []
    for root_entry, members in sorted(unexpected.items()):
        extra = (
            ""
            if len(members) == 1
            else f" (+{len(members) - 1} more staged under {root_entry}/)"
        )
        violations.append(
            Violation(
                members[0],
                "unexpected-root-entry",
                f'"{root_entry}" is not an expected repository-root entry{extra}',
            )
        )
    return violations


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Fail a commit that stages an asset, a development-tree file, "
        "or an unexpected repository-root entry."
    )
    parser.add_argument(
        "--all-tracked",
        action="store_true",
        help="check git ls-files (the whole tracked tree) instead of the staged change set",
    )
    args = parser.parse_args(argv)

    repo_root = _repo_root()
    paths = _listed_paths(repo_root, all_tracked=args.all_tracked)
    scope = "tracked tree" if args.all_tracked else "staged change set"

    violations = [
        *_check_assets(paths),
        *_check_development_trees(paths),
        *_check_root_entries(paths),
    ]
    if not violations:
        print(f"check_staged_assets: ok — {len(paths)} path(s) in the {scope}, no rule tripped.")
        return 0

    # The report goes to stderr: a failing check must still be readable when a
    # caller redirects stdout, and the exit status is the verdict either way.
    print(f"check_staged_assets: {len(violations)} violation(s) in the {scope}", file=sys.stderr)
    print("", file=sys.stderr)
    for violation in violations:
        print(f"  {violation.line()}", file=sys.stderr)

    for rule in dict.fromkeys(violation.rule for violation in violations):
        print("", file=sys.stderr)
        print(f"{rule}:", file=sys.stderr)
        for line in RULE_HELP[rule].splitlines():
            print(f"  {line}", file=sys.stderr)

    print("", file=sys.stderr)
    print(
        "Bypass deliberately with `git commit --no-verify`; CI is still the verdict.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
