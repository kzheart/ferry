"""Operation plan 的当前模型、摘要和 SQLite 装载。"""
from __future__ import annotations

import hashlib
import json
import secrets
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

from ..contracts.operations import (
    OPERATION_PLAN_ID_PREFIX,
    OPERATION_STATUSES,
)
from ..errors import AgentRequestError
from ..storage.database import StateDatabase


PLAN_TTL_MS = 10 * 60 * 1000


def now_ms() -> int:
    return int(time.time() * 1000)


def canonical_json(value) -> str:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    )


def digest_json(value_json: str) -> str:
    return hashlib.sha256(value_json.encode()).hexdigest()


@dataclass(frozen=True)
class OperationPlan:
    plan_id: str
    kind: str
    input_json: str
    preview_json: str
    input_digest: str
    preview_digest: str
    base_revision: str
    document_revision: str | None
    created_at: int
    expires_at: int

    def input(self) -> dict:
        return json.loads(self.input_json)

    def preview(self) -> dict:
        return json.loads(self.preview_json)


@dataclass
class OperationState:
    status: str = "planned"
    result_json: str | None = None
    error_type: str | None = None
    updated_at: int = 0

    def __post_init__(self):
        if self.status not in OPERATION_STATUSES:
            raise AgentRequestError(
                "operation status 非法", {"status": self.status},
            )


class OperationPlanStore:
    def __init__(self, snapshot_dir: Callable[[], str | Path]):
        self._snapshot_dir = snapshot_dir
        self._database_instance: StateDatabase | None = None
        self._database_path: Path | None = None

    def database(self) -> StateDatabase:
        path = Path(self._snapshot_dir()) / "ferry-state.sqlite3"
        if self._database_instance is None or self._database_path != path:
            self._database_instance = StateDatabase(path)
            self._database_path = path
        return self._database_instance

    def create(
        self,
        operation_input: dict,
        preview: dict,
        *,
        base_revision: str,
        document_revision: str | None,
    ) -> dict:
        input_json = canonical_json(operation_input)
        preview_json = canonical_json(preview)
        created_at = now_ms()
        operation = OperationPlan(
            plan_id=OPERATION_PLAN_ID_PREFIX + secrets.token_urlsafe(18),
            kind=operation_input["kind"],
            input_json=input_json,
            preview_json=preview_json,
            input_digest=digest_json(input_json),
            preview_digest=digest_json(preview_json),
            base_revision=base_revision,
            document_revision=document_revision,
            created_at=created_at,
            expires_at=created_at + PLAN_TTL_MS,
        )
        self.database().operations.store_plan(operation, created_at)
        return public_plan(operation)

    def get(self, plan_id: str) -> tuple[OperationPlan, OperationState]:
        if (
            not isinstance(plan_id, str)
            or not plan_id.startswith(OPERATION_PLAN_ID_PREFIX)
        ):
            raise AgentRequestError("plan_id 非法")
        row = self.database().operations.get(plan_id)
        if row is None:
            raise AgentRequestError("operation plan 不存在或已因重启失效")
        return (
            OperationPlan(
                plan_id=row["plan_id"],
                kind=row["kind"],
                input_json=row["input_json"],
                preview_json=row["preview_json"],
                input_digest=row["input_digest"],
                preview_digest=row["preview_digest"],
                base_revision=row["base_revision"],
                document_revision=row["document_revision"],
                created_at=row["created_at"],
                expires_at=row["expires_at"],
            ),
            OperationState(
                status=row["status"],
                result_json=row["result_json"],
                error_type=row["error_type"],
                updated_at=row["updated_at"],
            ),
        )

    def expire(
        self, operation: OperationPlan, state: OperationState
    ) -> None:
        if state.status == "planned" and operation.expires_at < now_ms():
            updated_at = now_ms()
            self.database().operations.expire(operation.plan_id, updated_at)
            state.status = "expired"
            state.updated_at = updated_at


def public_plan(operation: OperationPlan) -> dict:
    params = operation.input()
    if operation.kind == "migration":
        summary = (
            f"将 {params['source_tool']} 会话迁移到 {params['target_tool']}"
        )
    elif operation.kind == "metadata":
        summary = "修改会话元数据"
    elif operation.kind == "delete":
        summary = "删除原始会话（执行前创建恢复快照）"
    elif operation.kind == "restore-delete":
        summary = "恢复已删除的会话"
    else:
        summary = "修改原始会话（执行前自动创建可恢复快照）"
    return {
        "plan_id": operation.plan_id,
        "kind": operation.kind,
        "status": "planned",
        "preview": operation.preview(),
        "summary": summary,
        "risk": "low" if operation.kind == "metadata" else "high",
        "affected_refs": [params["ref"]] if "ref" in params else [],
        "base_revision": operation.base_revision,
        "document_revision": operation.document_revision,
        "input_digest": operation.input_digest,
        "preview_digest": operation.preview_digest,
        "created_at": operation.created_at,
        "expires_at": operation.expires_at,
    }
