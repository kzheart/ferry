"""内置 Adapter 的静态契约。

Ferry 只装配 Claude、Codex 与 OpenCode 三个完整 Adapter。所有已注册
Adapter 都必须具备相同的查询、迁移、编辑、校验、生命周期和模型能力；
不能在运行时根据 capability 走另一条业务路径。
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, runtime_checkable

@dataclass(frozen=True)
class AgentManifest:
    """Agent 行为的单一事实源，可序列化下发给前端与 Rust。"""

    id: str
    display_name: str
    icon: str
    source_path: str
    executables: tuple[str, ...] = ()   # launch descriptor 可执行文件白名单
    fallback_bin_dirs: tuple[str, ...] = ()

    def to_dict(self) -> dict:
        return {"id": self.id, "display_name": self.display_name,
                "icon": self.icon, "source_path": self.source_path,
                "executables": list(self.executables),
                "fallback_bin_dirs": list(self.fallback_bin_dirs)}


@dataclass(frozen=True)
class NativeSessionReference:
    """Adapter 内部的原生引用；不会离开 Python Engine。"""

    canonical_ref: str
    root: str | None
    path_backed: bool


def jsonl_reference(row: dict, source_path: str, resolve_ref) -> NativeSessionReference | None:
    """校验 JSONL 会话路径并收窄为 Adapter 可接受的内部引用。"""
    raw = row.get("path")
    if not isinstance(raw, str) or not raw:
        return None
    try:
        root = Path(source_path).expanduser().resolve(strict=True)
        path = Path(raw).expanduser().resolve(strict=True)
    except OSError:
        return None
    if not path.is_file() or path.suffix != ".jsonl" or not path.is_relative_to(root):
        return None
    try:
        resolved = Path(resolve_ref(str(path))).resolve(strict=True)
    except (OSError, ValueError):
        return None
    if resolved != path:
        return None
    return NativeSessionReference(str(path), str(root), True)


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

    @property
    def id(self) -> str:
        return self.manifest.id
