"""OpenCode 当前原生结构的静态 Adapter 装配。"""
from __future__ import annotations

from ..base.plugin import ToolManifest, ToolPlugin
from ..base.migration import TreeMigrationSource
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


class OpenCodeModels:
    def discover(self):
        return discover()

    def fallback(self):
        return fallback()


def build() -> ToolPlugin:
    browser = OpenCodeBrowser()
    lifecycle = OpenCodeLifecycle()
    lifecycle.executable = MANIFEST.executables[0]
    return ToolPlugin(
        manifest=MANIFEST,
        browser=browser,
        migration_source=TreeMigrationSource(browser),
        migration_target=OpenCodeMigrationTarget(),
        editor=OpenCodeBackend(),
        verifier=OpenCodeVerifier(),
        lifecycle=lifecycle,
        models=OpenCodeModels(),
    )
