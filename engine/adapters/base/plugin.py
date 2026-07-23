"""ToolPlugin 插件契约：manifest + 可选能力。

application 层只依赖本模块声明的 Protocol；缺少某能力时对应字段为
None，capabilities 查询返回 unsupported，而不是伪造实现。
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol, runtime_checkable

from ...domain.errors import CapabilityUnsupportedError

# 能力等级标识（manifest.capabilities 使用）
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
    migration_source: MigrationSource | None = None
    migration_target: MigrationTarget | None = None
    editor: SessionEditor | None = None
    verifier: SessionVerifier | None = None
    lifecycle: SessionLifecycle | None = None
    models: ModelCatalog | None = None

    @property
    def id(self) -> str:
        return self.manifest.id

    def capabilities(self) -> list[str]:
        caps = [CAP_BROWSE]
        if self.migration_source is not None:
            caps.append(CAP_MIGRATE_SOURCE)
        if self.migration_target is not None:
            caps.append(CAP_MIGRATE_TARGET)
        if self.editor is not None:
            caps.append(CAP_EDIT)
        if self.editor is not None and self.editor.capabilities().get("inplace"):
            caps.append(CAP_INPLACE)
        if self.verifier is not None:
            caps.append(CAP_VERIFIED)
        return caps

    def describe(self) -> dict:
        return self.manifest.to_dict(self.capabilities())

    def require(self, capability: str):
        """取用可选能力；缺失时抛出 unsupported 领域异常。"""
        value = getattr(self, capability.replace("-", "_"), None)
        if value is None:
            raise CapabilityUnsupportedError(self.id, capability)
        return value
