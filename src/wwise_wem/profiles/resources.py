"""Zip-safe access to checksum-addressed package resources."""

from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass
from importlib import resources

try:  # Python >= 3.11
    from importlib.resources.abc import Traversable
except ModuleNotFoundError:  # Python 3.10 (deprecated location)
    from importlib.abc import Traversable  # type: ignore[attr-defined]
from pathlib import PurePosixPath
from typing import Any


_SHA256 = re.compile(r"[0-9a-f]{64}")


def normalize_resource_path(path: str | PurePosixPath) -> PurePosixPath:
    """Return a safe package-relative POSIX resource path."""
    if not isinstance(path, (str, PurePosixPath)):
        raise ValueError("package resource path must be text")
    text = str(path)
    value = PurePosixPath(text)
    segments = text.split("/")
    if (
        not text
        or "\\" in text
        or value.is_absolute()
        or any(part in ("", ".", "..") for part in segments)
    ):
        raise ValueError(f"unsafe package resource path: {text!r}")
    return value


def resource_traversable(package: str, path: str | PurePosixPath) -> Traversable:
    """Resolve a resource without materializing it as a filesystem path."""
    if not isinstance(package, str) or not package:
        raise ValueError("resource package must not be empty")
    relative = normalize_resource_path(path)
    return resources.files(package).joinpath(*relative.parts)


@dataclass(frozen=True)
class ResourceRef:
    """Immutable package resource identity validated by SHA-256."""

    package: str
    # Normalized to PurePosixPath in __post_init__; str inputs remain accepted.
    path: PurePosixPath
    sha256: str

    def __post_init__(self) -> None:
        if not isinstance(self.package, str) or not self.package:
            raise ValueError("resource package must not be empty")
        object.__setattr__(self, "path", normalize_resource_path(self.path))
        digest = str(self.sha256).lower()
        if _SHA256.fullmatch(digest) is None:
            raise ValueError("resource SHA-256 must contain 64 hex digits")
        object.__setattr__(self, "sha256", digest)

    def traversable(self) -> Traversable:
        return resource_traversable(self.package, self.path)

    def read_bytes(self) -> bytes:
        try:
            payload = self.traversable().read_bytes()
        except (FileNotFoundError, IsADirectoryError) as error:
            raise ValueError(f"missing package resource {self.path}") from error
        actual = hashlib.sha256(payload).hexdigest()
        if actual != self.sha256:
            raise ValueError(
                f"resource {self.path} SHA-256 differs: "
                f"expected {self.sha256}, actual {actual}"
            )
        return payload

    def read_text(self, encoding: str = "utf-8") -> str:
        return self.read_bytes().decode(encoding)

    def read_json(self) -> Any:
        try:
            return json.loads(self.read_text())
        except json.JSONDecodeError as error:
            raise ValueError(f"resource {self.path} is not valid JSON") from error

    def verify(self) -> None:
        self.read_bytes()
