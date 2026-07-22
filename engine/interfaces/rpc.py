"""版本 2 RPC 契约与调度：结构化错误 envelope。"""

import contextlib
import json
import logging
import os
import sys

from ..application import services
from ..application import agent_tools
from ..application import agent_mutations
from ..application.verification import ProbeTimeout
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
    "tools": lambda p: services.tool_manifests(),
    "resume": lambda p: services.resume_command(
        p["tool"], p["session_id"], p.get("cwd") or "."),
    "models": lambda p: services.list_models(p["tool"]),
    "history": lambda p: services.history(),
    "pricing": lambda p: services.pricing(force=p.get("force", False)),
    "show": lambda p: services.show(p["tool"], p["ref"]),
    "session_asset": lambda p: services.session_asset(p["tool"], p["ref"], p["asset_id"]),
    "authoring_capabilities": lambda p: services.authoring_capabilities(p["tool"]),
    "authoring_preview": lambda p: services.authoring_preview(
        p["ref"], p["turn"], p["reply"], tool=p.get("tool", "claude")),
    "authoring_apply": lambda p: services.authoring_apply(
        p["ref"], p["turn"], p["reply"], probe=p.get("probe", False),
        save_as=p.get("save_as", False), tool=p.get("tool", "claude"),
        revision=p.get("revision")),
    "migrate": lambda p: services.migrate(p["src"], p["dst"], p["ref"],
        cwd=p.get("cwd"), dry_run=p.get("dry_run", False),
        probe=p.get("probe", False), max_turn=p.get("max_turn"),
        probe_model=p.get("probe_model") or None,
        content_locale=p.get("content_locale")),
    "handoff": lambda p: services.handoff(p["src"], p["ref"], p["dst"], cwd=p.get("cwd")),
    "edit_capabilities": lambda p: services.edit_capabilities(p["tool"]),
    "edit_preview": lambda p: services.edit_preview(p["ref"], p["ops"], tool=p.get("tool", "claude")),
    "edit_apply": lambda p: services.edit_apply(p["ref"], p["ops"], probe=p.get("probe", False),
        save_as=p.get("save_as", False), tool=p.get("tool", "claude")),
    "session_delete": lambda p: services.session_delete(p["tool"], p["ref"]),
    "session_undelete": lambda p: services.session_undelete(p["snapshot"]),
    "session_meta_list": lambda p: services.session_meta_list(),
    "session_meta_set": lambda p: services.session_meta_set(p["id"], p.get("patch") or {}),
    "agent_list_capabilities": lambda p: agent_tools.list_capabilities(),
    "agent_search_sessions": lambda p: agent_tools.search_sessions(
        p.get("query", ""), agents=p.get("agents"), projects=p.get("projects"),
        time_range=p.get("time_range"), limit=p.get("limit", 20)),
    "agent_get_session_context": lambda p: agent_tools.get_session_context(
        p["tool"], p["ref"], from_turn=p.get("from_turn", 1),
        to_turn=p.get("to_turn"),
        include_tool_outputs=p.get("include_tool_outputs", False),
        max_bytes=p.get("max_bytes", agent_tools.DEFAULT_CONTEXT_BYTES)),
    "agent_get_usage": lambda p: agent_tools.get_usage(
        agents=p.get("agents"), projects=p.get("projects"),
        time_range=p.get("time_range")),
    "agent_preview_migration": lambda p: agent_tools.preview_migration(
        p["source_tool"], p["ref"], p["target_tool"],
        max_turn=p.get("max_turn")),
    "agent_preview_edit": lambda p: agent_tools.preview_edit(
        p["tool"], p["ref"], ops=p.get("ops"), turn=p.get("turn"),
        reply=p.get("reply")),
    "agent_propose_migration": lambda p: agent_mutations.propose_migration(
        p["source_tool"], p["ref"], p["target_tool"], p["run_id"],
        max_turn=p.get("max_turn")),
    "agent_propose_edit": lambda p: agent_mutations.propose_edit(
        p["tool"], p["ref"], ops=p.get("ops"), turn=p.get("turn"),
        reply=p.get("reply"), save_as=p.get("save_as", True),
        run_id=p["run_id"]),
    "agent_propose_metadata_change": lambda p:
        agent_mutations.propose_metadata_change(
            p["tool"], p["ref"], p["patch"], p["run_id"]),
    "agent_operation_authorize": lambda p: agent_mutations.authorize(
        p["operation_id"], p["run_id"]),
    "agent_operation_detail": lambda p: agent_mutations.detail(
        p["operation_id"]),
    "agent_operation_apply": lambda p: agent_mutations.apply(
        p["operation_id"], p["run_id"], p["approval_token"]),
    "agent_operation_status": lambda p: agent_mutations.status(
        p["operation_id"]),
}


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
            with contextlib.redirect_stdout(sys.stderr):
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
