"""Engine 进程能力入口。

RPC/CLI 在进程边界构造此对象；各能力通过显式上下文访问依赖。
"""
from __future__ import annotations

import logging
import time

from .context import EngineContext
from .contracts.ipc import FERRY_CONTRACT_HASH
from .errors import (
    AgentReferenceError,
    AgentRequestError,
    require_agent_capability,
)
from .operations import history, metadata, verification
from .operations.service import OperationService
from .runtime import sessions as runtime_sessions
from .sessions import agent_read
from .sessions import search as session_search
from .sessions import usage as session_usage
from .sessions.content_index import ContentIndex
from .sessions.cleanup import CleanupService
from .sessions.index import AgentSessionIndex, IndexedSession
from .sessions.live import LiveIndexService
from .sessions import read as sessions
from .sessions import scan as scanning
from .system import environment, models
from .system.pricing import pricing


class EngineService:
    def __init__(self, ports: EngineContext,
                 index: AgentSessionIndex,
                 operations: OperationService,
                 content_index: ContentIndex | None = None,
                 cleanup: CleanupService | None = None):
        self._ports = ports
        self._index = index
        self._operations = operations
        self._content_index = content_index
        self._cleanup = cleanup or CleanupService(index, ports)
        self._live: LiveIndexService | None = None

    def close(self) -> None:
        if self._live is not None:
            self._live.stop()
        self._operations.shutdown()
        if self._content_index is not None:
            self._content_index.close()

    def enable_live_updates(self, notifier) -> None:
        """serve 模式专用:索引增量经 notifier 推送,并启动源变更轮询。"""
        self._index.on_delta = (
            lambda delta: notifier.emit("sessions.changed", delta)
        )
        self._live = LiveIndexService(self._index)
        self._live.start()

    def warm_agent_search(self) -> None:
        """serve 启动预热:先扫库,再把内容索引的缺口交给后台线程。"""
        if self._content_index is None:
            return
        log = logging.getLogger(__name__)
        try:
            started = time.monotonic()
            log.info("内容索引预热开始")
            records = self._index.refresh()
            log.info("预热扫库完成: %d 条会话 耗时=%.1fs",
                     len(records), time.monotonic() - started)
            self._content_index.sync(
                self._index, records, prefer_background=True,
            )
            log.info("内容索引预热完成 全程=%.1fs", time.monotonic() - started)
        except Exception:  # noqa: BLE001 - 预热失败不能影响 RPC 服务
            log.exception("内容索引预热失败")

    def health(self) -> dict:
        return {
            "status": "ready",
            "service": "engine",
            "contract_hash": FERRY_CONTRACT_HASH,
        }

    def version(self) -> dict:
        return {"version": self._ports.version}

    def scan(self) -> dict:
        return scanning.scan(self._ports, self._index, live=self._live)

    def scan_progress(self) -> dict:
        return scanning.scan_progress()

    def environment(self) -> dict:
        return environment.inspect(self._ports)

    def _checked_query(self, tool: str, ref: str, query):
        # UI 只读浏览走宽松解析:活跃会话随时被 CLI 追加写入,若像 Agent
        # 路径那样把内容 pin 死,点开正在进行的会话会稳定撞上
        # agent.reference_invalid。
        record = self._index.resolve(tool, ref, pin_content=False)
        return query(record)

    def resume_command(self, tool: str, ref: str) -> dict:
        lifecycle = require_agent_capability(
            self._ports.adapter(tool), "resume", "lifecycle",
        )

        def build(record: IndexedSession) -> dict:
            session_id = record.row.get("id")
            if not isinstance(session_id, str) or not session_id:
                raise AgentReferenceError("会话缺少原生 ID")
            cwd = record.row.get("dir")
            if not isinstance(cwd, str) or not cwd:
                cwd = "."
            return lifecycle.resume_descriptor(
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

    def show_session(self, tool: str, ref: str, *, from_message: int = 1,
                     limit: int | None = None) -> dict:
        require_agent_capability(
            self._ports.adapter(tool), "browse", "browser",
        )
        def show(record):
            # 浏览路径与 Agent 读取共用同一 locator 命名空间:详情里的
            # turn_locator 与 Agent preview 候选、operations 编辑通道对得上
            issuer = agent_read.browser_locator_issuer(self._index, record)
            if from_message == 1 and limit is None:
                return sessions.show(
                    tool, record.canonical_ref, self._ports,
                    locator_issuer=issuer,
                )
            return sessions.show(
                tool, record.canonical_ref, self._ports,
                from_message=from_message,
                message_limit=limit,
                tree_count=record.row.get("tree_count"),
                child_count=record.row.get("child_count"),
                total_count=record.row.get("count"),
                locator_issuer=issuer,
            )

        return self._checked_query(
            tool, ref,
            show,
        )

    def session_asset(self, tool: str, ref: str, asset_id: str) -> dict:
        require_agent_capability(
            self._ports.adapter(tool), "browse", "browser",
        )
        return self._checked_query(
            tool, ref,
            lambda record: sessions.session_asset(
                tool, record.canonical_ref, asset_id, self._ports,
            ),
        )

    def list_session_metadata(self) -> dict:
        return metadata.list_all(self._ports)

    def search_sessions_for_ui(self, query: str = "", **params) -> dict:
        return session_search.search_sessions_for_ui(
            query, index=self._index,
            content_index=self._content_index, **params,
        )

    def load_runtime_sessions(self) -> list[dict]:
        return runtime_sessions.load_all(self._ports)

    def commit_runtime_session(self, update: dict) -> dict:
        return runtime_sessions.commit(update, self._ports)

    def delete_runtime_session(self, session_id: str) -> dict:
        return runtime_sessions.delete(session_id, self._ports)

    def truncate_runtime_session(
        self, session_id: str, from_ordinal: int, from_seq: int,
    ) -> dict:
        return runtime_sessions.truncate(
            session_id, from_ordinal, from_seq, self._ports,
        )

    def agent_search_sessions(self, query: str = "", **params) -> dict:
        return session_search.search_sessions(
            query, index=self._index,
            content_index=self._content_index, **params,
        )

    def agent_session_read(self, tool: str, **params) -> dict:
        require_agent_capability(
            self._ports.adapter(tool), "browse", "browser",
        )
        return agent_read.session_read(tool, index=self._index, **params)

    def agent_get_usage(self, **params) -> dict:
        return session_usage.get_usage(index=self._index, **params)

    def agent_cleanup_inventory(
        self,
        scope: dict | None = None,
        cursor: str | None = None,
        page_size: int = 100,
    ) -> dict:
        return self._cleanup.inventory(scope, cursor, page_size)

    def agent_cleanup_triage(
        self,
        scope_id: str,
        verdicts: list[dict],
    ) -> dict:
        return self._cleanup.triage(scope_id, verdicts)

    def agent_prompt(
        self,
        tool: str,
        ref: str,
        prompt: str,
        model: str | None = None,
        timeout_sec: int = 360,
    ) -> dict:
        self._validate_agent_prompt(tool, ref, prompt, model, timeout_sec)
        record = self._index.resolve(tool, ref, pin_content=True)
        session_id = record.row.get("id")
        if not isinstance(session_id, str) or not session_id:
            raise AgentReferenceError("会话缺少原生 ID")
        cwd = record.row.get("dir")
        if not isinstance(cwd, str) or not cwd:
            cwd = "."

        try:
            report = verification.run_agent_prompt(
                tool,
                session_id,
                prompt,
                cwd,
                model,
                ports=self._ports,
                timeout=timeout_sec,
            )
        except Exception:
            self._refresh_agent_prompt_ref(tool, session_id)
            raise
        next_ref = self._refresh_agent_prompt_ref(tool, session_id)
        if not isinstance(report, dict):
            raise RuntimeError("Agent prompt verifier 返回值必须是 object")

        params = report.setdefault("params", {})
        if not isinstance(params, dict):
            raise RuntimeError("Agent prompt report params 必须是 object")
        params.setdefault("tool", tool)
        params["session_id"] = session_id
        if model is not None:
            params.setdefault("model", model)
        if next_ref is None:
            params["ref_refresh_failed"] = True
        else:
            report["next_ref"] = next_ref
        return report

    def _refresh_agent_prompt_ref(
        self,
        tool: str,
        session_id: str,
    ) -> str | None:
        try:
            records = self._index.refresh()
        except Exception:  # noqa: BLE001 - prompt 结果优先，刷新失败写入报告
            logging.getLogger(__name__).exception(
                "Agent prompt 后刷新索引失败",
            )
            return None
        match = next(
            (
                record for record in records
                if record.tool == tool and record.row.get("id") == session_id
            ),
            None,
        )
        return match.opaque_ref if match is not None else None

    def _validate_agent_prompt(
        self,
        tool,
        ref,
        prompt,
        model,
        timeout_sec,
    ) -> None:
        if (
            not isinstance(tool, str)
            or not tool
            or tool not in self._ports.adapters()
        ):
            raise AgentRequestError(
                "agent_prompt tool 无效",
                {"field": "tool"},
            )
        if not isinstance(ref, str) or not ref:
            raise AgentRequestError(
                "agent_prompt ref 无效",
                {"field": "ref"},
            )
        if not isinstance(prompt, str) or not 1 <= len(prompt) <= 100_000:
            raise AgentRequestError(
                "agent_prompt prompt 长度必须为 1..100000",
                {"field": "prompt"},
            )
        if model is not None and (
            not isinstance(model, str)
            or not 1 <= len(model) <= 512
            or any(ord(character) < 32 for character in model)
        ):
            raise AgentRequestError(
                "agent_prompt model 长度必须为 1..512",
                {"field": "model"},
            )
        if (
            isinstance(timeout_sec, bool)
            or not isinstance(timeout_sec, int)
            or not 1 <= timeout_sec <= 360
        ):
            raise AgentRequestError(
                "agent_prompt timeout_sec 必须为 1..360 的整数",
                {"field": "timeout_sec"},
            )

    def operation_plan(self, value: dict) -> dict:
        return self._operations.plan(value)

    def operation_apply(self, plan_id: str) -> dict:
        return self._operations.apply(plan_id)

    def operation_status(self, plan_id: str) -> dict:
        return self._operations.status(plan_id)

    def operation_cancel(self, plan_id: str) -> dict:
        return self._operations.cancel(plan_id)
