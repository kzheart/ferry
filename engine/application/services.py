#!/usr/bin/env python3
"""GUI 结构化接口层:全部函数返回可 JSON 序列化的 dict/list。

既可由 interfaces/rpc.py 调用，也可通过 CLI 调试:
    python3 -m engine.interfaces.cli scan
    python3 -m engine.interfaces.cli show claude <sid>
    python3 -m engine.interfaces.cli history / env
"""
import json
from pathlib import Path

from ..domain.errors import SnapshotInvalidSourceError
from .ports import ApplicationPorts, current


def _application_ports(ports: ApplicationPorts | None = None) -> ApplicationPorts:
    return ports or current()


def adapter(tool, ports: ApplicationPorts | None = None):
    return _application_ports(ports).adapter(tool)


def _adapter_for(tool: str, ports: ApplicationPorts | None):
    """显式 ports 走注入依赖；默认路径保留可替换的公开 seam。"""
    return ports.adapter(tool) if ports is not None else adapter(tool)


def _call_with_ports(fn, *args, ports: ApplicationPorts | None, **kwargs):
    if ports is None:
        return fn(*args, **kwargs)
    return fn(*args, ports=ports, **kwargs)


def adapters(ports: ApplicationPorts | None = None):
    return _application_ports(ports).adapters()


def resource_path(*parts, ports: ApplicationPorts | None = None):
    return _application_ports(ports).resource_path(*parts)


def snapshot_dir(ports: ApplicationPorts | None = None):
    return _application_ports(ports).snapshot_dir()

from . import history as _history
from .pricing import pricing  # noqa: F401  暴露给 RPC


# ---------- 会话元数据(重命名/置顶/归档/标签) ----------

from . import session_meta as _session_meta  # noqa: E402

META_FIELDS = {
    "name", "pinned", "archived", "tags",
    "summary", "cluster_id", "cluster_name", "dead_candidate", "dead_reason",
}


def session_meta_compare_and_set(
        tool: str, session_id: str, expected: dict, patch: dict,
) -> dict:
    return _session_meta.compare_and_set_entry(
        tool, session_id,
        expected, {k: v for k, v in patch.items() if k in META_FIELDS}, current())


def session_meta_compare_and_set_many(changes: list[dict]) -> dict:
    return _session_meta.compare_and_set_entries([
        {
            "tool": change["tool"],
            "id": change["id"],
            "expected": change.get("expected", {}),
            "patch": {k: v for k, v in change.get("patch", {}).items()
                      if k in META_FIELDS},
        }
        for change in changes
    ], current())


# ---------- 会话生命周期 ----------

def session_delete(tool: str, ref: str) -> dict:
    """删除会话前先落快照(回收站语义);具体策略由插件 lifecycle 决定。"""
    impl = adapter(tool)
    return impl.lifecycle.delete(impl, ref)


def session_undelete(snapshot: str) -> dict:
    """Validate a deletion snapshot, then delegate restoration to its adapter."""
    snap = Path(snapshot)
    if snap.parent != snapshot_dir():
        raise SnapshotInvalidSourceError("只允许从快照目录恢复", {"snapshot": snapshot})
    try:
        meta = json.loads(snap.with_suffix(".meta.json").read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise SnapshotInvalidSourceError("快照缺少元数据,无法撤销",
                                         {"snapshot": snapshot}) from error
    tool = meta.get("tool")
    if not isinstance(tool, str) or not tool:
        raise SnapshotInvalidSourceError("快照缺少来源 Agent", {"snapshot": snapshot})
    return adapter(tool).lifecycle.restore_delete(snap, meta)


# ---------- 迁移历史 / 快照 ----------

# ---------- 环境 / 模型列表 ----------


# ---------- CLI ----------


def version() -> dict:
    return {"version": current().version, "protocol": 2}


def health() -> dict:
    return {"status": "ok", **version()}


# 进程边界在这里取得一次依赖；查询用例本身不再触碰全局 composition。
from . import environment as _environment  # noqa: E402
from . import models as _models  # noqa: E402
from . import scanning as _scanning  # noqa: E402
from . import sessions as _sessions  # noqa: E402
from . import summaries as _summaries  # noqa: E402
from . import organizing as _organizing  # noqa: E402
from . import runtime_sessions as _runtime_sessions  # noqa: E402


def env() -> dict:
    return _environment.inspect(current())


def list_models(tool_name: str) -> dict:
    return _models.list_models(tool_name, current())


def scan() -> dict:
    return _scanning.scan(current())


def _append_history(entry: dict, *, ports: ApplicationPorts | None = None) -> str:
    return _history.append(entry, _application_ports(ports))


def history() -> list[dict]:
    return _history.list_entries(current())


def history_delete(history_id: str) -> dict:
    return _history.delete(history_id, current())


def _read_tree(tool_name: str, ref: str,
               *, ports: ApplicationPorts | None = None):
    return _sessions.read_tree(tool_name, ref, _application_ports(ports))


def show(tool_name: str, ref: str) -> dict:
    return _sessions.show(tool_name, ref, current())


def session_asset(tool_name: str, ref: str, asset_id: str) -> dict:
    return _sessions.session_asset(tool_name, ref, asset_id, current())


def session_meta_list() -> dict:
    return _session_meta.list_all(current())


def session_backbone(tool_name: str, ref: str) -> dict:
    return _summaries.build_backbone(tool_name, ref, current())


def set_session_summaries(tool_name: str, session_id: str, digests: dict) -> dict:
    return _summaries.set_summaries(tool_name, session_id, digests, current())


def organization_digest_context(targets: list[dict]) -> dict:
    return _organizing.digest_context(targets, current())


def organization_propose(targets: list[dict]) -> dict:
    return _organizing.propose(targets, current())


def organization_proposals_list(status: str | None = None) -> list[dict]:
    return _organizing.list_proposals(status, current())


def organization_proposal_modify(proposal_id: str, changes: list[dict]) -> dict:
    return _organizing.modify(proposal_id, changes, current())


def organization_proposal_decide(proposal_id: str, decision: str) -> dict:
    return _organizing.decide(proposal_id, decision, current())


def load_runtime_sessions() -> list[dict]:
    return _runtime_sessions.load_all(current())


def commit_runtime_session(update: dict) -> dict:
    return _runtime_sessions.commit(update, current())


def delete_runtime_session(session_id: str) -> dict:
    return _runtime_sessions.delete(session_id, current())
