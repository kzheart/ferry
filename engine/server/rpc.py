"""`ferry-ipc/1` RPC 契约与调度：结构化错误 envelope。"""

import json
import logging
import os

from ..app import EngineService
from ..bootstrap import build_engine
from ..contracts.engine_methods import ENGINE_METHOD_NAMES
from ..contracts.ipc import FERRY_IPC_PROTOCOL
from ..errors import (
    DomainError,
    InvalidJsonError,
    InvalidRequestError,
    MissingParamError,
    UnknownMethodError,
    UnsupportedProtocolError,
)
from ..operations.verification import ProbeTimeout
from ..sessions import agent_read

log = logging.getLogger(__name__)

PROTOCOL = FERRY_IPC_PROTOCOL


RPC_METHODS = ENGINE_METHOD_NAMES


def _error_envelope(error: DomainError, request_id: str) -> dict:
    payload = {"code": error.code, "params": error.params,
               "category": error.category, "retryable": error.retryable}
    return {
        "protocol": PROTOCOL,
        "id": request_id,
        "ok": False,
        "error": payload,
    }


class RpcDispatcher:
    """绑定一个 EngineService 的 RPC 调度器。"""

    def __init__(self, application: EngineService):
        self._application = application
        self._methods = {
            "health": lambda p: application.health(),
            "version": lambda p: application.version(),
            "scan": lambda p: application.scan(),
            "env": lambda p: application.environment(),
            "resume": lambda p: application.resume_command(p["tool"], p["ref"]),
            "models": lambda p: application.list_models(p["tool"]),
            "history": lambda p: application.migration_history(),
            "history_delete": lambda p: application.delete_migration_history(p["id"]),
            "pricing": lambda p: application.pricing(
                force=p.get("force", False),
            ),
            "show": lambda p: application.show_session(p["tool"], p["ref"]),
            "session_asset": lambda p: application.session_asset(p["tool"], p["ref"], p["asset_id"]),
            "session_meta_list": lambda p: application.list_session_metadata(),
            "session_backbone": lambda p: application.session_backbone(p["tool"], p["ref"]),
            "session_summaries_set": lambda p: application.set_session_summaries(p["tool"], p["id"], p.get("digests") or {}),
            "organization_digest_context": lambda p: application.organization_digest_context(p["targets"]),
            "organization_propose": lambda p: application.organization_propose(p["targets"]),
            "organization_proposals_list": lambda p: application.organization_proposals_list(p.get("status")),
            "organization_proposal_modify": lambda p: application.organization_proposal_modify(p["proposal_id"], p["changes"]),
            "organization_proposal_decide": lambda p: application.organization_proposal_decide(p["proposal_id"], p["decision"]),
            "runtime_sessions.load_all": lambda p: application.load_runtime_sessions(),
            "runtime_sessions.commit": lambda p: application.commit_runtime_session(p["update"]),
            "runtime_sessions.delete": lambda p: application.delete_runtime_session(p["session_id"]),
            "agent_search_sessions": lambda p: application.agent_search_sessions(p.get("query", ""), agents=p.get("agents"), projects=p.get("projects"), time_range=p.get("time_range"), limit=p.get("limit", 20)),
            "agent_session_read": lambda p: application.agent_session_read(p["tool"], ref=p["ref"], terms=p.get("terms"), roles=p.get("roles"), from_message=p.get("from_message", 1), limit=p.get("limit", 20), include_tool_outputs=p.get("include_tool_outputs", False), max_bytes=p.get("max_bytes", agent_read.DEFAULT_CONTEXT_BYTES)),
            "agent_get_usage": lambda p: application.agent_get_usage(agents=p.get("agents"), projects=p.get("projects"), time_range=p.get("time_range")),
            "operation.plan": lambda p: application.operation_plan(p["input"]),
            "operation.apply": lambda p: application.operation_apply(p["plan_id"]),
            "operation.status": lambda p: application.operation_status(p["plan_id"]),
            "operation.cancel": lambda p: application.operation_cancel(p["plan_id"]),
        }
        if set(self._methods) != RPC_METHODS:
            raise RuntimeError("Engine RPC 与生成方法契约不一致")

    @property
    def method_names(self):
        return frozenset(self._methods)

    def handle(self, request: str) -> dict:
        return _handle(request, self._methods)


def _handle(request: str, methods: dict) -> dict:
    request_id = "unknown"
    try:
        try:
            req = json.loads(request)
        except json.JSONDecodeError as error:
            raise InvalidJsonError(str(error)) from error
        if not isinstance(req, dict):
            raise InvalidJsonError("请求必须是 JSON object")
        if isinstance(req.get("id"), str) and req["id"]:
            request_id = req["id"]
        if req.get("protocol") != PROTOCOL:
            raise UnsupportedProtocolError(PROTOCOL, req.get("protocol"))
        if set(req) != {"protocol", "id", "method", "params"}:
            raise InvalidRequestError("请求 envelope 字段不匹配")
        if len(request_id) > 128:
            raise InvalidRequestError("请求 id 超出长度限制")
        if not isinstance(req.get("params"), dict):
            raise InvalidRequestError("params 必须是 JSON object")
        method = req.get("method")
        if not isinstance(method, str) or not method:
            raise InvalidRequestError("method 必须是非空字符串")
        fn = methods.get(method)
        if fn is None:
            raise UnknownMethodError(method)
        try:
            result = fn(req["params"])
        except KeyError as error:
            raise MissingParamError(str(error.args[0])) from error
        return {
            "protocol": PROTOCOL,
            "id": request_id,
            "ok": True,
            "result": result,
        }
    except ProbeTimeout as error:
        log.exception("RPC probe timeout")
        timeout = DomainError(str(error))
        timeout.code, timeout.category, timeout.retryable = \
            "probe.timeout", "internal", True
        return _error_envelope(timeout, request_id)
    except DomainError as error:
        log.warning("RPC domain error: %s", error, exc_info=True)
        return _error_envelope(error, request_id)
    except (SystemExit, Exception) as error:
        # 生产 RPC 不暴露任意异常文本；完整异常链只进日志。
        log.exception("RPC internal error")
        internal = DomainError("internal")
        if os.environ.get("FERRY_DEBUG"):
            internal = DomainError(f"{type(error).__name__}: {error}")
        return _error_envelope(internal, request_id)


def rpc(request: str) -> dict:
    """测试用一次性入口；sidecar 必须使用 RpcDispatcher。"""
    application = build_engine()
    try:
        return RpcDispatcher(application).handle(request)
    finally:
        application.close()
