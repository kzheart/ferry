"""已审批操作的原生写入执行。"""
from __future__ import annotations

from collections.abc import Callable

from ..context import EngineContext
from ..errors import (
    AgentReferenceError,
    AgentRequestError,
    ConcurrentModificationError,
    require_agent_capability,
)
from ..sessions import agent_read
from ..sessions.index import AgentSessionIndex
from . import metadata, verification as probe_mod
from .delete import SessionDeletionService
from .metadata_store import metadata_key
from .edit import EditOperationHandler
from .migrate import MigrationService
from .plan_store import OperationPlan


class OperationExecutor:
    def __init__(
        self,
        ports: EngineContext,
        index: AgentSessionIndex,
        migration: MigrationService,
        edit: EditOperationHandler,
        database: Callable,
    ):
        self._ports = ports
        self._index = index
        self._migration = migration
        self._edit = edit
        self._database = database

    def execute(self, operation: OperationPlan) -> dict:
        handlers = {
            "edit": self._apply_edit,
            "migration": self._apply_migration,
            "metadata": self._apply_metadata,
            "delete": self._apply_delete,
        }
        handler = handlers.get(operation.kind)
        if handler is None:
            raise AgentRequestError(
                "operation kind 非法", {"kind": operation.kind},
            )
        return handler(operation)

    def _apply_edit(self, operation: OperationPlan) -> dict:
        params = operation.input()
        if params["probe"]:
            require_agent_capability(
                self._ports.adapter(params["tool"]), "probe", "verifier",
            )
        return self._edit.apply(operation, self._finish_mutation)

    def _finish_mutation(self, tool, editor, result, document, snapshot, probe):
        if not probe:
            return result
        try:
            verifier = require_agent_capability(
                self._ports.adapter(tool), "probe", "verifier",
            )
            report = verifier.probe_edited(
                editor, document, result,
            )
        except probe_mod.ProbeTimeout as error:
            report = probe_mod.timeout_report(tool, error)
        result["probe"] = report
        if report["status"] == "passed":
            return result
        if snapshot:
            editor.restore_snapshot(snapshot, document)
            result.update(ok=False, error="隔离探针未通过,已自动还原快照")
        return result

    def _apply_migration(self, operation: OperationPlan) -> dict:
        params = operation.input()
        try:
            record = self._index.resolve(
                params["source_tool"], params["ref"],
            )
        except AgentReferenceError as error:
            raise ConcurrentModificationError(
                "会话在迁移计划生成后已变化，请重新计划"
            ) from error
        if record.revision != operation.base_revision:
            raise ConcurrentModificationError(
                "会话在迁移计划生成后已变化，请重新计划"
            )
        session = agent_read.read_indexed_session(self._index, record)
        result = self._migration.apply(
            params["source_tool"],
            params["target_tool"],
            record.canonical_ref,
            probe=params["probe"],
            max_turn=params.get("max_turn"),
            probe_model=params.get("probe_model"),
            session=session,
        )
        structure = result.get("validation", {}).get("structure", {})
        if result.get("rolled_back") or structure.get("ok") is not True:
            raise RuntimeError("迁移写入后的结构校验失败，产物已回滚")
        return result

    def _apply_metadata(self, operation: OperationPlan) -> dict:
        params = operation.input()
        try:
            record = self._index.resolve(params["tool"], params["ref"])
        except AgentReferenceError as error:
            raise ConcurrentModificationError(
                "会话在元数据计划生成后已变化，请重新计划"
            ) from error
        if record.revision != operation.base_revision:
            raise ConcurrentModificationError(
                "会话在元数据计划生成后已变化，请重新计划"
            )
        if record.row.get("id") != params["session_id"]:
            raise ConcurrentModificationError(
                "会话标识在元数据计划生成后已变化，请重新计划"
            )
        result = metadata.compare_and_set_entry(
            params["tool"],
            params["session_id"],
            params["metadata_before"],
            params["patch"],
            self._ports,
        )
        return {"metadata": result}

    @staticmethod
    def _protected_cause(metadata_row: dict) -> str | None:
        if metadata_row.get("pinned") is True:
            return "pinned"
        if metadata_row.get("archived") is True:
            return "archived"
        if metadata_row.get("tags"):
            return "tagged"
        return None

    def _apply_delete(self, operation: OperationPlan) -> dict:
        params = operation.input()
        tool = params["tool"]
        # 计划期的保护审查挡不住批准前才被 pin/archive/打标签的会话:元数据
        # 与会话内容 revision 无关,逐条 revision 比对不可能发现这种变化。
        metadata_rows = metadata.list_all(self._ports)
        deletion = SessionDeletionService(self._ports)
        result = {"succeeded": [], "skipped": [], "failed": []}
        for target in params["targets"]:
            ref = target["ref"]
            try:
                protected = self._protected_cause(
                    metadata_rows.get(
                        metadata_key(tool, target["session_id"]), {},
                    ),
                )
                if protected is not None:
                    result["skipped"].append({
                        "tool": tool,
                        "ref": ref,
                        "cause": "protected",
                        "protection": protected,
                    })
                    continue
                try:
                    record = self._index.resolve(tool, ref)
                except AgentReferenceError as error:
                    reason = error.params.get("reason")
                    result["skipped"].append({
                        "tool": tool,
                        "ref": ref,
                        "cause": "changed" if reason == "session_changed" else "not_found",
                    })
                    continue
                if (
                    record.row.get("id") != target["session_id"]
                    or record.revision != target["revision"]
                ):
                    result["skipped"].append({
                        "tool": tool,
                        "ref": ref,
                        "cause": "changed",
                    })
                    continue
                deletion.delete(tool, record.canonical_ref)
                self._index.evict(tool, record.canonical_ref)
                result["succeeded"].append({"tool": tool, "ref": ref})
            except Exception as error:  # noqa: BLE001 - 单条失败继续批处理
                result["failed"].append({
                    "tool": tool,
                    "ref": ref,
                    "error": str(error)[:500],
                })
        return result
