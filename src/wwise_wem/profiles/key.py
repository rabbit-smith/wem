"""Encoder profile identity.

The profile values live in the kernel as Rust constants
(``crates/wem-profiles/src/generated``); what a language that needs them holds
is the identity below plus whatever it decodes from the compiled artifact
(``wwise_wem._core.profile_tables()``). There is no profile *name*: the human
label is derived from the identity, so nothing stores a label that could drift
from the values it names. The identity is the key itself — generation,
geometry and channel layout — and nothing derived from the setup packet: the
packet travels as bytes, so a digest carried beside it would only ever be
checked against those bytes.
"""

from __future__ import annotations

from dataclasses import dataclass

#: The Wwise generation of the installed profiles.
WWISE_GENERATION = "2013.2"
#: The short spelling of the same generation, as it appears in messages, on a
#: command line, and in a selection description; mirrors the kernel's
#: ``WwiseVersion::label()`` (the oracle and the tooling cannot read that type,
#: because they must also run where the native extension is absent).
WWISE_GENERATION_LABEL = "2013"


def profile_label(channels: int, sample_rate: int, generation: str) -> str:
    """The human label for one profile identity: geometry plus generation."""
    return f"{channels}ch/{sample_rate}Hz/{generation}"


def profile_description(
    channels: int,
    sample_rate: int,
    generation: str,
    channel_layout: str,
) -> str:
    """The complete description of one identity, for resolution diagnostics.

    Two profiles that share generation and geometry are told apart here and
    nowhere else — the label alone would print them identically, which is
    exactly the ambiguity a selection cannot resolve.
    """
    return f"{profile_label(channels, sample_rate, generation)}/{channel_layout}"


@dataclass(frozen=True, order=True)
class ProfileKey:
    """Complete encoder profile identity."""

    channels: int
    sample_rate: int
    generation: str
    channel_layout: str

    def __post_init__(self) -> None:
        if self.channels <= 0 or self.sample_rate <= 0:
            raise ValueError("profile channels and sample rate must be positive")
        if not self.generation:
            raise ValueError("profile generation must not be empty")
        if not self.channel_layout:
            raise ValueError("profile channel layout must not be empty")

    def label(self) -> str:
        """The human label for this profile, derived from the identity."""
        return profile_label(self.channels, self.sample_rate, self.generation)

    def describe(self) -> str:
        """Complete identity description used in resolution diagnostics."""
        return profile_description(
            self.channels,
            self.sample_rate,
            self.generation,
            self.channel_layout,
        )


__all__ = [
    "ProfileKey",
    "WWISE_GENERATION",
    "WWISE_GENERATION_LABEL",
    "profile_description",
    "profile_label",
]
