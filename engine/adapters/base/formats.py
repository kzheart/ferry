"""Agent native-format profiles and version-aware selection."""
from __future__ import annotations

import copy
import re
from dataclasses import dataclass
from typing import Callable


Version = tuple[int, int, int]
TemplateFactory = Callable[[], dict]


def parse_version(value: str | None) -> Version | None:
    if not value:
        return None
    match = re.search(r"(\d+)\.(\d+)\.(\d+)", value)
    if not match:
        return None
    return tuple(int(part) for part in match.groups())


@dataclass(frozen=True)
class VersionRange:
    minimum: str
    maximum_exclusive: str

    def contains(self, value: str | None) -> bool:
        version = parse_version(value)
        minimum = parse_version(self.minimum)
        maximum = parse_version(self.maximum_exclusive)
        return bool(
            version is not None
            and minimum is not None
            and maximum is not None
            and minimum <= version < maximum
        )

    def describe(self) -> str:
        return f">={self.minimum},<{self.maximum_exclusive}"


@dataclass(frozen=True)
class FormatProfile:
    id: str
    output_version: str
    compatible: VersionRange
    tested_versions: tuple[str, ...]
    template_factory: TemplateFactory

    def templates(self) -> dict:
        return copy.deepcopy(self.template_factory())

    def status(self, installed_version: str | None) -> str:
        if installed_version in self.tested_versions:
            return "verified"
        if self.compatible.contains(installed_version):
            return "compatible"
        return "unsupported"


@dataclass(frozen=True)
class FormatRegistry:
    agent: str
    profiles: tuple[FormatProfile, ...]

    def __post_init__(self):
        if not self.profiles:
            raise ValueError(f"{self.agent} must define at least one format profile")
        ids = [profile.id for profile in self.profiles]
        if len(ids) != len(set(ids)):
            raise ValueError(f"{self.agent} format profile ids must be unique")

    @property
    def latest(self) -> FormatProfile:
        return max(
            self.profiles,
            key=lambda profile: parse_version(profile.output_version) or (0, 0, 0),
        )

    def resolve(self, installed_version: str | None) -> FormatProfile | None:
        exact = next(
            (
                profile
                for profile in self.profiles
                if installed_version in profile.tested_versions
            ),
            None,
        )
        if exact is not None:
            return exact
        compatible = [
            profile
            for profile in self.profiles
            if profile.compatible.contains(installed_version)
        ]
        return max(
            compatible,
            key=lambda profile: parse_version(profile.output_version) or (0, 0, 0),
            default=None,
        )

    def templates(self, installed_version: str | None = None) -> dict:
        profile = self.resolve(installed_version) if installed_version else self.latest
        if profile is None:
            raise RuntimeError(
                f"{self.agent} {installed_version} has no compatible format profile"
            )
        return profile.templates()

    def inspect(self, installed_version: str | None) -> dict:
        profile = self.resolve(installed_version)
        if profile is None:
            return {
                "profile": None,
                "status": "unsupported" if installed_version else "unknown",
                "supported_range": None,
                "output_version": None,
                "tested_versions": [],
            }
        return {
            "profile": profile.id,
            "status": profile.status(installed_version),
            "supported_range": profile.compatible.describe(),
            "output_version": profile.output_version,
            "tested_versions": list(profile.tested_versions),
        }
