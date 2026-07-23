#!/usr/bin/env python3
"""GUI 结构化接口层:全部函数返回可 JSON 序列化的 dict/list。

既可 import(gui/server.py 直接调用),也可命令行调试:
    python3 -m engine.api scan
    python3 -m engine.api show claude <sid>
    python3 -m engine.api history / env
"""
import json
import time
from pathlib import Path

from ..domain.errors import SnapshotInvalidSourceError
from . import verification as probe_mod
from ..adapters.base import narration
from .ports import current


def adapter(tool):
    return current().adapter(tool)


def adapters():
    return current().adapters()


def resource_path(*parts):
    return current().resource_path(*parts)


def snapshot_dir():
    return current().snapshot_dir()

from .history import (
    append as _append_history,
    delete as history_delete,
    list_entries as history,
)
from .pricing import pricing  # noqa: F401  暴露给 RPC


# ---------- 迁移 ----------

def resume_command(tool: str, sid: str, cwd: str) -> dict:
    return adapter(tool).lifecycle.resume_descriptor(sid, cwd)


def tool_manifests() -> list[dict]:
    """插件 manifest 列表：前端与 Rust 的 Agent 定义单一事实源。"""
    return [adapter(name).describe() for name in adapters()]


# ---------- 范围截断 ----------

def _truncate_rounds(sess, max_turn: int):
    """裁剪根会话，并保留由保留消息明确发起的完整子树。

    ``max_turn`` 的范围只针对根会话的用户轮次。子会话不是根会话的
    独立轮次；只要其 ``AgentEdge.spawn_message_id`` 落在保留的根消息中，
    就将该任务及其完整后代作为该轮的结果保留。没有可追溯 spawn 消息的
    目录关联子会话无法证明属于截断范围，因此不迁移。
    """
    kept, turn = [], 0
    for m in sess.messages:
        if m.role == "user":
            turn += 1
        if turn > max_turn:
            break
        kept.append(m)
    dropped = len(sess.messages) - len(kept)
    if dropped:
        sess.lose("migration.truncated", max_turn=max_turn, dropped=dropped)
    sess.messages = kept
    kept_ids = {m.source_id for m in kept if m.source_id}
    children_by_id = {}
    for child in sess.children:
        children_by_id.setdefault(child.source_id, child)
    # 仅以落在已保留根消息中的原始 spawn 为准。不能把目录扫描得到、但
    # 没有 spawn_message_id 的子会话当作截断范围内的内容。
    edges, kept_children = [], set()
    for edge in sess.agent_edges:
        if (edge.child_session_id not in children_by_id or
                not edge.spawn_message_id or edge.spawn_message_id not in kept_ids or
                edge.child_session_id in kept_children):
            continue
        edges.append(edge)
        kept_children.add(edge.child_session_id)
    children = [child for child_id, child in children_by_id.items()
                if child_id in kept_children]
    removed = len(sess.children) - len(children)
    if removed:
        sess.lose("migration.children_not_migrated", count=removed)
    sess.children = children
    sess.agent_edges = edges
    return sess


def _migration_counts(sess) -> tuple[int, int]:
    """返回实际将写出的整棵树节点数与消息数。"""
    nodes = sum(1 for _ in sess.walk())
    return nodes, sess.message_count()


def _prepare_migration(src: str, dst: str, ref: str,
                       cwd: str | None = None,
                       max_turn: int | None = None,
                       probe_model: str | None = None, *, _session=None):
    sess = _session if _session is not None else _read_tree(src, ref)
    if max_turn:
        _truncate_rounds(sess, int(max_turn))
    target = adapter(dst).migration_target
    target_cwd = str(Path(cwd or sess.cwd or ".").resolve())
    stats = target.plan(sess)
    tree_count, message_count = _migration_counts(sess)
    edge_count = sum(len(node.agent_edges) for node in sess.walk())
    topology = {"nodes": tree_count,
                "edges": max(0, tree_count - 1),
                "agent_edges": edge_count,
                "preserved": True,
                "detail": "父子会话关系将按原拓扑写入" if tree_count > 1
                          else "普通单会话,无子会话拓扑"}
    base = {"src": src, "dst": dst, "source_id": sess.source_id,
            "title": sess.title, "cwd": target_cwd, "loss": stats,
            "tree_count": tree_count, "child_count": tree_count - 1,
            "topology": topology,
            "max_turn": max_turn, "msg_count": message_count,
            "root_msg_count": len(sess.messages),
            "probe_model": probe_model or None}
    return sess, target, target_cwd, base


def preview_migration(src: str, dst: str, ref: str,
                      cwd: str | None = None,
                      max_turn: int | None = None,
                      probe_model: str | None = None,
                      content_locale: str | None = None, *, _session=None) -> dict:
    sess, target, target_cwd, base = _prepare_migration(
        src, dst, ref, cwd, max_turn, probe_model, _session=_session,
    )
    with narration.content_locale(content_locale):
        preview = target.preview(sess, target_cwd)
    return {**base, "preview": preview}


def apply_migration(src: str, dst: str, ref: str,
                    cwd: str | None = None, probe: bool = False,
                    max_turn: int | None = None,
                    probe_model: str | None = None,
                    content_locale: str | None = None, *, _session=None) -> dict:
    sess, target, target_cwd, base = _prepare_migration(
        src, dst, ref, cwd, max_turn, probe_model, _session=_session,
    )

    with narration.content_locale(content_locale):
        sid, dest = target.write(sess, target_cwd)
    artifact_active = True
    try:
        # 写回阶段可能追加新的损耗(writer 分发时才知道)
        base["loss"] = target.plan(sess)
        result = {**base, "session_id": sid, "dest": str(dest),
                  "resume": resume_command(dst, sid, target_cwd)}

        # 静态结构验证始终执行:重读产物,校验节点数 / 父子边 / 拓扑,不调用模型
        ok, tree_detail = validate_written_tree(dst, sid, dest, _tree_shape(sess))
        validation = {"structure": {"ok": ok, "detail": tree_detail},
                      "runtime": {"status": "skipped"}}
        runtime_report = None
        if ok and probe:
            # 运行时探针只打在临时影子副本上,正式产物不接收探针消息
            with narration.content_locale(content_locale):
                runtime_report = _isolated_migrate_probe(
                    dst, sess, target_cwd, model=probe_model)
            validation["runtime"] = {**runtime_report,
                                     "model": probe_model or None}
            ok = runtime_report["status"] == "passed"
        result["validation"] = validation
        if probe or not ok:             # 历史记录/UI 消费的 probe 字段
            result["probe"] = runtime_report or {
                "status": "passed" if ok else "failed",
                "code": None if ok else "probe.structure_invalid",
                "params": {},
                "diagnostic": {"stdout": tree_detail, "stderr": "",
                               "truncated": False}}
            if probe:
                result["probe"]["model"] = probe_model or None
        if not ok:                      # 验收失败:删除产物,不留半成品
            _cleanup_artifact(dst, sid, dest)
            artifact_active = False
            result["rolled_back"] = True
        _append_history({**result, "time": int(time.time() * 1000)})
        return result
    except Exception:
        if artifact_active:
            _cleanup_artifact(dst, sid, dest)
        raise


def _isolated_migrate_probe(dst: str, sess, cwd: str,
                            model: str | None = None) -> dict:
    """把同一棵会话树再写一份影子副本,对影子 resume 探测,结束后清理。

    writer 每次生成全新随机 session id,影子与正式产物互不冲突;
    写入过程可能追加损耗记录,探测前保存、结束后还原。
    """
    saved_loss = [(node, list(node.loss)) for node in sess.walk()]
    shadow_sid = shadow_dest = None
    try:
        shadow_sid, shadow_dest = adapter(dst).migration_target.write(sess, cwd)
        rep = run_probe(dst, shadow_sid, cwd, model=model)
        rep.setdefault("isolation", {"kind": "shadow_copy", "id": shadow_sid,
                                     "cleaned": True})
        return rep
    finally:
        for node, loss in saved_loss:
            node.loss = loss
        if shadow_sid is not None:
            _cleanup_artifact(dst, shadow_sid, shadow_dest)


def run_probe(tool: str, sid: str, cwd: str,
              model: str | None = None) -> dict:
    try:
        return probe_mod.run_probe(
            tool, sid, adapter(tool).lifecycle.probe_cwd(cwd), model)
    except probe_mod.ProbeTimeout as error:
        return probe_mod.timeout_report(tool, error)


def _tree_shape(sess) -> tuple:
    return tuple(sorted((_tree_shape(child) for child in sess.children),
                        key=repr))


def validate_written_tree(tool: str, sid: str, dest,
                          expected_shape: tuple) -> tuple[bool, str]:
    try:
        ref = adapter(tool).lifecycle.validation_ref(sid, dest)
        restored = adapter(tool).browser.read(ref)
        nodes = list(restored.walk())
        ids = [node.source_id for node in nodes]
        edge_count = sum(len(node.children) for node in nodes)
        expected = 1 + sum(1 for _ in _shape_nodes(expected_shape))
        ok = (len(nodes) == expected and len(set(ids)) == expected and
              edge_count == max(0, expected - 1) and
              _tree_shape(restored) == expected_shape)
        detail = (f"树结构验收: 节点 {len(nodes)}/{expected}, "
                  f"父子边 {edge_count}/{max(0, expected - 1)}, "
                  f"层级拓扑 {'一致' if _tree_shape(restored) == expected_shape else '不一致'}")
        return ok, detail
    except Exception as error:
        return False, f"树结构验收失败: {error}"


def _shape_nodes(shape):
    for child in shape:
        yield child
        yield from _shape_nodes(child)


def _cleanup_artifact(dst: str, sid: str, dest):
    adapter(dst).lifecycle.cleanup(sid, dest)


# ---------- 会话元数据(重命名/置顶/归档/标签) ----------

from .session_meta import list_all as session_meta_list  # noqa: E402
from .session_meta import set_entry as _meta_set
from .session_meta import compare_and_set_entry as _meta_compare_and_set
from .session_meta import compare_and_set_entries as _meta_compare_and_set_entries

META_FIELDS = {
    "name", "pinned", "archived", "tags",
    "summary", "cluster_id", "cluster_name", "dead_candidate", "dead_reason",
}


def session_meta_set(sid: str, patch: dict) -> dict:
    return _meta_set(sid, {k: v for k, v in patch.items() if k in META_FIELDS})


def session_meta_compare_and_set(sid: str, expected: dict, patch: dict) -> dict:
    return _meta_compare_and_set(
        sid, expected, {k: v for k, v in patch.items() if k in META_FIELDS})


def session_meta_compare_and_set_many(changes: list[dict]) -> dict:
    return _meta_compare_and_set_entries([
        {
            "id": change["id"],
            "expected": change.get("expected", {}),
            "patch": {k: v for k, v in change.get("patch", {}).items()
                      if k in META_FIELDS},
        }
        for change in changes
    ])


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

# ---------- 会话编辑(可扩展原生后端) ----------

def _finish_mutation(tool, impl, result, doc, snapshot, probe):
    if not probe:
        return result
    rep = _probe_edited(tool, impl, doc, result)
    result["probe"] = rep
    if rep["status"] == "passed":
        return result
    if snapshot:
        impl.restore_snapshot(snapshot, doc)
        result.update(ok=False, error="隔离探针未通过,已自动还原快照")
    return result


def edit_capabilities(tool: str) -> dict:
    plugin = adapter(tool)
    editor = plugin.editor
    capabilities = editor.capabilities()
    operation_modes = {
        operation: ["inplace"]
        for operation, modes in capabilities.get("operation_modes", {}).items()
        if "inplace" in modes
    }
    return {
        "tool": tool,
        "operations": sorted(operation_modes),
        "inplace": bool(operation_modes),
        "operation_modes": operation_modes,
    }


def _probe_edited(tool: str, impl, doc, result: dict) -> dict:
    """各后端都只探测临时影子，不让 probe 消息污染交付会话。"""
    try:
        return adapter(tool).verifier.probe_edited(impl, doc, result)
    except probe_mod.ProbeTimeout as error:
        return probe_mod.timeout_report(tool, error)


# ---------- 环境 / 模型列表 ----------

from .environment import inspect as env


# ---------- CLI ----------


def version() -> dict:
    return {"version": current().version, "protocol": 2}


def health() -> dict:
    return {"status": "ok", **version()}


# 稳定门面仍从本模块导出，具体用例由职责更小的应用模块持有。
from .models import list_models  # noqa: E402,F811
from .scanning import scan  # noqa: E402,F811
from .sessions import read_tree as _read_tree, session_asset, show  # noqa: E402,F811
