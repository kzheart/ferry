"""内置 Adapter 的静态能力契约。"""
from __future__ import annotations

import os
from dataclasses import dataclass
from functools import lru_cache
from typing import Protocol, runtime_checkable

from ..contracts.agents import AGENT_CAPABILITIES


_COMPONENT_CAPABILITIES = {
    "browser": ("browse",),
    "migration_source": ("migration-source",),
    "migration_target": ("migration-target",),
    "editor": ("edit",),
    "verifier": ("probe",),
    "lifecycle": ("resume", "delete"),
    "models": ("models",),
}


@dataclass(frozen=True)
class AgentManifest:
    """Agent 行为的单一事实源，可序列化下发给前端与 Rust。"""

    id: str
    display_name: str
    icon: str
    source_path: str
    capabilities: tuple[str, ...]
    edit_operations: tuple[str, ...]
    executables: tuple[str, ...] = ()   # launch descriptor 可执行文件白名单
    fallback_bin_dirs: tuple[str, ...] = ()

    def to_dict(self) -> dict:
        return {"id": self.id, "display_name": self.display_name,
                "icon": self.icon, "source_path": self.source_path,
                "capabilities": list(self.capabilities),
                "edit_operations": list(self.edit_operations),
                "executables": list(self.executables),
                "fallback_bin_dirs": list(self.fallback_bin_dirs)}


@dataclass(frozen=True)
class NativeSessionReference:
    """Adapter 内部的原生引用；不会离开 Python Engine。"""

    canonical_ref: str
    root: str | None
    path_backed: bool


@lru_cache(maxsize=32)
def _resolved_root(source_path: str) -> str | None:
    """扫描根只有三个固定目录,逐个会话重解析一次在数千行时是纯浪费。"""
    try:
        return os.path.realpath(os.path.expanduser(source_path), strict=True)
    except OSError:
        return None


def _is_within(path: str, root: str) -> bool:
    if path == root:
        return True
    prefix = root if root.endswith(os.sep) else root + os.sep
    return path.startswith(prefix)


def jsonl_reference(row: dict, source_path: str, resolve_ref) -> NativeSessionReference | None:
    """校验 JSONL 会话路径并收窄为 Adapter 可接受的内部引用。"""
    raw = row.get("path")
    if not isinstance(raw, str) or not raw:
        return None
    root = _resolved_root(source_path)
    if root is None:
        return None
    try:
        path = os.path.realpath(os.path.expanduser(raw), strict=True)
    except OSError:
        return None
    if (
        not path.endswith(".jsonl")
        or not os.path.isfile(path)
        or not _is_within(path, root)
    ):
        return None
    try:
        resolved = os.path.realpath(resolve_ref(path), strict=True)
    except (OSError, ValueError):
        return None
    if resolved != path:
        return None
    return NativeSessionReference(path, root, True)


def id_reference(row: dict) -> NativeSessionReference | None:
    """校验由 Adapter 管理的原生 ID。"""
    raw = row.get("id")
    if not isinstance(raw, str) or not raw or len(raw) > 512 or "\x00" in raw:
        return None
    return NativeSessionReference(raw, None, False)


@runtime_checkable
class SessionBrowser(Protocol):
    """读侧最小能力：扫描、读取、引用解析。"""

    def scan(self, cache) -> list[dict]: ...

    def read(self, ref: str): ...

    def read_agent(self, ref: str): ...

    def resolve_ref(self, ref: str) -> str: ...

    def fingerprint(self, ref: str): ...

    def agent_fingerprint(self, ref: str): ...

    def canonicalize(self, row: dict) -> NativeSessionReference | None: ...

    def validate_read_scope(self, ref: NativeSessionReference) -> None: ...


@runtime_checkable
class MigrationSource(Protocol):
    def export_tree(self, ref: str): ...


@runtime_checkable
class MigrationTarget(Protocol):
    def plan(self, session) -> dict: ...

    def preview(self, session, cwd: str | None = None) -> dict: ...

    def write(self, session, cwd: str): ...

    def classify_tool_call(self, tool_call) -> str: ...


@runtime_checkable
class SessionEditor(Protocol):
    name: str
    operations: tuple[str, ...]

    def load(self, ref: str): ...

    def apply_ops(self, doc, ops: list[dict]) -> list: ...

    def replace_reply(self, doc, turn, reply) -> list: ...

    def validate(self, doc) -> None: ...

    def stats(self, doc) -> dict: ...

    def commit(self, doc) -> dict: ...

    def snapshot(self, doc, reason_code=None, extra: dict | None = None): ...

    def restore_snapshot(self, snapshot, doc) -> None: ...

    def saved_revision(self, result: dict, doc) -> str: ...


@runtime_checkable
class SessionVerifier(Protocol):
    def probe(self, session_id: str, cwd, model=None): ...

    def probe_edited(self, editor, doc, result: dict, model=None): ...


@runtime_checkable
class ModelCatalog(Protocol):
    def discover(self): ...

    def fallback(self) -> list[dict]: ...


@runtime_checkable
class SessionLifecycle(Protocol):
    """会话生命周期策略：resume/清理/校验引用/删除与恢复。"""

    delete_undoable: bool

    def resume_descriptor(self, session_id: str, cwd: str) -> dict: ...

    def cleanup(self, session_id: str, dest) -> None: ...

    def validation_ref(self, session_id: str, dest) -> str: ...

    def probe_cwd(self, cwd): ...

    def delete(self, adapter: "AgentAdapter", ref: str) -> dict: ...

    def restore_delete(self, snapshot, meta: dict) -> dict: ...


@dataclass(frozen=True)
class AgentAdapter:
    manifest: AgentManifest
    browser: SessionBrowser | None = None
    migration_source: MigrationSource | None = None
    migration_target: MigrationTarget | None = None
    editor: SessionEditor | None = None
    verifier: SessionVerifier | None = None
    lifecycle: SessionLifecycle | None = None
    models: ModelCatalog | None = None

    def __post_init__(self):
        capabilities = self.manifest.capabilities
        if (
            len(capabilities) != len(set(capabilities))
            or any(capability not in AGENT_CAPABILITIES for capability in capabilities)
            or capabilities != tuple(
                capability
                for capability in AGENT_CAPABILITIES
                if capability in capabilities
            )
        ):
            raise ValueError(
                f"Adapter capability 契约无效: {self.manifest.id}"
            )
        for component, required_capabilities in _COMPONENT_CAPABILITIES.items():
            expected = any(
                capability in capabilities
                for capability in required_capabilities
            )
            if (getattr(self, component) is not None) != expected:
                raise ValueError(
                    f"Adapter capability/component 不一致: "
                    f"{self.manifest.id}.{component}"
                )
        if "edit" not in capabilities and self.manifest.edit_operations:
            raise ValueError(
                f"Adapter 未声明 edit 但包含编辑操作: {self.manifest.id}"
            )
        if "edit" in capabilities and not self.manifest.edit_operations:
            raise ValueError(
                f"Adapter 声明 edit 但未包含编辑操作: {self.manifest.id}"
            )
        if self.editor is not None and (
            tuple(self.editor.operations) != self.manifest.edit_operations
        ):
            raise ValueError(
                f"Adapter 编辑操作契约不一致: {self.manifest.id} "
                f"manifest={self.manifest.edit_operations!r}, "
                f"editor={tuple(self.editor.operations)!r}"
            )

    @property
    def id(self) -> str:
        return self.manifest.id

    def supports(self, capability: str) -> bool:
        return capability in self.manifest.capabilities

    def require(self, capability: str, component: str):
        if capability not in AGENT_CAPABILITIES:
            raise ValueError(f"未知 Agent capability: {capability}")
        if component not in _COMPONENT_CAPABILITIES:
            raise ValueError(f"未知 Agent component: {component}")
        if capability not in _COMPONENT_CAPABILITIES[component]:
            raise ValueError(
                f"Agent capability/component 映射无效: "
                f"{capability}.{component}"
            )
        value = getattr(self, component)
        if not self.supports(capability) or value is None:
            raise ValueError(
                f"Agent 不支持能力: {self.manifest.id}.{capability}"
            )
        return value
