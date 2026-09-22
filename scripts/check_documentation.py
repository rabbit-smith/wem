#!/usr/bin/env python3
"""Check local Markdown links and current source-file references, without a build.

Historical findings may name retired source files in inline code. Their links,
like all other local Markdown links, must still resolve. Generated binary output,
shell commands and arbitrary prose are not treated as source references.
"""
from __future__ import annotations

import re
import subprocess
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"!?\[[^\]]*\]\(\s*(<[^>]+>|[^\s)]+)(?:\s+\"[^\"]*\")?\s*\)")
SOURCE = re.compile(
    r"`((?:crates|src|reference|tests|scripts|include|docs|js|examples)/"
    r"[\w./-]+\.(?:rs|py|md|toml|h|ts|mjs))`"
)


def check_document(path: Path, root: Path) -> list[str]:
    """Return broken targets with their source line; do not fetch external URLs."""
    text = path.read_text(encoding="utf-8")
    # Preserve line offsets while ignoring illustrative fenced code blocks.
    text = re.sub(r"(?ms)^(```|~~~)[^\n]*\n.*?^\1[^\n]*$",
                  lambda match: "\n" * match.group().count("\n"), text)
    failures = []
    for match in LINK.finditer(text):
        target = match[1].strip("<>")
        parsed = urlsplit(target)
        if parsed.scheme or parsed.netloc or not parsed.path:
            continue
        destination = (path.parent / unquote(parsed.path)).resolve()
        if not destination.exists():
            line = text.count("\n", 0, match.start()) + 1
            failures.append(f"{path.relative_to(root)}:{line}: missing link target: {target}")
    relative = path.relative_to(root).as_posix()
    if not relative.startswith("docs/findings/") and path.name != "THIRD_PARTY_NOTICES.md":
        for match in SOURCE.finditer(text):
            if not (root / match[1]).is_file() and not (path.parent / match[1]).is_file():
                line = text.count("\n", 0, match.start()) + 1
                failures.append(f"{relative}:{line}: missing source file: {match[1]}")
    return failures


def main() -> int:
    result = subprocess.run(
        ["git", "ls-files", "-z", "--", "*.md"], cwd=ROOT,
        check=True, capture_output=True,
    )
    failures = []
    for name in result.stdout.decode().split("\0"):
        if name:
            failures.extend(check_document(ROOT / name, ROOT))
    for failure in failures:
        print(failure)
    if not failures:
        print("Documentation links and current source references resolve.")
    return int(bool(failures))


if __name__ == "__main__":
    raise SystemExit(main())
