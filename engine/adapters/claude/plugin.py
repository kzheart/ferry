"""Claude 插件装配：manifest + 各能力实现。"""
from __future__ import annotations

from ..base.builder import BrowserAdapter, ModelCatalogAdapter, build_plugin
from ..base.plugin import ToolManifest, ToolPlugin
from . import editing as claude_edit
from .editor import ClaudeBackend
from .lifecycle import ClaudeLifecycle
from .migration import ClaudeMigrationTarget
from .models import discover, fallback
from .probe import ClaudeVerifier
from .reader import read
from .scanner import agent_fingerprint, fingerprint, scan

MANIFEST = ToolManifest(
    id="claude",
    display_name="Claude Code",
    icon="claude",
    source_path="~/.claude/projects",
    reference_kind="path",
    executables=("claude",),
)


def build() -> ToolPlugin:
    return build_plugin(
        MANIFEST,
        BrowserAdapter(scan, read, lambda ref: str(claude_edit.resolve(ref)),
                       fingerprint=fingerprint, agent_fingerprint=agent_fingerprint),
        migration_target=ClaudeMigrationTarget(),
        editor=ClaudeBackend(),
        verifier=ClaudeVerifier(),
        lifecycle=ClaudeLifecycle(),
        models=ModelCatalogAdapter(discover, fallback),
    )
