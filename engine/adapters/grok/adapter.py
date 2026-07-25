"""Grok Build read-side adapter composition."""
from pathlib import Path

from ...contracts.agents import AGENTS
from ...errors import AgentReferenceError, SessionNotFoundError
from ...system.paths import grok_home
from ..contracts import (
    AgentAdapter, AgentManifest, NativeSessionReference, filesystem_reference,
)
from ..shared.migration import TreeMigrationSource
from .lifecycle import GrokLifecycle
from .models import discover, fallback
from .reader import read
from .scanner import agent_fingerprint, fingerprint, scan

MANIFEST = AgentManifest(id="grok", **AGENTS["grok"])


def resolve(ref):
    root = (grok_home() / "sessions").resolve()
    path = Path(ref).expanduser()
    if path.is_dir():
        resolved = path.resolve()
        if resolved.is_relative_to(root):
            return resolved
        raise SessionNotFoundError("grok", ref)
    hits = []
    for summary in root.rglob("summary.json") if root.exists() else ():
        try:
            import json
            data = json.loads(summary.read_text())
        except (OSError, ValueError):
            continue
        if (data.get("info") or {}).get("id") == ref:
            hits.append(summary.parent.resolve())
    if len(hits) == 1:
        return hits[0]
    raise SessionNotFoundError("grok", ref)


class GrokBrowser:
    def scan(self, cache): return scan(cache)
    def read(self, ref): return read(str(resolve(ref)))
    def read_agent(self, ref): return self.read(ref)
    def resolve_ref(self, ref): return str(resolve(ref))
    def fingerprint(self, ref): return fingerprint(str(resolve(ref)))
    def agent_fingerprint(self, ref): return agent_fingerprint(str(resolve(ref)))

    def canonicalize(self, row):
        return filesystem_reference(
            row, MANIFEST.source_path, self.resolve_ref,
            kind="directory", required_name="summary.json",
        )

    def validate_read_scope(self, ref: NativeSessionReference):
        if ref.storage_kind != "directory" or not ref.root:
            raise AgentReferenceError("Grok 会话必须使用目录引用")
        path, root = Path(ref.canonical_ref).resolve(strict=True), \
            Path(ref.root).resolve(strict=True)
        if not path.is_dir() or not path.is_relative_to(root) \
                or not (path / "summary.json").is_file():
            raise AgentReferenceError("Grok 会话读取范围超出会话根目录")


class GrokModels:
    def discover(self): return discover()
    def fallback(self): return fallback()


def build():
    browser, lifecycle = GrokBrowser(), GrokLifecycle()
    lifecycle.executable = MANIFEST.executables[0]
    return AgentAdapter(
        manifest=MANIFEST, browser=browser,
        migration_source=TreeMigrationSource(browser),
        lifecycle=lifecycle, models=GrokModels(),
    )
