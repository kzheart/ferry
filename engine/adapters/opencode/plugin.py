"""OpenCode 插件装配：manifest + 各能力实现。"""
from __future__ import annotations

from ..base.builder import BrowserAdapter, ModelCatalogAdapter, build_plugin
from ..base.plugin import ToolManifest, ToolPlugin
from .authoring import OpenCodeAuthoringCompiler
from .editor import OpenCodeBackend
from .lifecycle import OpenCodeLifecycle
from .migration import OpenCodeMigrationTarget
from .models import discover, fallback
from .probe import OpenCodeVerifier
from .scanner import fingerprint, scan
from .session import read, read_preview

MANIFEST = ToolManifest(
    id="opencode",
    display_name="OpenCode",
    icon="opencode",
    source_path="~/.local/share/opencode",
    reference_kind="id",
    executables=("opencode",),
    fallback_bin_dirs=("~/.opencode/bin",),
)


def build() -> ToolPlugin:
    return build_plugin(
        MANIFEST,
        BrowserAdapter(scan, read, lambda ref: ref, fingerprint=fingerprint,
                       agent_read=read_preview),
        migration_target=OpenCodeMigrationTarget(),
        editor=OpenCodeBackend(),
        authoring=OpenCodeAuthoringCompiler(),
        verifier=OpenCodeVerifier(),
        lifecycle=OpenCodeLifecycle(),
        models=ModelCatalogAdapter(discover, fallback),
    )
