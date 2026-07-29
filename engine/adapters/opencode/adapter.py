"""OpenCode 当前原生结构的静态 Adapter 装配。"""
from __future__ import annotations

from ..contracts import (
    AgentManifest,
    AgentAdapter,
    NativeSessionReference,
    id_reference,
)
from ..shared.migration import TreeMigrationSource
from ...contracts.agents import AGENTS
from ...errors import AgentReferenceError
from .editor import OpenCodeBackend
from .lifecycle import OpenCodeLifecycle
from .migration import OpenCodeMigrationTarget
from .models import discover, fallback
from .probe import OpenCodeVerifier
from .scanner import (
    ensure_fingerprint_index_fresh,
    fingerprint,
    scan,
    scan_fingerprint,
)
from .reader import read, read_preview

MANIFEST = AgentManifest(id="opencode", **AGENTS["opencode"])


class OpenCodeBrowser:
    """OpenCode 的当前 SQLite 读取路径。"""

    def scan(self, cache):
        return scan(cache)

    def read(self, ref):
        return read(ref)

    def read_agent(self, ref):
        return read_preview(ref)

    def resolve_ref(self, ref):
        return ref

    def fingerprint(self, ref):
        return fingerprint(ref)

    def agent_fingerprint(self, ref):
        return fingerprint(ref)

    def scan_fingerprint(self, ref):
        # 扫描路径容忍落后一轮的快照,库频繁写入时不把全量刷新拖住。
        return scan_fingerprint(ref)

    def post_scan_maintenance(self):
        # 扫描完成后再补重建,避免重建与扫描并行互相拖慢。
        ensure_fingerprint_index_fresh()

    def canonicalize(self, row):
        return id_reference(row)

    def validate_read_scope(self, ref: NativeSessionReference) -> None:
        if ref.storage_kind != "id" or ref.root is not None:
            raise AgentReferenceError("OpenCode 会话引用必须由原生 id 支持")


class OpenCodeModels:
    def discover(self):
        return discover()

    def fallback(self):
        return fallback()


def build() -> AgentAdapter:
    browser = OpenCodeBrowser()
    lifecycle = OpenCodeLifecycle()
    lifecycle.executable = MANIFEST.executables[0]
    return AgentAdapter(
        manifest=MANIFEST,
        browser=browser,
        migration_source=TreeMigrationSource(browser),
        migration_target=OpenCodeMigrationTarget(),
        editor=OpenCodeBackend(),
        verifier=OpenCodeVerifier(),
        lifecycle=lifecycle,
        models=OpenCodeModels(),
    )
