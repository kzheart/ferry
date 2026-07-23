"""Codex 插件装配：manifest + 各能力实现。"""
from __future__ import annotations

from ..base.builder import BrowserAdapter, ModelCatalogAdapter, build_plugin
from ..base.plugin import ToolManifest, ToolPlugin
from .authoring import CodexAuthoringCompiler
from .editor import CodexBackend, resolve
from .formats import FORMATS
from .lifecycle import CodexLifecycle
from .migration import CodexMigrationTarget
from .models import discover, fallback
from .probe import CodexVerifier
from .reader import read
from .scanner import agent_fingerprint, fingerprint, scan

MANIFEST = ToolManifest(
    id="codex",
    display_name="Codex CLI",
    icon="codex",
    source_path="~/.codex/sessions",
    reference_kind="path",
    executables=("codex",),
)


def build() -> ToolPlugin:
    return build_plugin(
        MANIFEST,
        BrowserAdapter(scan, read, lambda ref: str(resolve(ref)),
                       fingerprint=fingerprint, agent_fingerprint=agent_fingerprint),
        migration_target=CodexMigrationTarget(),
        editor=CodexBackend(),
        authoring=CodexAuthoringCompiler(),
        verifier=CodexVerifier(),
        lifecycle=CodexLifecycle(),
        models=ModelCatalogAdapter(discover, fallback),
        formats=FORMATS,
    )
