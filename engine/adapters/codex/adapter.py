"""Codex 当前原生结构的静态 Adapter 装配。"""
from __future__ import annotations

from pathlib import Path

from ..contracts import (
    AgentManifest,
    AgentAdapter,
    NativeSessionReference,
    jsonl_reference,
)
from ..shared.migration import TreeMigrationSource
from ...contracts.agents import AGENTS
from ...errors import AgentReferenceError
from .editor import CodexBackend, resolve
from .lifecycle import CodexLifecycle
from .migration import CodexMigrationTarget
from .models import discover, fallback
from .probe import CodexVerifier
from .reader import read
from .scanner import agent_fingerprint, fingerprint, scan

MANIFEST = AgentManifest(id="codex", **AGENTS["codex"])


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

    def validate_read_scope(self, ref: NativeSessionReference) -> None:
        if not ref.path_backed or not ref.root:
            raise AgentReferenceError("Codex 会话必须使用路径引用")
        try:
            root = Path(ref.root).resolve(strict=True)
            path = Path(ref.canonical_ref).resolve(strict=True)
        except OSError as error:
            raise AgentReferenceError("Codex 会话读取范围包含失效文件") from error
        if (
            not path.is_file()
            or path.suffix != ".jsonl"
            or not path.is_relative_to(root)
        ):
            raise AgentReferenceError("Codex 会话读取范围超出会话根目录")
        for candidate in root.rglob("rollout*.jsonl"):
            try:
                resolved = candidate.resolve(strict=True)
            except OSError as error:
                raise AgentReferenceError(
                    "Codex 会话子树包含失效文件"
                ) from error
            if not resolved.is_file() or not resolved.is_relative_to(root):
                raise AgentReferenceError(
                    "Codex 会话子树超出 Agent 会话根目录"
                )


class CodexModels:
    def discover(self):
        return discover()

    def fallback(self):
        return fallback()


def build() -> AgentAdapter:
    browser = CodexBrowser()
    lifecycle = CodexLifecycle()
    lifecycle.executable = MANIFEST.executables[0]
    return AgentAdapter(
        manifest=MANIFEST,
        browser=browser,
        migration_source=TreeMigrationSource(browser),
        migration_target=CodexMigrationTarget(),
        editor=CodexBackend(),
        verifier=CodexVerifier(),
        lifecycle=lifecycle,
        models=CodexModels(),
    )
