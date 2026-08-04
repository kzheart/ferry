"""统一操作计划生成。"""
from __future__ import annotations

from collections.abc import Callable

from ..contracts.operations import OPERATION_KINDS
from ..context import EngineContext
from ..errors import (
    AgentReferenceError,
    AgentRequestError,
    ConcurrentModificationError,
    require_agent_capability,
)
from ..sessions import agent_read
from ..sessions.index import AgentSessionIndex
from ..sessions.safety import record_session_id, truncate_text
from .metadata_store import metadata_key
from . import metadata
from .edit import EditOperationHandler
from .migrate import MigrationService
from .validation import (
    validate_delete_input,
    validate_edit_input,
    validate_metadata_input,
    validate_migration_input,
)


class OperationPlanner:
    def __init__(
        self,
        ports: EngineContext,
        index: AgentSessionIndex,
        migration: MigrationService,
        edit: EditOperationHandler,
        store_plan: Callable[..., dict],
        database: Callable,
    ):
        self._ports = ports
        self._index = index
        self._migration = migration
        self._edit = edit
        self._store_plan = store_plan
        self._database = database

    def plan(self, value: dict) -> dict:
        if not isinstance(value, dict):
            raise AgentRequestError("operation input 必须是 object")
        kind = value.get("kind")
        if kind not in OPERATION_KINDS:
            raise AgentRequestError("operation kind 非法", {"kind": kind})
        handlers = {
            "edit": self._plan_edit,
            "migration": self._plan_migration,
            "metadata": self._plan_metadata,
            "delete": self._plan_delete,
        }
        handler = handlers.get(kind)
        if handler is None:
            raise AssertionError("Operation contract kind 未绑定处理器")
        return handler(value)

    def _plan_edit(self, value: dict) -> dict:
        operation_input = validate_edit_input(value)
        tool = operation_input["tool"]
        ref = operation_input["ref"]
        require_agent_capability(
            self._ports.adapter(tool), "edit", "editor",
        )
        if operation_input["probe"]:
            require_agent_capability(
                self._ports.adapter(tool), "probe", "verifier",
            )
        before = self._index.resolve(tool, ref)
        preview = self._edit.preview(before, operation_input["ops"])
        after = self._index.resolve(tool, ref)
        if before.revision != after.revision:
            raise ConcurrentModificationError(
                "会话在生成操作计划时已变化，请重新计划"
            )
        self._edit.ensure_supported(after, operation_input["ops"])
        return self._store_plan(
            operation_input,
            preview,
            base_revision=after.revision,
            document_revision=str(preview["revision"]),
        )

    def _plan_migration(self, value: dict) -> dict:
        operation_input = validate_migration_input(
            value, self._ports.adapters(),
        )
        source_tool = operation_input["source_tool"]
        target_tool = operation_input["target_tool"]
        require_agent_capability(
            self._ports.adapter(source_tool),
            "migration-source",
            "migration_source",
        )
        require_agent_capability(
            self._ports.adapter(target_tool),
            "migration-target",
            "migration_target",
        )
        require_agent_capability(
            self._ports.adapter(target_tool), "browse", "browser",
        )
        require_agent_capability(
            self._ports.adapter(target_tool), "resume", "lifecycle",
        )
        if operation_input.get("probe"):
            require_agent_capability(
                self._ports.adapter(target_tool), "probe", "verifier",
            )
        ref = operation_input["ref"]
        before = self._index.resolve(source_tool, ref)
        session = agent_read.read_indexed_session(self._index, before)
        preview = self._migration.preview(
            source_tool,
            operation_input["target_tool"],
            before.canonical_ref,
            max_turn=operation_input.get("max_turn"),
            probe_model=operation_input.get("probe_model"),
            session=session,
        )
        try:
            after = self._index.resolve(source_tool, ref)
        except AgentReferenceError as error:
            raise ConcurrentModificationError(
                "会话在生成操作计划时已变化，请重新计划"
            ) from error
        if before.revision != after.revision:
            raise ConcurrentModificationError(
                "会话在生成操作计划时已变化，请重新计划"
            )
        return self._store_plan(
            operation_input,
            preview,
            base_revision=after.revision,
            document_revision=None,
        )

    def _plan_metadata(self, value: dict) -> dict:
        operation_input = validate_metadata_input(value)
        tool = operation_input["tool"]
        ref = operation_input["ref"]
        before = self._index.resolve(tool, ref)
        session_id = before.row.get("id")
        if not isinstance(session_id, str) or not session_id:
            raise AgentRequestError("会话缺少可用的 metadata id")
        metadata_before = metadata.list_all(self._ports).get(
            metadata_key(tool, session_id), {},
        )
        operation_input["session_id"] = session_id
        operation_input["metadata_before"] = metadata_before
        preview = {
            "tool": tool,
            "ref": ref,
            "before": metadata_before,
            "after_patch": operation_input["patch"],
        }
        after = self._index.resolve(tool, ref)
        if before.revision != after.revision:
            raise ConcurrentModificationError(
                "会话在生成操作计划时已变化，请重新计划"
            )
        return self._store_plan(
            operation_input,
            preview,
            base_revision=after.revision,
            document_revision=None,
        )

    def _plan_delete(self, value: dict) -> dict:
        operation_input = validate_delete_input(
            value, self._ports.adapters(),
        )
        tool = operation_input["tool"]
        require_agent_capability(
            self._ports.adapter(tool), "delete", "lifecycle",
        )
        metadata_rows = metadata.list_all(self._ports)
        sessions = []
        excluded = []
        targets = []
        total_size = 0
        for ref in operation_input["refs"]:
            record = self._index.resolve(tool, ref)
            session_id = record.row.get("id")
            if not isinstance(session_id, str) or not session_id:
                raise AgentRequestError("会话缺少可用的原生 ID")
            title = truncate_text(str(record.row.get("title") or ""), 512)[0]
            metadata_row = metadata_rows.get(
                metadata_key(tool, session_id), {},
            )
            if metadata_row.get("pinned") is True:
                cause = "pinned"
            elif metadata_row.get("archived") is True:
                cause = "archived"
            elif metadata_row.get("tags"):
                cause = "tagged"
            else:
                # targets 只收未被排除的会话:它就是 apply 的删除名单,被
                # 保护规则剔除的会话一旦混进来,预览说"已保护"而执行照删。
                targets.append({
                    "ref": record.opaque_ref,
                    "session_id": session_id,
                    "revision": record.revision,
                })
                size = int(record.row.get("size") or 0)
                total_size += size
                sessions.append({
                    "tool": tool,
                    "ref": record.opaque_ref,
                    "session_id": record_session_id(record),
                    "title": title,
                    "project": record.row.get("dir"),
                    "size": size,
                    "updated": record.row.get("updated"),
                })
                continue
            excluded.append({
                "tool": tool,
                "ref": record.opaque_ref,
                "title": title,
                "cause": cause,
            })
        operation_input["targets"] = targets
        preview = {
            "tool": tool,
            "sessions": sessions,
            "excluded": excluded,
            "totals": {"count": len(sessions), "size_bytes": total_size},
            # 删除不可恢复:预览必须明说,审批卡靠它渲染永久删除警示
            "permanent": True,
        }
        return self._store_plan(
            operation_input,
            preview,
            base_revision="batch",
            document_revision=None,
        )
