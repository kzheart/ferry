"""Engine 的应用层入口。

RPC/CLI 在进程边界构造此对象；每个用例通过显式 ports 访问基础设施，
而不是在业务路径中回读全局 composition。
"""
from __future__ import annotations

from . import agent_tools, environment, history, models, organizing, runtime_sessions
from . import scanning, session_meta, sessions, summaries
from .operations import OperationService
from .ports import ApplicationPorts
from .pricing import pricing


class EngineApplication:
    def __init__(self, ports: ApplicationPorts,
                 index: agent_tools.AgentSessionIndex,
                 operations: OperationService):
        self._ports = ports
        self._index = index
        self._operations = operations

    def close(self) -> None:
        self._operations.shutdown()

    def health(self) -> dict:
        return {"status": "ok", **self.version()}

    def version(self) -> dict:
        return {"version": self._ports.version, "protocol": 2}

    def scan(self) -> dict:
        return scanning.scan(self._ports, self._index)

    def environment(self) -> dict:
        return environment.inspect(self._ports)

    def resume_command(self, tool: str, session_id: str, cwd: str) -> dict:
        return self._ports.adapter(tool).lifecycle.resume_descriptor(session_id, cwd)

    def list_models(self, tool: str) -> dict:
        return models.list_models(tool, self._ports)

    def pricing(self, force: bool = False) -> dict:
        return pricing(force=force)

    def migration_history(self) -> list[dict]:
        return history.list_entries(self._ports)

    def delete_migration_history(self, history_id: str) -> dict:
        return history.delete(history_id, self._ports)

    def show_session(self, tool: str, ref: str) -> dict:
        return sessions.show(tool, ref, self._ports)

    def session_asset(self, tool: str, ref: str, asset_id: str) -> dict:
        return sessions.session_asset(tool, ref, asset_id, self._ports)

    def list_session_metadata(self) -> dict:
        return session_meta.list_all(self._ports)

    def session_backbone(self, tool: str, ref: str) -> dict:
        return summaries.build_backbone(tool, ref, self._ports)

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
        return agent_tools.search_sessions(query, index=self._index, **params)

    def agent_session_read(self, tool: str, **params) -> dict:
        return agent_tools.session_read(tool, index=self._index, **params)

    def agent_get_usage(self, **params) -> dict:
        return agent_tools.get_usage(index=self._index, **params)

    def operation_plan(self, value: dict) -> dict:
        return self._operations.plan(value)

    def operation_apply(self, plan_id: str) -> dict:
        return self._operations.apply(plan_id)

    def operation_status(self, plan_id: str) -> dict:
        return self._operations.status(plan_id)

    def operation_cancel(self, plan_id: str) -> dict:
        return self._operations.cancel(plan_id)
