"""Pi current-format adapter composition."""
from __future__ import annotations

from pathlib import Path
import json

from ...contracts.agents import AGENTS
from ...errors import AgentReferenceError, SessionNotFoundError
from ...system.paths import pi_session_roots
from ..contracts import (
    AgentAdapter, AgentManifest, NativeSessionReference, filesystem_reference,
)
from ..shared.migration import TreeMigrationSource
from .editor import PiBackend
from .lifecycle import PiLifecycle
from .migration import PiMigrationTarget
from .models import discover, fallback
from .probe import PiVerifier
from .reader import read
from .scanner import agent_fingerprint, fingerprint, scan

MANIFEST = AgentManifest(id="pi", **AGENTS["pi"])


def resolve(ref: str) -> Path:
    roots = [root.resolve() for root in pi_session_roots() if root.exists()]
    path = Path(ref).expanduser()
    if path.is_file():
        resolved = path.resolve()
        if any(resolved.is_relative_to(root) for root in roots):
            return resolved
        raise SessionNotFoundError("pi", ref)
    hits = []
    for root in roots:
        for candidate in root.rglob("*.jsonl"):
            try:
                header = json.loads(candidate.open().readline())
            except (OSError, ValueError, TypeError):
                continue
            if header.get("type") == "session" and header.get("version") == 3 \
                    and header.get("id") == ref:
                hits.append(candidate.resolve())
    if len(hits) == 1:
        return hits[0]
    raise SessionNotFoundError("pi", ref)


class PiBrowser:
    def scan(self, cache):
        return scan(cache)

    def read(self, ref):
        return read(str(resolve(ref)))

    def read_agent(self, ref):
        return self.read(ref)

    def resolve_ref(self, ref):
        return str(resolve(ref))

    def fingerprint(self, ref):
        return fingerprint(str(resolve(ref)))

    def agent_fingerprint(self, ref):
        return agent_fingerprint(str(resolve(ref)))

    def canonicalize(self, row):
        for root in pi_session_roots():
            ref = filesystem_reference(
                row, str(root), self.resolve_ref, kind="file",
            )
            if ref and Path(ref.canonical_ref).suffix == ".jsonl":
                return ref
        return None

    def validate_read_scope(self, ref: NativeSessionReference):
        if ref.storage_kind != "file" or not ref.root:
            raise AgentReferenceError("Pi 会话必须使用文件引用")
        path = Path(ref.canonical_ref).resolve(strict=True)
        root = Path(ref.root).resolve(strict=True)
        if (
            not path.is_file()
            or path.suffix != ".jsonl"
            or not path.is_relative_to(root)
        ):
            raise AgentReferenceError("Pi 会话读取范围超出会话根目录")


class PiModels:
    def discover(self):
        return discover()

    def fallback(self):
        return fallback()


def build():
    browser, lifecycle = PiBrowser(), PiLifecycle()
    lifecycle.executable = MANIFEST.executables[0]
    return AgentAdapter(
        manifest=MANIFEST,
        browser=browser,
        migration_source=TreeMigrationSource(browser),
        migration_target=PiMigrationTarget(),
        editor=PiBackend(),
        verifier=PiVerifier(),
        lifecycle=lifecycle,
        models=PiModels(),
    )
