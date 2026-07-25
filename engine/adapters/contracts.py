"""内置 Adapter 的静态契约。

Ferry 只装配 Claude、Codex 与 OpenCode 三个完整 Adapter。所有已注册
Adapter 都必须具备相同的能力接口；各原生格式支持的内容编辑操作由静态
manifest 精确声明，并在装配时与 editor 实现校验一致。
"""
from __future__ import annotations

import os
from dataclasses import dataclass
from functools import lru_cache
from typing import Protocol, runtime_checkable

@dataclass(frozen=True)
class AgentManifest:
    """Agent 行为的单一事实源，可序列化下发给前端与 Rust。"""

    id: str
    display_name: str
    icon: str
    source_path: str
    edit_operations: tuple[str, ...]
    executables: tuple[str, ...] = ()   # launch descriptor 可执行文件白名单
    fallback_bin_dirs: tuple[str, ...] = ()

    def to_dict(self) -> dict:
        return {"id": self.id, "display_name": self.display_name,
                "icon": self.icon, "source_path": self.source_path,
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
    browser: SessionBrowser
    migration_source: MigrationSource
    migration_target: MigrationTarget
    editor: SessionEditor
    verifier: SessionVerifier
    lifecycle: SessionLifecycle
    models: ModelCatalog

    def __post_init__(self):
        for name in (
            "browser", "migration_source", "migration_target", "editor",
            "verifier", "lifecycle", "models",
        ):
            if getattr(self, name) is None:
                raise ValueError(f"内置 Adapter 缺少必填能力: {self.manifest.id}.{name}")
        if tuple(self.editor.operations) != self.manifest.edit_operations:
            raise ValueError(
                f"Adapter 编辑操作契约不一致: {self.manifest.id} "
                f"manifest={self.manifest.edit_operations!r}, "
                f"editor={tuple(self.editor.operations)!r}"
            )

    @property
    def id(self) -> str:
        return self.manifest.id
