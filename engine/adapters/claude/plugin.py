"""Claude 插件装配：manifest + 各能力实现。"""
from __future__ import annotations

from ..base.migration import TreeMigrationSource
from ..base.plugin import ToolManifest, ToolPlugin
from . import editing as claude_edit
from .authoring import ClaudeAuthoringCompiler
from .editor import ClaudeBackend
from .lifecycle import ClaudeLifecycle
from .migration import ClaudeMigrationTarget
from .models import discover, fallback
from .probe import ClaudeVerifier
from .reader import read
from .scanner import scan

MANIFEST = ToolManifest(
    id="claude",
    display_name="Claude Code",
    icon="claude",
    source_path="~/.claude/projects",
    reference_kind="path",
    executables=("claude",),
)


class ClaudeBrowser:
    def scan(self, cache):
        return scan(cache)

    def read(self, ref):
        return read(ref)

    def resolve_ref(self, ref):
        return str(claude_edit.resolve(ref))


class ClaudeModels:
    def discover(self):
        return discover()

    def fallback(self):
        return fallback()


def build() -> ToolPlugin:
    lifecycle = ClaudeLifecycle()
    lifecycle.executable = MANIFEST.executables[0]
    browser = ClaudeBrowser()
    return ToolPlugin(
        manifest=MANIFEST,
        browser=browser,
        migration_source=TreeMigrationSource(browser),
        migration_target=ClaudeMigrationTarget(),
        editor=ClaudeBackend(),
        authoring=ClaudeAuthoringCompiler(),
        verifier=ClaudeVerifier(),
        lifecycle=lifecycle,
        models=ClaudeModels(),
    )
