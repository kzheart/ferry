#!/usr/bin/env python3
"""GUI 结构化接口层:全部函数返回可 JSON 序列化的 dict/list。

既可 import(gui/server.py 直接调用),也可命令行调试:
    python3 -m engine.api scan
    python3 -m engine.api show claude <sid>
    python3 -m engine.api migrate claude codex <ref> [--dry-run] [--probe] [--cwd DIR]
    python3 -m engine.api history / snapshots / env
"""
import json
import re
import time
from pathlib import Path

from . import verification as probe_mod
from .ports import current


def adapter(tool):
    return current().adapter(tool)


def adapters():
    return current().adapters()


def resource_path(*parts):
    return current().resource_path(*parts)


def snapshot_dir():
    return current().snapshot_dir

from .history import append as _append_history, list_entries as history

# 各目标已实现原生映射的规范操作
NATIVE_OPS = {"claude", "codex", "opencode"}


# ---------- 迁移 ----------

def resume_command(tool: str, sid: str, cwd: str) -> dict:
    return adapter(tool).resume_descriptor(sid, cwd)


def _loss_stats(sess, dst: str) -> dict:
    """预演:统计原生映射/降级/丢弃(与 writer 的分发逻辑一致)。"""
    native = degrade = 0
    details = []
    dropped = []
    for node in sess.walk():
        dropped.extend(node.loss)
        for m in node.messages:
            for b in m.blocks:
                if b.kind == "text":
                    native += 1
                elif b.kind == "tool":
                    if b.tool.op:
                        native += 1
                    else:
                        degrade += 1
                        details.append(
                            f"工具 {b.tool.name} 将降级为叙述文本")
    return {"native": native, "degrade": degrade, "drop": len(dropped),
            "degrade_details": details, "drop_details": dropped}


# ---------- 范围截断 ----------

def _truncate_rounds(sess, max_turn: int):
    """仅保留到第 max_turn 轮(用户消息计轮),含该轮的全部后续回应。"""
    kept, turn = [], 0
    for m in sess.messages:
        if m.role == "user":
            turn += 1
        if turn > max_turn:
            break
        kept.append(m)
    dropped = len(sess.messages) - len(kept)
    if dropped:
        sess.loss.append(f"按迁移范围截断: 丢弃第 {max_turn} 轮之后的 {dropped} 条消息")
    sess.messages = kept
    kept_ids = {m.source_id for m in kept if m.source_id}
    edges = [e for e in sess.agent_edges
             if e.spawn_message_id is None or e.spawn_message_id in kept_ids]
    kept_children = {e.child_session_id for e in edges}
    removed = [c for c in sess.children
               if sess.agent_edges and c.source_id not in kept_children]
    if removed:
        sess.loss.append(f"截断范围外的 {len(removed)} 个子会话未迁移")
        sess.children = [c for c in sess.children if c.source_id in kept_children]
        sess.agent_edges = edges
    return sess


def migrate(src: str, dst: str, ref: str, cwd: str | None = None,
            dry_run: bool = False, probe: bool = False,
            max_turn: int | None = None,
            probe_model: str | None = None) -> dict:
    sess = _read_tree(src, ref)
    if max_turn:
        _truncate_rounds(sess, int(max_turn))
    target_cwd = str(Path(cwd or sess.cwd or ".").resolve())
    stats = _loss_stats(sess, dst)
    tree_count = sum(1 for _ in sess.walk())
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
            "max_turn": max_turn, "msg_count": len(sess.messages),
            "probe_model": probe_model or None}
    if dry_run:
        return {**base, "dry_run": True}

    sid, dest = adapter(dst).writer(sess, cwd=target_cwd)
    # 写回阶段可能追加新的损耗(writer 分发时才知道)
    base["loss"] = _loss_stats(sess, dst)
    result = {**base, "session_id": sid, "dest": str(dest),
              "resume": resume_command(dst, sid, target_cwd)}

    # 静态结构验证始终执行:重读产物,校验节点数 / 父子边 / 拓扑,不调用模型
    ok, tree_detail = validate_written_tree(dst, sid, dest, _tree_shape(sess))
    validation = {"structure": {"ok": ok, "detail": tree_detail},
                  "runtime": {"status": "skipped"}}
    detail = tree_detail
    if ok and probe:
        # 运行时探针只打在临时影子副本上,正式产物不接收探针消息
        ok, runtime_detail = _isolated_migrate_probe(
            dst, sess, target_cwd, model=probe_model)
        validation["runtime"] = {"status": "passed" if ok else "failed",
                                 "detail": runtime_detail,
                                 "model": probe_model or None}
        detail = f"{tree_detail}\n{runtime_detail}"
    result["validation"] = validation
    if probe or not ok:                 # 兼容历史记录/UI 的 probe 字段
        result["probe"] = {"ok": ok, "detail": detail,
                           "model": (probe_model or None) if probe else None}
    if not ok:                          # 验收失败:删除产物,不留半成品
        _cleanup_artifact(dst, sid, dest)
        result["rolled_back"] = True
    _append_history({**result, "time": int(time.time() * 1000)})
    return result


def _isolated_migrate_probe(dst: str, sess, cwd: str,
                            model: str | None = None) -> tuple[bool, str]:
    """把同一棵会话树再写一份影子副本,对影子 resume 探测,结束后清理。

    writer 每次生成全新随机 session id,影子与正式产物互不冲突;
    写入过程可能追加损耗记录,探测前保存、结束后还原。
    """
    saved_loss = [(node, list(node.loss)) for node in sess.walk()]
    shadow_sid, shadow_dest = adapter(dst).writer(sess, cwd=cwd)
    for node, loss in saved_loss:
        node.loss = loss
    try:
        ok, detail = run_probe(dst, shadow_sid, cwd, model=model)
        return ok, f"(影子副本 {shadow_sid} 已探测并清理)\n{detail}"
    finally:
        _cleanup_artifact(dst, shadow_sid, shadow_dest)


def run_probe(tool: str, sid: str, cwd: str,
              model: str | None = None) -> tuple[bool, str]:
    try:
        ok, detail = probe_mod.run_probe(
            tool, sid, cwd if tool != "codex" else None, model)
    except probe_mod.ProbeTimeout as error:
        return False, str(error)
    if len(detail) > 12000:
        detail = detail[:12000] + f"\n…(截断,共 {len(detail)} 字符)"
    return ok, detail


def _tree_shape(sess) -> tuple:
    return tuple(sorted((_tree_shape(child) for child in sess.children),
                        key=repr))


def validate_written_tree(tool: str, sid: str, dest,
                          expected_shape: tuple) -> tuple[bool, str]:
    try:
        ref = adapter(tool).validation_ref(sid, dest)
        restored = adapter(tool).reader(ref)
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
    adapter(dst).cleanup(sid, dest)


def handoff(src: str, ref: str, dst: str, cwd: str | None = None) -> dict:
    """降级迁移:把会话渲染成上下文摘要文档 + 目标工具的开始命令。

    原生迁移不可行时的兜底(F22)。摘要是确定性浓缩,不逐轮还原。
    """
    impl = adapter(src)
    path = impl.resolve_ref(ref)
    sess = impl.reader(path)
    target_cwd = str(Path(cwd or sess.cwd or ".").resolve())
    lines = [f"# 会话接力摘要(来自 {src})",
             f"- 源会话: {sess.source_id}",
             f"- 工作目录: {target_cwd}", "",
             "以下是此前对话的浓缩记录。请把它当作已经发生的上下文,"
             "直接从这里继续工作,不要重做已完成的步骤。", ""]
    turn = 0
    for m in sess.messages:
        if m.role == "user":
            turn += 1
            lines.append(f"## 第 {turn} 轮")
        who = "用户" if m.role == "user" else "助手"
        for b in m.blocks:
            if b.kind == "text" and b.text.strip():
                lines.append(f"**{who}**: {b.text[:800]}")
            elif b.kind == "tool":
                t = b.tool
                inp = json.dumps(t.input, ensure_ascii=False)[:200] \
                    if isinstance(t.input, dict) else str(t.input)[:200]
                out_clip = (t.output or "").strip()[:300]
                lines.append(f"- 工具 `{t.name}` {inp}\n  结果: {out_clip}")
        lines.append("")
    doc_dir = Path.home() / ".resume-harness" / "handoff"
    doc_dir.mkdir(parents=True, exist_ok=True)
    doc = doc_dir / f"{sess.source_id}.md"
    doc.write_text("\n".join(lines))
    return {"doc": str(doc), "preview": "\n".join(lines)[:3000],
            "command": {"tool": dst, "cwd": target_cwd,
                        "handoff_doc": str(doc)}}


# ---------- 会话元数据(重命名/置顶/归档/标签) ----------

from .session_meta import list_all as session_meta_list  # noqa: E402
from .session_meta import set_entry as _meta_set

META_FIELDS = {"name", "pinned", "archived", "tags"}


def session_meta_set(sid: str, patch: dict) -> dict:
    return _meta_set(sid, {k: v for k, v in patch.items() if k in META_FIELDS})


# ---------- 会话生命周期 ----------

def _backup_file(path: Path, tool: str, reason: str, extra: dict | None = None) -> Path:
    d = snapshot_dir()
    d.mkdir(parents=True, exist_ok=True)
    import shutil
    dest = d / f"{path.stem}-{time.time_ns()}.jsonl"
    shutil.copy(path, dest)
    dest.with_suffix(".meta.json").write_text(json.dumps(
        {"reason": reason, "tool": tool, "source": str(path), **(extra or {})},
        ensure_ascii=False))
    return dest


def session_snapshot(tool: str, ref: str) -> dict:
    """手动为会话创建一份快照,不改动会话本身。"""
    impl = adapter(tool).editor
    doc = impl.load(ref)
    snap = impl.snapshot(doc, reason="手动快照")
    return {"ok": True, "snapshot": str(snap)}


def session_delete(tool: str, ref: str) -> dict:
    """删除会话前先落快照(回收站语义);文件型工具可通过 session_undelete 撤销。"""
    import shutil
    impl = adapter(tool)
    doc = impl.editor.load(ref)
    if tool == "opencode":
        snap = impl.editor.snapshot(doc, reason="删除前自动")
        impl.cleanup(ref, None)
        return {"ok": True, "snapshot": str(snap), "undoable": False}

    path = doc.handle if isinstance(doc.handle, Path) else Path(impl.resolve_ref(ref))
    children = []
    closure = getattr(doc, "context", None)
    if closure is not None and hasattr(closure, "nodes"):
        for node in closure.nodes.values():
            child = Path(node.path)
            if child != path and child.exists():
                child_snap = _backup_file(child, tool, "删除前自动")
                children.append({"snapshot": str(child_snap), "source": str(child)})
                child.unlink()
    snap = _backup_file(path, tool, "删除前自动",
                        {"children": children} if children else None)
    sidecar = path.with_suffix("")
    if tool == "claude" and sidecar.is_dir():
        shutil.move(str(sidecar), str(snap.with_suffix("")))
    path.unlink()
    return {"ok": True, "snapshot": str(snap), "undoable": True,
            "children": len(children)}


def session_undelete(snapshot: str) -> dict:
    """把「删除前自动」快照写回原路径,恢复整棵会话。"""
    import shutil
    snap = Path(snapshot)
    if snap.parent != snapshot_dir():
        return {"ok": False, "error": "只允许从快照目录恢复"}
    try:
        meta = json.loads(snap.with_suffix(".meta.json").read_text())
    except (OSError, json.JSONDecodeError):
        return {"ok": False, "error": "快照缺少元数据,无法撤销"}
    source = meta.get("source")
    if not source or not str(source).startswith("/"):
        return {"ok": False, "error": "该快照没有可恢复的源路径"}
    target = Path(source)
    if target.exists():
        return {"ok": False, "error": "源会话仍存在,未覆盖"}
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy(snap, target)
    sidecar = snap.with_suffix("")
    if sidecar.is_dir():
        shutil.move(str(sidecar), str(target.with_suffix("")))
    restored = 1
    for child in meta.get("children", []):
        child_snap, child_src = Path(child["snapshot"]), Path(child["source"])
        if child_snap.exists() and not child_src.exists():
            child_src.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(child_snap, child_src)
            restored += 1
    return {"ok": True, "restored": restored, "target": str(target)}


# ---------- 迁移历史 / 快照 ----------

def snapshots() -> list[dict]:
    d = snapshot_dir()
    if not d.exists():
        return []
    out = []
    for f in sorted(d.glob("*.jsonl"), reverse=True):
        m = re.match(r"(.+)-(\d+)$", f.stem)
        if not m:
            continue
        meta = {}
        try:
            meta = json.loads(f.with_suffix(".meta.json").read_text())
        except (OSError, json.JSONDecodeError):
            pass            # 旧快照没有 sidecar,用默认值
        raw_time = int(m.group(2))
        snapshot_time = raw_time // 1_000_000 if raw_time > 10**15 else raw_time * 1000
        out.append({"session": m.group(1), "time": snapshot_time,
                    "size": f.stat().st_size, "path": str(f),
                    "reason": meta.get("reason") or "会话编辑前自动",
                    "tool": meta.get("tool") or "claude",
                    "source": meta.get("source") or m.group(1)})
    return out


def snapshot_restore(session_id: str, run_probe_after: bool = False,
                     tool: str = "claude") -> dict:
    impl = adapter(tool).editor
    try:
        doc = impl.load(session_id)
    except (SystemExit, Exception):
        # 源会话文件已不存在(多半是被删除):直接把最新快照写回原路径
        stem = Path(session_id).stem
        cands = sorted(snapshot_dir().glob(f"{stem}-*.jsonl"))
        if cands:
            return session_undelete(str(cands[-1]))
        raise
    path = doc.handle if isinstance(doc.handle, Path) else None
    stem = path.stem if path else str(doc.ref)
    cands = sorted(snapshot_dir().glob(f"{stem}-*.jsonl"))
    if not cands:
        return {"ok": False, "error": "没有该会话的快照"}
    if path is None:
        guard = impl.snapshot(doc, reason="还原前保护")
        impl.restore_snapshot(cands[-1], doc)
        result = {"ok": True, "from": str(cands[-1]), "guard": str(guard)}
        if run_probe_after:
            restored = impl.load(session_id)
            shadow = impl.save_copy(restored)
            try:
                cwd = restored.data.get("info", {}).get("directory") or "."
                ok, detail = run_probe(tool, shadow["session_id"], cwd)
            finally:
                impl.discard(shadow)
            result["probe"] = {"ok": ok, "detail": detail, "isolated": True}
            if not ok:
                impl.restore_snapshot(guard, restored)
                result.update(ok=False, error="还原后隔离探针未通过,已保持现状")
        return result
    import shutil
    cur = path.read_bytes()               # 保住现状,探针失败时回退
    guard = impl.snapshot(doc, reason="还原前保护")      # UI 承诺的保护快照
    shutil.copy(cands[-1], path)
    result = {"ok": True, "from": str(cands[-1]), "guard": str(guard)}
    if run_probe_after:
        restored = impl.load(str(path))
        saved = {"session_id": session_id, "saved_as": str(path)}
        closure = getattr(restored, "context", None)
        if closure is not None and hasattr(closure, "nodes"):
            saved["published_paths"] = [str(node.path)
                                        for node in closure.nodes.values()]
        ok, detail = _probe_edited(tool, impl, restored, saved)
        result["probe"] = {"ok": ok, "detail": detail, "isolated": True}
        if not ok:
            path.write_bytes(cur)
            result.update(ok=False, error="还原后隔离探针未通过,已保持现状")
    return result


def snapshot_delete(path: str) -> dict:
    p = Path(path)
    if p.parent != snapshot_dir():
        return {"ok": False, "error": "只允许删除快照目录内的文件"}
    p.unlink(missing_ok=True)
    p.with_suffix(".meta.json").unlink(missing_ok=True)
    return {"ok": True}


# ---------- 会话编辑(可扩展原生后端) ----------

def _finish_mutation(tool, impl, result, doc, snapshot, probe, save_as):
    if not probe:
        return result
    ok, detail = _probe_edited(tool, impl, doc, result)
    result["probe"] = {"ok": ok, "detail": detail, "isolated": True}
    if ok:
        return result
    if save_as:
        impl.discard(result)
        result.update(ok=False, error="隔离探针未通过,已删除新副本,原会话未受影响")
    elif snapshot:
        impl.restore_snapshot(snapshot, doc)
        result.update(ok=False, error="隔离探针未通过,已自动还原快照")
    return result


def authoring_capabilities(tool: str) -> dict:
    from .authoring import capabilities
    return capabilities(adapter(tool).authoring)


def authoring_preview(ref: str, turn: int | str, reply: dict,
                      tool: str = "claude") -> dict:
    from .authoring import preview
    impl = adapter(tool)
    return preview(impl.editor, impl.authoring, ref, turn, reply)


def authoring_apply(ref: str, turn: int | str, reply: dict, probe: bool = False,
                    save_as: bool = False, tool: str = "claude",
                    revision: str | None = None) -> dict:
    from .authoring import apply
    impl = adapter(tool)
    result, doc, snapshot = apply(
        impl.editor, impl.authoring, ref, turn, reply, save_as, revision)
    return _finish_mutation(
        tool, impl.editor, result, doc, snapshot, probe, save_as)

def edit_capabilities(tool: str) -> dict:
    return adapter(tool).editor.capabilities()


def edit_preview(ref: str, ops: list[dict], tool: str = "claude") -> dict:
    """在内存中施加操作,返回前后统计与摘要,不落盘。"""
    from .editing import preview
    return preview(adapter(tool).editor, ref, ops)


def edit_apply(ref: str, ops: list[dict], probe: bool = False,
               save_as: bool = False, tool: str = "claude") -> dict:
    from .editing import apply
    impl = adapter(tool).editor
    result, doc, snapshot = apply(impl, ref, ops, save_as=save_as)
    return _finish_mutation(tool, impl, result, doc, snapshot, probe, save_as)


def _probe_edited(tool: str, impl, doc, result: dict) -> tuple[bool, str]:
    """各后端都只探测临时影子，不让 probe 消息污染交付会话。"""
    try:
        return adapter(tool).probe_edited(impl, doc, result)
    except probe_mod.ProbeTimeout as error:
        return False, str(error)


# ---------- 环境 / 模型列表 ----------

from .environment import inspect as env


# ---------- CLI ----------


def version() -> dict:
    return {"version": current().version, "protocol": 1}


def health() -> dict:
    return {"status": "ok", **version()}


# 稳定门面仍从本模块导出，具体用例由职责更小的应用模块持有。
from .models import list_models  # noqa: E402,F811
from .scanning import scan  # noqa: E402,F811
from .sessions import read_tree as _read_tree, show  # noqa: E402,F811
