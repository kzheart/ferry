"""统一写操作计划。"""
from __future__ import annotations

import json
import threading
from concurrent.futures import Future, ThreadPoolExecutor

from ..sessions.index import AgentSessionIndex
from ..context import EngineContext
from ..errors import AgentRequestError
from .edit import EditOperationHandler
from .executor import OperationExecutor
from .migrate import MigrationService
from .plan_store import (
    OperationPlan,
    OperationPlanStore,
    OperationState,
    canonical_json,
    digest_json,
    now_ms,
)
from .planner import OperationPlanner


MUTATION_WORKERS = 1


class OperationService:
    def __init__(self, ports: EngineContext,
                 index: AgentSessionIndex):
        migration = MigrationService(ports)
        edit = EditOperationHandler(ports, index)
        self._lock = threading.RLock()
        # 所有写操作都在同一个持久化队列中串行执行。这样 IPC 请求可立即
        # 返回，同时不放宽已有 Adapter/原生文件的写入并发假设。
        self._executor = ThreadPoolExecutor(
            max_workers=MUTATION_WORKERS,
            thread_name_prefix="engine-operation",
        )
        self._jobs: dict[str, Future[None]] = {}
        self._plans = OperationPlanStore(ports.snapshot_dir)
        self._planner = OperationPlanner(
            ports,
            index,
            migration,
            edit,
            self._store_plan,
            self._database,
        )
        self._operation_executor = OperationExecutor(
            ports,
            index,
            migration,
            edit,
            self._database,
        )

    def _database(self):
        return self._plans.database()

    def plan(self, value: dict) -> dict:
        return self._planner.plan(value)

    def _store_plan(self, operation_input: dict, preview: dict, *,
                    base_revision: str,
                    document_revision: str | None) -> dict:
        with self._lock:
            return self._plans.create(
                operation_input,
                preview,
                base_revision=base_revision,
                document_revision=document_revision,
            )

    def apply(self, plan_id: str) -> dict:
        with self._lock:
            operation, state = self._get(plan_id)
            self._expire(operation, state)
            if state.status != "planned":
                raise AgentRequestError(
                    "operation plan 当前状态不可执行",
                    {"plan_id": plan_id, "status": state.status},
                )
            if not self._database().operations.enqueue(plan_id, now_ms()):
                _operation, current_state = self._get(plan_id)
                raise AgentRequestError(
                    "operation plan 当前状态不可执行",
                    {"plan_id": plan_id, "status": current_state.status},
                )
            self._jobs[plan_id] = self._executor.submit(self._run, plan_id)
        return self.status(plan_id)

    def _run(self, plan_id: str) -> None:
        with self._lock:
            operation, state = self._get(plan_id)
            if state.status != "queued":
                return
            if not self._database().operations.claim_queued(plan_id, now_ms()):
                return
        try:
            result = self._operation_executor.execute(operation)
        except Exception as error:
            with self._lock:
                self._database().operations.fail(
                    plan_id, type(error).__name__, now_ms(),
                )
            raise

        result_json = canonical_json(result)
        with self._lock:
            self._database().operations.finish(
                plan_id, result_json, digest_json(result_json), now_ms(),
            )

    def status(self, plan_id: str) -> dict:
        with self._lock:
            operation, state = self._get(plan_id)
            self._expire(operation, state)
            result = {
                "plan_id": plan_id,
                "kind": operation.kind,
                "status": state.status,
                "created_at": operation.created_at,
                "expires_at": operation.expires_at,
                "updated_at": state.updated_at,
            }
            if state.error_type:
                result["error_type"] = state.error_type
            if state.result_json is not None:
                result["result"] = json.loads(state.result_json)
            return result

    def cancel(self, plan_id: str) -> dict:
        with self._lock:
            operation, state = self._get(plan_id)
            self._expire(operation, state)
            if state.status not in {"planned", "queued"}:
                raise AgentRequestError(
                    "仅 planned 或 queued operation 可以取消",
                    {"plan_id": plan_id, "status": state.status},
                )
            if not self._database().operations.cancel(
                plan_id, state.status, now_ms(),
            ):
                raise AgentRequestError(
                    "operation plan 当前状态不可取消",
                    {"plan_id": plan_id},
                )
            return {"plan_id": plan_id, "status": "cancelled"}

    def wait(self, plan_id: str, timeout: float | None = None) -> dict:
        """测试与进程内编排辅助：等待已排队任务，不属于 RPC surface。"""
        with self._lock:
            job = self._jobs.get(plan_id)
        if job is not None:
            job.result(timeout=timeout)
        return self.status(plan_id)

    def shutdown(self) -> None:
        """仅在 Engine 重建或测试清理时调用，确保不遗留后台写入线程。"""
        self._executor.shutdown(wait=True, cancel_futures=True)

    def audit(self, plan_id: str) -> list[dict]:
        with self._lock:
            self._get(plan_id)
            return self._database().operations.audit(plan_id)

    def _get(self, plan_id: str) -> tuple[OperationPlan, OperationState]:
        return self._plans.get(plan_id)

    def _expire(self, operation: OperationPlan, state: OperationState) -> None:
        self._plans.expire(operation, state)
