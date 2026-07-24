"""Claude 当前原生结构的静态 Adapter 装配。"""
from __future__ import annotations

from ..contracts import AgentManifest, AgentAdapter, jsonl_reference
from ..base.migration import TreeMigrationSource
from ...contracts.agents import AGENTS
from . import editing as claude_edit
from .editor import ClaudeBackend
from .lifecycle import ClaudeLifecycle
from .migration import ClaudeMigrationTarget
from .models import discover, fallback
from .probe import ClaudeVerifier
from .reader import read
from .scanner import agent_fingerprint, fingerprint, scan

MANIFEST = AgentManifest(id="claude", **AGENTS["claude"])


class ClaudeBrowser:
    """Claude 扫描与读取实现，不复用跨 Agent 的函数适配器。"""

    def scan(self, cache):
        return scan(cache)

    def read(self, ref):
        return read(ref)

    def read_agent(self, ref):
        return read(ref)

    def resolve_ref(self, ref):
        return str(claude_edit.resolve(ref))

    def fingerprint(self, ref):
        return fingerprint(ref)

    def agent_fingerprint(self, ref):
        return agent_fingerprint(ref)

    def canonicalize(self, row):
        return jsonl_reference(row, MANIFEST.source_path, self.resolve_ref)


class ClaudeModels:
    def discover(self):
        return discover()

    def fallback(self):
        return fallback()


def build() -> AgentAdapter:
    browser = ClaudeBrowser()
    lifecycle = ClaudeLifecycle()
    lifecycle.executable = MANIFEST.executables[0]
    return AgentAdapter(
        manifest=MANIFEST,
        browser=browser,
        migration_source=TreeMigrationSource(browser),
        migration_target=ClaudeMigrationTarget(),
        editor=ClaudeBackend(),
        verifier=ClaudeVerifier(),
        lifecycle=lifecycle,
        models=ClaudeModels(),
    )
