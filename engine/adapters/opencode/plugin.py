"""OpenCode 插件装配：manifest + 各能力实现。"""
from __future__ import annotations

from ..base.migration import TreeMigrationSource
from ..base.plugin import ToolManifest, ToolPlugin
from .authoring import OpenCodeAuthoringCompiler
from .editor import OpenCodeBackend
from .lifecycle import OpenCodeLifecycle
from .migration import OpenCodeMigrationTarget
from .models import discover, fallback
from .probe import OpenCodeVerifier
from .scanner import scan
from .session import read

MANIFEST = ToolManifest(
    id="opencode",
    display_name="OpenCode",
    icon="opencode",
    source_path="~/.local/share/opencode",
    reference_kind="id",
    executables=("opencode",),
    fallback_bin_dirs=("~/.opencode/bin",),
)


class OpenCodeBrowser:
    def scan(self, cache):
        return scan(cache)

    def read(self, ref):
        return read(ref)

    def resolve_ref(self, ref):
        # OpenCode 的 ref 就是 session id，无文件路径可解析。
        return ref


class OpenCodeModels:
    def discover(self):
        return discover()

    def fallback(self):
        return fallback()


def build() -> ToolPlugin:
    from ...infrastructure import executables

    executables.register_fallback_dirs(
        MANIFEST.executables, MANIFEST.fallback_bin_dirs)
    lifecycle = OpenCodeLifecycle()
    lifecycle.executable = MANIFEST.executables[0]
    browser = OpenCodeBrowser()
    return ToolPlugin(
        manifest=MANIFEST,
        browser=browser,
        migration_source=TreeMigrationSource(browser),
        migration_target=OpenCodeMigrationTarget(),
        editor=OpenCodeBackend(),
        authoring=OpenCodeAuthoringCompiler(),
        verifier=OpenCodeVerifier(),
        lifecycle=lifecycle,
        models=OpenCodeModels(),
    )
