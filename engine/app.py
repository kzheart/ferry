"""Engine 进程能力入口。

RPC/CLI 在进程边界构造此对象；各能力通过显式上下文访问依赖。
"""
from __future__ import annotations

from .context import EngineContext
from .contracts.ipc import FERRY_CONTRACT_HASH
from .errors import AgentReferenceError
from .operations import history, metadata
from .operations.service import OperationService
from .organization import proposals as organizing
from .organization import summaries
from .runtime import sessions as runtime_sessions
from .sessions import catalog as agent_tools
from .sessions import search as session_search
from .sessions import usage as session_usage
from .sessions.index import AgentSessionIndex, IndexedSession
from .sessions import read as sessions
from .sessions import scan as scanning
from .system import environment, models
from .system.pricing import pricing


class EngineService:
    def __init__(self, ports: EngineContext,
                 index: AgentSessionIndex,
                 operations: OperationService):
        self._ports = ports
        self._index = index
        self._operations = operations

    def close(self) -> None:
        self._operations.shutdown()

    def health(self) -> dict:
        return {
            "status": "ready",
            "service": "engine",
            "contract_hash": FERRY_CONTRACT_HASH,
        }

    def version(self) -> dict:
        return {"version": self._ports.version}

    def scan(self) -> dict:
        return scanning.scan(self._ports, self._index)

    def environment(self) -> dict:
        return environment.inspect(self._ports)

    def _resolve_session(self, tool: str, ref: str) -> IndexedSession:
        return self._index.resolve(tool, ref)

    def _checked_query(self, tool: str, ref: str, query):
        record = self._resolve_session(tool, ref)
        result = query(record)
        self._resolve_session(tool, ref)
        return result

    def resume_command(self, tool: str, ref: str) -> dict:
        def build(record: IndexedSession) -> dict:
            session_id = record.row.get("id")
            if not isinstance(session_id, str) or not session_id:
                raise AgentReferenceError("会话缺少原生 ID")
            cwd = record.row.get("dir")
            if not isinstance(cwd, str) or not cwd:
                cwd = "."
            return self._ports.adapter(tool).lifecycle.resume_descriptor(
                session_id, cwd,
            )

        return self._checked_query(tool, ref, build)

    def list_models(self, tool: str) -> dict:
        return models.list_models(tool, self._ports)

    def pricing(self, force: bool = False) -> dict:
        return pricing(force=force)

    def migration_history(self) -> list[dict]:
        return history.list_entries(self._ports)

    def delete_migration_history(self, history_id: str) -> dict:
        return history.delete(history_id, self._ports)

    def show_session(self, tool: str, ref: str) -> dict:
        return self._checked_query(
            tool, ref,
            lambda record: sessions.show(
                tool, record.canonical_ref, self._ports,
            ),
        )

    def session_asset(self, tool: str, ref: str, asset_id: str) -> dict:
        return self._checked_query(
            tool, ref,
            lambda record: sessions.session_asset(
                tool, record.canonical_ref, asset_id, self._ports,
            ),
        )

    def list_session_metadata(self) -> dict:
        return metadata.list_all(self._ports)

    def session_backbone(self, tool: str, ref: str) -> dict:
        return self._checked_query(
            tool, ref,
            lambda record: summaries.build_backbone(
                tool, record.canonical_ref, self._ports,
            ),
        )

    def set_session_summaries(self, tool: str, session_id: str, digests: dict) -> dict:
        return summaries.set_summaries(tool, session_id, digests, self._ports)

    def organization_digest_context(self, targets: list[dict]) -> dict:
        return organizing.digest_context(targets, self._ports)

    def organization_propose(self, targets: list[dict]) -> dict:
        return organizing.propose(targets, self._ports)

    def organization_proposals_list(self, status: str | None = None) -> list[dict]:
        return organizing.list_proposals(status, self._ports)

    def organization_proposal_modify(self, proposal_id: str, changes: list[dict]) -> dict:
        return organizing.modify(proposal_id, changes, self._ports)

    def organization_proposal_decide(self, proposal_id: str, decision: str) -> dict:
        return organizing.decide(proposal_id, decision, self._ports)

    def load_runtime_sessions(self) -> list[dict]:
        return runtime_sessions.load_all(self._ports)

    def commit_runtime_session(self, update: dict) -> dict:
        return runtime_sessions.commit(update, self._ports)

    def delete_runtime_session(self, session_id: str) -> dict:
        return runtime_sessions.delete(session_id, self._ports)

    def agent_search_sessions(self, query: str = "", **params) -> dict:
        return session_search.search_sessions(
            query, index=self._index, **params,
        )

    def agent_session_read(self, tool: str, **params) -> dict:
        return agent_tools.session_read(tool, index=self._index, **params)

    def agent_get_usage(self, **params) -> dict:
        return session_usage.get_usage(index=self._index, **params)

    def operation_plan(self, value: dict) -> dict:
        return self._operations.plan(value)

    def operation_apply(self, plan_id: str) -> dict:
        return self._operations.apply(plan_id)

    def operation_status(self, plan_id: str) -> dict:
        return self._operations.status(plan_id)

    def operation_cancel(self, plan_id: str) -> dict:
        return self._operations.cancel(plan_id)
