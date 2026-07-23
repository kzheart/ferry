"""Reusable assembly primitives for self-contained agent adapters."""
from __future__ import annotations

from collections.abc import Callable

from ...infrastructure import executables
from .migration import TreeMigrationSource
from .plugin import ToolManifest, ToolPlugin


class BrowserAdapter:
    """Turn three adapter functions into the SessionBrowser contract."""

    def __init__(self, scan: Callable, read: Callable, resolve_ref: Callable,
                 fingerprint: Callable | None = None,
                 agent_fingerprint: Callable | None = None,
                 agent_read: Callable | None = None):
        self._scan = scan
        self._read = read
        self._resolve_ref = resolve_ref
        self._fingerprint = fingerprint
        self._agent_fingerprint = agent_fingerprint
        self._agent_read = agent_read

    def scan(self, cache):
        return self._scan(cache)

    def read(self, ref):
        return self._read(ref)

    def read_agent(self, ref):
        return (self._agent_read or self._read)(ref)

    def resolve_ref(self, ref):
        return self._resolve_ref(ref)

    def fingerprint(self, ref):
        return self._fingerprint(ref) if self._fingerprint else None

    def agent_fingerprint(self, ref):
        """Agent 索引用轻量修订标记，避免搜索时深度遍历整个历史库。"""
        if self._agent_fingerprint:
            return self._agent_fingerprint(ref)
        return self.fingerprint(ref)


class ModelCatalogAdapter:
    """Expose adapter model discovery through the shared ModelCatalog protocol."""

    def __init__(self, discover: Callable, fallback: Callable):
        self._discover = discover
        self._fallback = fallback

    def discover(self):
        return self._discover()

    def fallback(self):
        return self._fallback()


def build_plugin(manifest: ToolManifest, browser: BrowserAdapter, *,
                 migration_target=None, editor=None,
                 verifier=None, lifecycle=None, models=None) -> ToolPlugin:
    """Assemble a plugin without duplicating lifecycle and binary wiring."""
    executables.register_fallback_dirs(
        manifest.executables, manifest.fallback_bin_dirs)
    if lifecycle is not None and manifest.executables:
        lifecycle.executable = manifest.executables[0]
    return ToolPlugin(
        manifest=manifest,
        browser=browser,
        migration_source=TreeMigrationSource(browser),
        migration_target=migration_target,
        editor=editor,
        verifier=verifier,
        lifecycle=lifecycle,
        models=models,
    )
