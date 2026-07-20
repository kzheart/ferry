"""Codex 插件装配：manifest + 各能力实现。"""
from __future__ import annotations

from ..base.migration import TreeMigrationSource
from ..base.plugin import ToolManifest, ToolPlugin
from .authoring import CodexAuthoringCompiler
from .editor import CodexBackend, resolve
from .lifecycle import CodexLifecycle
from .migration import CodexMigrationTarget
from .models import discover, fallback
from .probe import CodexVerifier
from .reader import read
from .scanner import scan

MANIFEST = ToolManifest(
    id="codex",
    display_name="Codex CLI",
    icon="codex",
    source_path="~/.codex/sessions",
    reference_kind="path",
    executables=("codex",),
)


class CodexBrowser:
    def scan(self, cache):
        return scan(cache)

    def read(self, ref):
        return read(ref)

    def resolve_ref(self, ref):
        return str(resolve(ref))


class CodexModels:
    def discover(self):
        return discover()

    def fallback(self):
        return fallback()


def build() -> ToolPlugin:
    lifecycle = CodexLifecycle()
    lifecycle.executable = MANIFEST.executables[0]
    browser = CodexBrowser()
    return ToolPlugin(
        manifest=MANIFEST,
        browser=browser,
        migration_source=TreeMigrationSource(browser),
        migration_target=CodexMigrationTarget(),
        editor=CodexBackend(),
        authoring=CodexAuthoringCompiler(),
        verifier=CodexVerifier(),
        lifecycle=lifecycle,
        models=CodexModels(),
    )
