#!/usr/bin/env python3
"""The provenance gate: no development-process coordinates on a published surface.

The pattern sets and the scan live here rather than in the suite, so that the
gate can run before a build: `tests/parity/test_distribution.py` imports this
module, and a pre-commit hook runs it as a script. Nothing here imports
`wwise_wem` — this is the one check that must work on a fresh checkout with no
compiled extension.

    python3 scripts/check_provenance.py             # every published surface
    python3 scripts/check_provenance.py --staged    # the content in the index
    python3 scripts/check_provenance.py PATH...     # these files

A violation prints as `path:line: label: match`, one per line; line 0 means the
path itself is the surface. Exit status is 0 when clean, 1 on any violation, 2
when the scan could not run (not a git checkout, unreadable index or path).
Standard library only, no ambient state beyond the repository and the index.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys

from pathlib import Path
from typing import Callable, Iterable, Iterator, NamedTuple, Sequence

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = ROOT / "src" / "wwise_wem"
PACKAGE_PREFIX = PACKAGE.relative_to(ROOT).as_posix() + "/"
TEXT_SUFFIXES = {".py", ".json"}

# These expressions target provenance markers, not ordinary hexadecimal data.
# RIFF constants, packed float words, and SHA-256 digests therefore remain valid.
# Assembled from adjacent literals so that this module does not contain, as a
# literal, any string its own patterns match. The module is published: a pattern
# that doubles as an example of what it forbids defeats itself.
ADDRESS_LABEL = "R" "VA"
MODULE_LABEL = "D" "LL"
IMAGE_LABEL = "Conversion" "Plugin"
SECTION_LABEL = "r" "data"
POSIX_HOME = "/Users" "/"

PROVENANCE_PATTERNS = {
    "disassembler symbol name": re.compile(
        r"(?i)\b(?:sub|loc|off|byte|word|dword|qword|unk|stru|nullsub|asc|flt|dbl)"
        r"_[0-9a-f]{3,}\b"
    ),
    # The bare suffix of such a symbol, standing alone in prose — the companion
    # of the pattern above. Narrow on purpose, and calibrated in the note below
    # the pattern sets; `(?m)` makes `^` a line start, so the line-based scan
    # and a whole-file scan agree on where a word begins.
    "bare address suffix": re.compile(
        r"(?im)(?:^|(?<=[ \t]))"
        r"(?=[0-9a-fA-F]{4}(?![0-9a-zA-Z_]))"
        r"(?=[0-9a-fA-F]*[a-fA-F])(?=[0-9a-fA-F]*[0-9])"
        r"[0-9a-fA-F]{4}(?=[ \t]+[a-z])"
    ),
    "binary address label": re.compile(r"(?i)\b" + ADDRESS_LABEL + r"\b"),
    "binary module label": re.compile(r"(?i)\b" + MODULE_LABEL + r"\b"),
    # `0x10203040` is the synthetic frame count the container suites use as
    # test data, not an image address, so it is excluded here.
    "image address": re.compile(r"(?i)\b0x10(?!203040)[0-9a-f]{6}\b"),
    "virtual address": re.compile(r"(?i)\bVA 0x[0-9a-f]+"),
    "binary image label": re.compile(
        r"(?i)" + IMAGE_LABEL + r"|\b" + SECTION_LABEL + r"\b"
    ),
    "local absolute path": re.compile(
        r"(?i)(?:^|[\s\"'])(?:" + POSIX_HOME + r"|/home/|[a-z]:\\\\users\\\\)"
    ),
    "internal lane label": re.compile(r"\bLane-[A-Z]\b"),
    # Escaped, so that naming the pattern here does not itself disclose the
    # internal report reference it looks for.
    "internal report reference": re.compile("\u00a7\u673a\u5236|\breport \u00a7"),
    "temporary path": re.compile(r"(?i)(?:^|[\s\"'])/tmp(?:/|\b)"),
    "tool workspace path": re.compile(r"(?i)(?:^|[\s\"'])tools[/\\]"),
}

# Markers a development document may legitimately discuss — the untracked
# development corpus and the round labels of the work that produced a value —
# but which a shipped or compiled surface may only name as a live reference.
CODE_SURFACE_PATTERNS = {
    # `corpus/profiles/` is the documented untracked development-material tree
    # the profile generators read, so naming it is a live reference. Any other
    # `corpus/<...>` subpath points at a development artifact no reader can
    # obtain, and the bare word is allowed so a surface can state that it does
    # not read the tree at all.
    "development corpus path": re.compile(r"(?i)\bcorpus/(?!profiles\b)[a-z0-9]"),
    "development round label": re.compile(r"\bR-\d+[a-z]?\b"),
    "development script name": re.compile(r"\bgen_r\d+[a-z0-9_]*\.py\b"),
}

# The bare suffix of an address, written by itself in prose, is caught by `bare
# address suffix` above. That pattern is narrow on purpose, and the calibration
# behind its shape is recorded rather than remembered:
#
#   * the broad form, `(?i)\b[0-9a-f]{4}\b`, is unusable: 1824 hits across 198
#     of the tracked files — the Wwise year, block sizes, Rust float-suffix
#     literals, exponents of quantised values written in scientific notation,
#     linter codes, Huffman codewords and test data.
#   * requiring at least one a-f letter and one digit leaves 142 hits;
#     requiring exactly two of each still leaves test data and misses two of the
#     five suffixes this pattern was calibrated against.
#   * the shape above — case-insensitive, one letter and one digit, delimited by
#     the line start or a blank, followed by a blank and a lowercase word — is
#     the narrowest form that catches all five, and it hits nothing on the
#     tracked tree (366 files when that was measured). Re-measure it over
#     `tracked_names()` rather than trusting these counts.
#
# What it does not catch, said plainly so that no reader mistakes it for
# closure: a suffix made only of a-f letters (no pattern can tell one of those
# from an English word, which is why a digit is required); a suffix set off by
# punctuation rather than blanks, or ending its line; and a suffix written into
# a table cell or a code identifier rather than into prose. What it can cost: a
# bare quantised value written in scientific notation inside a sentence, and
# any other four-hex word with this shape, must be reworded rather than
# allowed — which is the price of catching the shape at all.

# Surfaces whose text is compiled into a shipped artifact or read from the
# published tree: the Rust kernel (its comments and strings end up in the
# extension), the Python reference the suites drive, the tooling and the
# suites themselves.
CODE_SURFACE_PREFIXES = (
    ("crates/", ".rs"),
    ("reference/", ".py"),
    ("reference/", ".json"),
    ("scripts/", ".py"),
    ("tests/", ".py"),
    ("tests/", ".json"),
)

# Documents are the tracked Markdown and YAML plus the root files no suffix
# rule reaches. The root files are listed rather than matched by a wildcard so
# that every one of them is a deliberate entry: `undeclared_root_files` fails
# the gate when a new root entry appears, instead of leaving it unscanned. A
# licence text is listed like any other because a document is a document.
DOCUMENT_SUFFIXES = (".md", ".yml")
ROOT_TEXT_FILES = (
    ".gitattributes",
    ".gitignore",
    "LICENSE",
    "LICENSE-APACHE",
    "LICENSE-MIT",
    "Makefile",
    "pyproject.toml",
    "pyrightconfig.json",
)
# Root entries that are binary or generated and that the surface above
# deliberately does not scan. Empty: every tracked root entry is text today.
ROOT_NON_TEXT_FILES: tuple[str, ...] = ()

# The guard below reads the *tracked* root entries, and that is a deliberate
# loosening: a working-tree artefact sitting at the root is not a published
# surface and needs no entry. `.pi-lens.json`, the local linter configuration
# that `.gitignore` still names, was exactly that until it was untracked. A root
# file becomes a surface the moment it is tracked, and then it is declared. The
# remedy the guard prints names this file and this tuple, because a stray root
# file is resolved by declaring it, not by loosening the scan.
GATE_FILE = Path(__file__).resolve().relative_to(ROOT).as_posix()
ROOT_GUARD_REMEDY = (
    f"add it to ROOT_TEXT_FILES in {GATE_FILE}, deliberately "
    "(or to ROOT_NON_TEXT_FILES when it is binary or generated)"
)


class GitError(RuntimeError):
    """A git invocation the gate depends on did not succeed."""


class Violation(NamedTuple):
    """One pattern hit, at a line number (0 when the path itself is the surface)."""

    path: str
    line: int
    label: str
    match: str

    def report(self) -> str:
        """The one-line form the pre-commit hook and the CLI print."""
        return f"{self.path}:{self.line}: {self.label}: {self.match}"


def violations(
    surface: str,
    patterns: Iterable[tuple[str, re.Pattern[str]]] | None = None,
) -> list[tuple[str, str]]:
    """(label, matched text) for every pattern hit in one surface string."""
    if patterns is None:
        patterns = PROVENANCE_PATTERNS.items()
    return [
        (label, match.group(0))
        for label, pattern in patterns
        for match in pattern.finditer(surface)
    ]


def surface_patterns(name: str) -> tuple[tuple[str, re.Pattern[str]], ...]:
    """The patterns a repo-relative path is held to.

    A shipped or published code surface is held to the development-corpus
    patterns as well, because it may only point at a live reference.
    """
    if is_code_surface(name):
        return (*PROVENANCE_PATTERNS.items(), *CODE_SURFACE_PATTERNS.items())
    return tuple(PROVENANCE_PATTERNS.items())


def is_code_surface(name: str) -> bool:
    """Whether a repo-relative path is one of the compiled or published code surfaces."""
    return any(
        name.startswith(prefix) and name.endswith(suffix)
        for prefix, suffix in CODE_SURFACE_PREFIXES
    )


def _git(*arguments: str) -> bytes:
    """Run one git command in this repository and return its raw stdout.

    Raw bytes, because the gate scans content: decoding here would guess twice.
    """
    result = subprocess.run(
        ["git", "-C", str(ROOT), *arguments],
        capture_output=True,
    )
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", errors="replace").strip()
        raise GitError(f"git {' '.join(arguments)}: {detail}")
    return result.stdout


def _git_names(*arguments: str) -> tuple[str, ...]:
    """NUL-separated git output as sorted names."""
    listing = _git(*arguments).decode("utf-8", errors="surrogateescape")
    return tuple(sorted(name for name in listing.split("\0") if name))


def _has_head() -> bool:
    """Whether the repository has a commit yet."""
    result = subprocess.run(
        ["git", "-C", str(ROOT), "rev-parse", "--verify", "-q", "HEAD"],
        capture_output=True,
    )
    return result.returncode == 0


def tracked_names() -> tuple[str, ...]:
    """Every tracked path, repo-relative and sorted.

    The tracked set is the published surface. A recursive glob is not: it also
    reads build output such as `crates/target/**/out/embedded_profiles.rs`,
    which is ignored and never published.
    """
    return _git_names("ls-files", "-z")


def tracked_files(*suffixes: str) -> Iterator[Path]:
    """Tracked files ending in one of `suffixes`, sorted by path."""
    for name in tracked_names():
        if name.endswith(suffixes):
            yield ROOT / name


def package_files() -> Iterator[Path]:
    """Every file in the package directory, excluding bytecode caches."""
    for path in sorted(PACKAGE.rglob("*")):
        if path.is_file() and "__pycache__" not in path.parts:
            yield path


def code_surface_files() -> Iterator[Path]:
    """Every tracked code surface, excluding this module.

    This module is excluded because it defines the patterns; scanning it would
    only ever match its own definitions.
    """
    this_file = Path(__file__).resolve()
    for path in tracked_files(*{suffix for _, suffix in CODE_SURFACE_PREFIXES}):
        if not is_code_surface(path.relative_to(ROOT).as_posix()):
            continue
        if path.resolve() == this_file:
            continue
        yield path


def document_files() -> Iterator[Path]:
    """Every document published with the tree, in path order.

    The tracked Markdown and YAML — the root documents, the subtree guides, the
    documentation set — plus the root text files, which include the CI manifest
    the packaging steps read.
    """
    for name in tracked_names():
        if name.endswith(DOCUMENT_SUFFIXES) or name in ROOT_TEXT_FILES:
            yield ROOT / name


def json_strings(value: object, location: str = "$") -> Iterator[tuple[str, str]]:
    """(JSON pointer, text) for every key and string value in a decoded document."""
    if isinstance(value, dict):
        for key, child in value.items():
            key_location = f"{location}.{key}"
            yield key_location, str(key)
            yield from json_strings(child, key_location)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from json_strings(child, f"{location}[{index}]")
    elif isinstance(value, str):
        yield location, value


def decode(data: bytes) -> str | None:
    """The content as text, or None for binary content.

    A NUL byte in the first block marks a binary blob — the same test git uses
    to decide a blob is not text. The path is still a surface; those bytes are
    not, because a pattern cannot be read out of them and matching them anyway
    would report an address that no reader can see.
    """
    if b"\0" in data[:8192]:
        return None
    return data.decode("utf-8", errors="replace")


def path_violations(name: str) -> Iterator[Violation]:
    """A path is a published surface too: a file name may not name a coordinate."""
    for label, match in violations(name):
        yield Violation(name, 0, label, match)


def line_violations(name: str, text: str) -> Iterator[Violation]:
    """Every pattern hit in the content of `name`, with 1-based line numbers."""
    for number, line in enumerate(text.splitlines(), start=1):
        for label, match in violations(line, surface_patterns(name)):
            yield Violation(name, number, label, match)


def scan(names: Iterable[str], read: Callable[[str], str | None]) -> Iterator[Violation]:
    """Scan each name's path and, when it is text, its content.

    Every scanned path is read as text whatever its suffix, so a surface the
    suffix rules do not know yet still cannot smuggle a coordinate in; only its
    pattern set differs. `read` returns None for binary content.
    """
    for name in names:
        yield from path_violations(name)
        text = read(name)
        if text is not None:
            yield from line_violations(name, text)


def undeclared_root_files(names: Iterable[str] | None = None) -> list[Violation]:
    """Root entries that no surface rule accounts for.

    A new root file is a deliberate addition to `ROOT_TEXT_FILES` — or to
    `ROOT_NON_TEXT_FILES` when it is binary or generated — so the gate names it
    rather than leaving it silently unscanned. The violation carries the remedy
    in its `match` field, because there is no matched text to report. `names`
    defaults to the tracked root entries; the staged scan passes the index's own
    root paths.
    """
    candidates = tracked_names() if names is None else tuple(names)
    declared = set(ROOT_TEXT_FILES) | set(ROOT_NON_TEXT_FILES)
    return [
        Violation(name, 0, "undeclared root surface", ROOT_GUARD_REMEDY)
        for name in candidates
        if "/" not in name
        and name not in declared
        and not name.endswith(DOCUMENT_SUFFIXES)
    ]


def _ordered(found: Iterable[Violation]) -> tuple[Violation, ...]:
    """Violations in one stable order, so two runs print the same report."""
    return tuple(sorted(found, key=lambda item: (item.path, item.line, item.label, item.match)))


def audited_names() -> tuple[str, ...]:
    """Every published surface, in path order.

    The package (its paths and its text), the code surfaces, the documents and
    the root files — the surface the suite covers, plus the path and root-entry
    rules that keep a future file from arriving unscanned.
    """
    names = []
    for name in tracked_names():
        if (
            name.startswith(PACKAGE_PREFIX)
            or is_code_surface(name)
            or name.endswith(DOCUMENT_SUFFIXES)
            or name in ROOT_TEXT_FILES
        ):
            names.append(name)
    return tuple(names)


def read_tracked(name: str) -> str | None:
    """The content of a tracked file."""
    return decode((ROOT / name).read_bytes())


def read_staged(name: str) -> str | None:
    """The index content of a path: what the next commit would carry."""
    return decode(_git("show", f":{name}"))


def staged_names() -> tuple[str, ...]:
    """Index paths whose staged content differs from HEAD.

    A path deleted from the index is not listed: there is nothing left to scan.
    In a repository with no commit yet the whole index is staged by definition,
    so it is scanned whole rather than reported as an unreadable index.
    """
    if not _has_head():
        return _git_names("ls-files", "-z", "--cached")
    return _git_names(
        "diff", "--cached", "--name-only", "-z", "--diff-filter=ACMRT"
    )


def audit_violations() -> tuple[Violation, ...]:
    """Every violation in the tracked tree: the scan the CLI runs with no arguments."""
    found = [*scan(audited_names(), read_tracked), *undeclared_root_files()]
    return _ordered(found)


def staged_violations() -> tuple[Violation, ...]:
    """Every violation in the staged content."""
    names = staged_names()
    found = [*scan(names, read_staged), *undeclared_root_files(names)]
    return _ordered(found)


def explicit_violations(paths: Sequence[str]) -> tuple[Violation, ...]:
    """Every violation in the files the caller named."""
    found: list[Violation] = []
    for argument in paths:
        path = Path(argument)
        try:
            name = path.resolve().relative_to(ROOT).as_posix()
        except ValueError:
            name = path.as_posix()
        found.extend(path_violations(name))
        text = decode(path.read_bytes())
        if text is not None:
            found.extend(line_violations(name, text))
    return _ordered(found)


def main(argv: Sequence[str] | None = None) -> int:
    """Run the gate over the index, the named paths, or the tracked tree."""
    parser = argparse.ArgumentParser(
        description="Scan published surfaces for development-process provenance.",
    )
    parser.add_argument(
        "--staged",
        action="store_true",
        help="scan the content staged in the index instead of the tree",
    )
    parser.add_argument(
        "paths",
        nargs="*",
        help="files to scan (default: every tracked surface)",
    )
    arguments = parser.parse_args(argv)
    if arguments.staged and arguments.paths:
        parser.error("--staged scans the index; it does not take paths")

    try:
        if arguments.staged:
            found = staged_violations()
        elif arguments.paths:
            found = explicit_violations(arguments.paths)
        else:
            found = audit_violations()
    except (GitError, OSError) as failure:
        print(f"check_provenance: {failure}", file=sys.stderr)
        return 2

    for violation in found:
        print(violation.report())
    return 1 if found else 0


if __name__ == "__main__":
    raise SystemExit(main())
