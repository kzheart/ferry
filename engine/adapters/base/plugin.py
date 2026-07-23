"""内置 Adapter 的静态契约。

Ferry 只装配 Claude、Codex 与 OpenCode 三个完整 Adapter。所有已注册
Adapter 都必须具备相同的查询、迁移、编辑、校验、生命周期和模型能力；
不能在运行时根据 capability 走另一条业务路径。
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol, runtime_checkable

# 静态能力标识（manifest.capabilities 使用）。它们是内置产品定义，不是
# 可选插件协商协议。
CAP_BROWSE = "browse"
CAP_MIGRATE_SOURCE = "migrate-source"
CAP_MIGRATE_TARGET = "migrate-target"
CAP_EDIT = "edit"
CAP_INPLACE = "inplace"
CAP_VERIFIED = "verified"


@dataclass(frozen=True)
class ToolManifest:
    """Agent 行为的单一事实源，可序列化下发给前端与 Rust。"""

    id: str
    display_name: str
    icon: str
    source_path: str
    reference_kind: str                 # "path" | "id"
    executables: tuple[str, ...] = ()   # launch descriptor 可执行文件白名单
    fallback_bin_dirs: tuple[str, ...] = ()

    def to_dict(self, capabilities: list[str] | None = None) -> dict:
        return {"id": self.id, "display_name": self.display_name,
                "icon": self.icon, "source_path": self.source_path,
                "reference_kind": self.reference_kind,
                "executables": list(self.executables),
                "capabilities": capabilities or []}


@runtime_checkable
class SessionBrowser(Protocol):
    """读侧最小能力：扫描、读取、引用解析。"""

    def scan(self, cache) -> list[dict]: ...

    def read(self, ref: str): ...

    def read_agent(self, ref: str): ...

    def resolve_ref(self, ref: str) -> str: ...

    def fingerprint(self, ref: str): ...

    def agent_fingerprint(self, ref: str): ...


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

    def capabilities(self) -> dict: ...

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

    def delete(self, plugin: "ToolPlugin", ref: str) -> dict: ...

    def restore_delete(self, snapshot, meta: dict) -> dict: ...


@dataclass(frozen=True)
class ToolPlugin:
    manifest: ToolManifest
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

    def capabilities(self) -> list[str]:
        return [
            CAP_BROWSE,
            CAP_MIGRATE_SOURCE,
            CAP_MIGRATE_TARGET,
            CAP_EDIT,
            CAP_INPLACE,
            CAP_VERIFIED,
        ]

    def describe(self) -> dict:
        return self.manifest.to_dict(self.capabilities())
