"""Codex 当前原生结构的静态 Adapter 装配。"""
from __future__ import annotations

from ..base.plugin import ToolManifest, ToolPlugin, jsonl_reference
from ..base.migration import TreeMigrationSource
from ...contracts.agents import AGENTS
from .editor import CodexBackend, resolve
from .lifecycle import CodexLifecycle
from .migration import CodexMigrationTarget
from .models import discover, fallback
from .probe import CodexVerifier
from .reader import read
from .scanner import agent_fingerprint, fingerprint, scan

MANIFEST = ToolManifest(id="codex", **AGENTS["codex"])


class CodexBrowser:
    """Codex 扫描与读取实现，不复用跨 Agent 的函数适配器。"""

    def scan(self, cache):
        return scan(cache)

    def read(self, ref):
        return read(ref)

    def read_agent(self, ref):
        return read(ref)

    def resolve_ref(self, ref):
        return str(resolve(ref))

    def fingerprint(self, ref):
        return fingerprint(ref)

    def agent_fingerprint(self, ref):
        return agent_fingerprint(ref)

    def canonicalize(self, row):
        return jsonl_reference(row, MANIFEST.source_path, self.resolve_ref)


class CodexModels:
    def discover(self):
        return discover()

    def fallback(self):
        return fallback()


def build() -> ToolPlugin:
    browser = CodexBrowser()
    lifecycle = CodexLifecycle()
    lifecycle.executable = MANIFEST.executables[0]
    return ToolPlugin(
        manifest=MANIFEST,
        browser=browser,
        migration_source=TreeMigrationSource(browser),
        migration_target=CodexMigrationTarget(),
        editor=CodexBackend(),
        verifier=CodexVerifier(),
        lifecycle=lifecycle,
        models=CodexModels(),
    )
