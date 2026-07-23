"""版本 2 RPC 契约与调度：结构化错误 envelope。"""

import json
import logging
import os

from ..application import services
from ..application import agent_tools
from ..application import organizing
from ..application import operations
from ..application import runtime_sessions
from ..application.verification import ProbeTimeout
from ..contracts.engine_methods import ENGINE_METHOD_NAMES
from ..domain.errors import (
    DomainError, InvalidJsonError, MissingParamError, UnknownMethodError,
)

log = logging.getLogger(__name__)

PROTOCOL = 2
RPC_METHODS = {
    "health": lambda p: services.health(),
    "version": lambda p: services.version(),
    "scan": lambda p: services.scan(),
    "env": lambda p: services.env(),
    "resume": lambda p: services.resume_command(
        p["tool"], p["session_id"], p.get("cwd") or "."),
    "models": lambda p: services.list_models(p["tool"]),
    "history": lambda p: services.history(),
    "history_delete": lambda p: services.history_delete(p["id"]),
    "pricing": lambda p: services.pricing(force=p.get("force", False)),
    "show": lambda p: services.show(p["tool"], p["ref"]),
    "session_asset": lambda p: services.session_asset(p["tool"], p["ref"], p["asset_id"]),
    "edit_capabilities": lambda p: services.edit_capabilities(p["tool"]),
    "session_meta_list": lambda p: services.session_meta_list(),
    "session_backbone": lambda p: services.session_backbone(p["tool"], p["ref"]),
    "session_summaries_set": lambda p: services.set_session_summaries(
        p["tool"], p["id"], p.get("digests") or {}),
    "organization_digest_context": lambda p: organizing.digest_context(
        p["targets"]),
    "organization_propose": lambda p: organizing.propose(p["targets"]),
    "organization_proposals_list": lambda p: organizing.list_proposals(
        p.get("status")),
    "organization_proposal_modify": lambda p: organizing.modify(
        p["proposal_id"], p["changes"]),
    "organization_proposal_decide": lambda p: organizing.decide(
        p["proposal_id"], p["decision"]),
    "runtime_sessions.load_all": lambda p: runtime_sessions.load_all(),
    "runtime_sessions.commit": lambda p: runtime_sessions.commit(p["update"]),
    "runtime_sessions.delete": lambda p: runtime_sessions.delete(p["session_id"]),
    "agent_search_sessions": lambda p: agent_tools.search_sessions(
        p.get("query", ""), agents=p.get("agents"), projects=p.get("projects"),
        time_range=p.get("time_range"), limit=p.get("limit", 20)),
    "agent_session_read": lambda p: agent_tools.session_read(
        p["tool"], ref=p.get("ref"), session_id=p.get("session_id"),
        terms=p.get("terms"), roles=p.get("roles"),
        from_message=p.get("from_message", 1), limit=p.get("limit", 20),
        include_tool_outputs=p.get("include_tool_outputs", False),
        max_bytes=p.get("max_bytes", agent_tools.DEFAULT_CONTEXT_BYTES)),
    "agent_get_usage": lambda p: agent_tools.get_usage(
        agents=p.get("agents"), projects=p.get("projects"),
        time_range=p.get("time_range")),
    "operation.plan": lambda p: operations.plan(p["input"]),
    "operation.apply": lambda p: operations.apply(p["plan_id"]),
    "operation.status": lambda p: operations.status(p["plan_id"]),
    "operation.cancel": lambda p: operations.cancel(p["plan_id"]),
}

if set(RPC_METHODS) != ENGINE_METHOD_NAMES:
    missing = ENGINE_METHOD_NAMES - set(RPC_METHODS)
    extra = set(RPC_METHODS) - ENGINE_METHOD_NAMES
    raise RuntimeError(f"Engine RPC 与生成方法契约不一致: missing={missing}, extra={extra}")


def _error_envelope(error: DomainError, request_id) -> dict:
    payload = {"code": error.code, "params": error.params,
               "category": error.category, "retryable": error.retryable,
               "request_id": request_id}
    if os.environ.get("FERRY_DEBUG"):
        payload["debug"] = str(error)
    return {"protocol": PROTOCOL, "ok": False, "error": payload}


def rpc(request: str) -> dict:
    request_id = None
    try:
        try:
            req = json.loads(request)
        except json.JSONDecodeError as error:
            raise InvalidJsonError(str(error)) from error
        if not isinstance(req, dict):
            raise InvalidJsonError("请求必须是 JSON object")
        request_id = req.get("request_id")
        method = req.get("method")
        fn = RPC_METHODS.get(method)
        if fn is None:
            raise UnknownMethodError(method)
        try:
            result = fn(req.get("params") or {})
        except KeyError as error:
            raise MissingParamError(str(error.args[0])) from error
        return {"protocol": PROTOCOL, "ok": True, "result": result,
                "request_id": request_id}
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
