#!/usr/bin/env python3
"""GUI 结构化接口层:全部函数返回可 JSON 序列化的 dict/list。

既可 import(gui/server.py 直接调用),也可命令行调试:
    python3 -m engine.api scan
    python3 -m engine.api show claude <sid>
    python3 -m engine.api migrate claude codex <ref> [--dry-run] [--cwd DIR]
    python3 -m engine.api history / snapshots / env
"""
import glob
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

from . import convert, edit as edit_mod
from .reader_codex import session_id as codex_session_id

REPO = Path(__file__).resolve().parent.parent
HISTORY = Path.home() / ".resume-harness" / "history.jsonl"
OPENCODE_DB = Path.home() / ".local/share/opencode/opencode.db"

# 各目标已实现原生映射的规范操作(与 spec/mapping/tools.yaml 一致)
NATIVE_OPS = {"claude", "codex", "opencode"}


# ---------- 会话扫描 ----------

SCAN_CACHE = Path.home() / ".resume-harness" / "scan-cache.json"
SCAN_CACHE_VERSION = 5
_cache: dict | None = None


def _cache_get(path: Path, st) -> dict | None:
    global _cache
    if _cache is None:
        try:
            _cache = json.loads(SCAN_CACHE.read_text())
        except (OSError, json.JSONDecodeError):
            _cache = {}
    hit = _cache.get(str(path))
    if hit and hit.get("version") == SCAN_CACHE_VERSION and \
            hit["mtime"] == st.st_mtime_ns and hit["size"] == st.st_size:
        return hit["meta"]
    return None


def _cache_put(path: Path, st, meta: dict):
    _cache[str(path)] = {"version": SCAN_CACHE_VERSION,
                         "mtime": st.st_mtime_ns, "size": st.st_size,
                         "meta": meta}


def _cache_flush():
    if _cache is not None:
        SCAN_CACHE.parent.mkdir(parents=True, exist_ok=True)
        tmp = SCAN_CACHE.with_name(f"{SCAN_CACHE.name}.{os.getpid()}.tmp")
        tmp.write_text(json.dumps(_cache))
        os.replace(tmp, SCAN_CACHE)


def _clip(s: str, n: int = 80) -> str:
    s = " ".join(s.split())
    return s[:n] + ("…" if len(s) > n else "")


def _session_roots(rows: list[dict]) -> list[dict]:
    """把扫描得到的扁平节点挂成树，并把子树统计汇总到每个节点。"""
    nodes = {}
    for source in rows:
        node = dict(source)
        node["children"] = []
        node["own_count"] = source.get("own_count", source.get("count", 0))
        node["own_size"] = source.get("own_size", source.get("size", 0))
        node["own_updated"] = source.get(
            "own_updated", source.get("updated", 0))
        nodes[node["id"]] = node

    roots = []
    for node in nodes.values():
        parent = nodes.get(node.get("parent_id"))
        cursor = parent
        seen = {node["id"]}
        while cursor is not None and cursor["id"] not in seen:
            seen.add(cursor["id"])
            cursor = nodes.get(cursor.get("parent_id"))
        if cursor is not None:
            parent = None
        if parent is not None and parent is not node:
            parent["children"].append(node)
        else:
            node["parent_id"] = None
            roots.append(node)

    visiting = set()

    def summarize(node, root_id):
        if node["id"] in visiting:  # 格式损坏时切断环，仍让会话可见。
            node["children"] = []
        visiting.add(node["id"])
        node["root_id"] = root_id
        for child in node["children"]:
            summarize(child, root_id)
        visiting.discard(node["id"])
        node["children"].sort(
            key=lambda child: child.get("updated", 0), reverse=True)
        node["child_count"] = len(node["children"])
        node["tree_count"] = 1 + sum(
            child["tree_count"] for child in node["children"])
        node["count"] = node["own_count"] + sum(
            child["count"] for child in node["children"])
        node["size"] = node["own_size"] + sum(
            child["size"] for child in node["children"])
        node["updated"] = max(
            [node["own_updated"],
             *(child["updated"] for child in node["children"])])

    for root in roots:
        summarize(root, root["id"])
    roots.sort(key=lambda node: node["updated"], reverse=True)
    return roots


def _scan_claude() -> list[dict]:
    out = []
    base = Path(os.path.expanduser("~/.claude/projects"))
    for f in glob.glob(str(base / "**/*.jsonl"), recursive=True):
        p = Path(f)
        st = p.stat()
        cached = _cache_get(p, st)
        if cached is not None:
            if cached:
                out.append(cached)
            continue
        cwd, title, count = "", "", 0
        try:
            for line in p.read_text().splitlines():
                if not line.strip():
                    continue
                r = json.loads(line)
                t = r.get("type")
                if t in ("user", "assistant"):
                    count += 1
                    if not cwd:
                        cwd = r.get("cwd", "")
                    if not title and t == "user":
                        c = (r.get("message") or {}).get("content")
                        if isinstance(c, str) and c.strip() and \
                                not c.strip().startswith("<"):
                            title = _clip(c)
                elif t == "ai-title":
                    title = r.get("title", "") or title
        except (json.JSONDecodeError, OSError):
            continue
        try:
            rel = p.relative_to(base)
        except ValueError:
            rel = p
        is_child = len(rel.parts) > 2
        root_id = rel.parts[1] if is_child else p.stem
        meta = {} if count == 0 else \
            {"tool": "claude", "id": p.stem, "title": title,
             "dir": cwd, "updated": int(st.st_mtime * 1000),
             "count": count, "size": st.st_size, "path": str(p),
             "parent_id": root_id if is_child else None,
             "root_id": root_id}
        _cache_put(p, st, meta)
        if meta:
            out.append(meta)
    return _session_roots(out)


def _scan_codex() -> list[dict]:
    out = []
    for f in glob.glob(os.path.expanduser(
            "~/.codex/sessions/*/*/*/rollout-*.jsonl")):
        p = Path(f)
        st = p.stat()
        cached = _cache_get(p, st)
        if cached is not None:
            if cached:
                out.append(cached)
            continue
        sid, cwd, title, count, parent_id = p.stem, "", "", 0, None
        root_id, agent_id, agent_path, agent_type = None, None, None, None
        has_meta = False
        try:
            for line in p.read_text().splitlines():
                if not line.strip():
                    continue
                r = json.loads(line)
                if r.get("type") == "session_meta" and not has_meta:
                    payload = r["payload"]
                    sid = codex_session_id(payload, sid)
                    cwd = payload.get("cwd", "")
                    source = payload.get("source") or payload.get("thread_source")
                    source = source if isinstance(source, dict) else {}
                    subagent = source.get("subagent")
                    subagent = subagent if isinstance(subagent, dict) else {}
                    spawn = subagent.get("thread_spawn")
                    spawn = spawn if isinstance(spawn, dict) else {}
                    root_id = payload.get("session_id") or sid
                    parent_id = (payload.get("parent_thread_id") or
                                 spawn.get("parent_thread_id") or
                                 subagent.get("parent_thread_id"))
                    if not parent_id and root_id != sid:
                        parent_id = root_id
                    agent_id = (subagent.get("agent_id") or
                                spawn.get("agent_id") or payload.get("agent_id"))
                    agent_path = (subagent.get("agent_path") or
                                  spawn.get("agent_path") or
                                  payload.get("agent_path"))
                    agent_type = (subagent.get("agent_type") or
                                  spawn.get("agent_type") or
                                  payload.get("agent_type"))
                    has_meta = True
                elif r.get("type") == "response_item":
                    pl = r["payload"]
                    if pl.get("type") == "message":
                        count += 1
                        if not title and pl.get("role") == "user":
                            txt = "\n".join(c.get("text", "")
                                            for c in pl.get("content", []))
                            if txt.strip() and not txt.strip()[0] in "<[":
                                title = _clip(txt)
        except (json.JSONDecodeError, OSError):
            continue
        meta = {} if count == 0 else \
            {"tool": "codex", "id": sid, "title": title,
             "dir": cwd, "updated": int(st.st_mtime * 1000),
             "count": count, "size": st.st_size, "path": str(p),
             "parent_id": parent_id, "root_id": root_id or sid,
             "agent_id": agent_id, "agent_path": agent_path,
             "agent_type": agent_type}
        _cache_put(p, st, meta)
        if meta:
            out.append(meta)
    return _session_roots(out)


def _scan_opencode() -> list[dict]:
    if not OPENCODE_DB.exists():
        return []
    out = []
    uri = f"file:{OPENCODE_DB}?mode=ro"
    with sqlite3.connect(uri, uri=True, timeout=5) as db:
        counts = dict(db.execute(
            "SELECT session_id, COUNT(*) FROM message GROUP BY session_id"))
        rows = db.execute(
            "SELECT id, title, directory, time_updated, parent_id "
            "FROM session").fetchall()
    for sid, title, d, upd, parent_id in rows:
        n = counts.get(sid, 0)
        out.append({"tool": "opencode", "id": sid, "title": title or "",
                    "dir": d or "", "updated": upd or 0,
                    "count": n, "size": 0, "path": "",
                    "parent_id": parent_id})
    return [root for root in _session_roots(out) if root["count"]]


SOURCE_PATHS = {"claude": "~/.claude/projects",
                "codex": "~/.codex/sessions",
                "opencode": "~/.local/share/opencode"}


def scan() -> dict:
    """三家会话列表 + 各家扫描状态(未安装/出错如实报告)。"""
    tools = {}
    sessions = []
    for name, fn in (("claude", _scan_claude), ("codex", _scan_codex),
                     ("opencode", _scan_opencode)):
        try:
            rows = fn()
            sessions.extend(rows)
            tools[name] = {"ok": True, "count": len(rows),
                           "path": SOURCE_PATHS[name]}
        except Exception as e:
            tools[name] = {"ok": False, "error": str(e)[:200],
                           "path": SOURCE_PATHS[name]}
    _cache_flush()
    sessions.sort(key=lambda s: s["updated"], reverse=True)
    return {"tools": tools, "sessions": sessions}


# ---------- 会话详情 ----------

def _walk_meta(nodes):
    for node in nodes:
        yield node
        yield from _walk_meta(node.get("children", []))


def _read_tree(tool: str, ref: str):
    path = convert.resolve_ref(tool, ref)
    sess = convert.READERS[tool](path)
    if sess.children:
        return sess
    scanner = {"claude": _scan_claude, "codex": _scan_codex,
               "opencode": _scan_opencode}[tool]
    roots = scanner()
    target = next((node for node in _walk_meta(roots)
                   if node["id"] == sess.source_id
                   or (node.get("path") and node["path"] == str(path))), None)
    if target is None:
        return sess

    metadata = {node["id"]: node for node in _walk_meta(roots)}

    def attach(current, meta, root_id):
        current.source_id = meta["id"]
        current.root_id = root_id
        current.parent_id = meta.get("parent_id")
        current.title = current.title or meta.get("title", "")
        current.cwd = current.cwd or meta.get("dir", "")
        existing = {child.source_id: child for child in current.children}
        children = []
        for child_meta in meta.get("children", []):
            child = existing.get(child_meta["id"])
            if child is None:
                child_ref = child_meta.get("path") or child_meta["id"]
                child = convert.READERS[tool](child_ref)
            attach(child, child_meta, root_id)
            children.append(child)
        current.children = children

    attach(sess, target, target.get("root_id") or target["id"])
    return sess


def _message_json(messages) -> list[dict]:
    msgs = []
    for i, m in enumerate(messages):
        blocks = []
        for b in m.blocks:
            if b.kind == "text":
                blocks.append({"kind": "text", "text": b.text,
                               "size": len(b.text)})
            elif b.kind == "tool":
                t = b.tool
                inp = t.input if isinstance(t.input, dict) else str(t.input)
                blocks.append({"kind": "tool", "name": t.name, "op": t.op,
                               "input": inp, "output": t.output,
                               "size": len(t.output or "")})
        entry = {"index": i, "role": m.role, "blocks": blocks}
        if m.raw and isinstance(m.raw[0], dict) and m.raw[0].get("uuid"):
            entry["uuid"] = m.raw[0]["uuid"]
        msgs.append(entry)
    return msgs


def _session_json(sess) -> dict:
    children = [_session_json(child) for child in sess.children]
    edges = []
    for edge in sess.agent_edges:
        edges.append({name: getattr(edge, name) for name in (
            "parent_session_id", "child_session_id", "source_call_id",
            "spawn_message_id", "result_message_id", "agent_id", "agent_path",
            "agent_type", "prompt", "status", "meta")})
    messages = _message_json(sess.messages)
    return {"tool": sess.source_tool, "id": sess.source_id,
            "title": sess.title, "dir": sess.cwd,
            "root_id": sess.root_id or sess.source_id,
            "parent_id": sess.parent_id, "agent_id": sess.agent_id,
            "agent_path": sess.agent_path, "agent_type": sess.agent_type,
            "count": len(messages), "child_count": len(children),
            "tree_count": 1 + sum(child["tree_count"] for child in children),
            "loss": list(sess.loss), "messages": messages,
            "children": children, "agent_edges": edges}


def show(tool: str, ref: str) -> dict:
    return _session_json(_read_tree(tool, ref))


# ---------- 迁移 ----------

def resume_command(tool: str, sid: str, cwd: str) -> str:
    if tool == "claude":
        return f"cd {cwd} && claude --resume {sid}"
    if tool == "codex":
        return f"codex resume {sid}"
    return f"cd {cwd} && opencode -s {sid}"


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


# ---------- 敏感信息检测 / 脱敏 ----------

SENSITIVE_PATTERNS = [
    ("api_key", "疑似 API Key", re.compile(
        r"(?:sk-[A-Za-z0-9_-]{16,}|ghp_[A-Za-z0-9]{20,}|gho_[A-Za-z0-9]{20,}"
        r"|xox[bapors]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}"
        r"|AIza[0-9A-Za-z_-]{30,})")),
    ("bearer", "Bearer Token", re.compile(
        r"Bearer\s+[A-Za-z0-9._~+/-]{20,}=*")),
    ("email", "邮箱地址", re.compile(
        r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[A-Za-z]{2,}\b")),
]


def _mask_secret(kind: str, text: str) -> str:
    if kind == "email":
        name, _, domain = text.partition("@")
        return f"{name[:1]}***@{domain}"
    return f"{text[:6]}…[已脱敏]"


def _session_strings(sess):
    """产出树内所有可含敏感信息的字符串及其写回函数。"""
    for node in sess.walk():
        for m in node.messages:
            for b in m.blocks:
                if b.kind in ("text", "thinking") and b.text:
                    yield b.text, (lambda blk: (lambda v: setattr(blk, "text", v)))(b)
                elif b.kind == "tool" and b.tool:
                    t = b.tool
                    if t.output:
                        yield t.output, (lambda tc: (lambda v: setattr(tc, "output", v)))(t)
                    if isinstance(t.input, str) and t.input:
                        yield t.input, (lambda tc: (lambda v: setattr(tc, "input", v)))(t)
                    elif isinstance(t.input, dict):
                        for key, val in t.input.items():
                            if isinstance(val, str) and val:
                                yield val, (lambda d, k: (lambda v: d.__setitem__(k, v)))(t.input, key)


def _scan_sensitive(sess) -> dict:
    findings = {}
    for text, _setter in _session_strings(sess):
        for kind, label, pat in SENSITIVE_PATTERNS:
            for m in pat.finditer(text):
                f = findings.setdefault(kind, {"kind": kind, "label": label,
                                               "count": 0, "samples": []})
                f["count"] += 1
                masked = _mask_secret(kind, m.group(0))
                if masked not in f["samples"] and len(f["samples"]) < 3:
                    f["samples"].append(masked)
    out = sorted(findings.values(), key=lambda f: -f["count"])
    return {"findings": out, "total": sum(f["count"] for f in out)}


def _redact_session(sess) -> dict:
    """就地把树内敏感片段替换为脱敏占位,返回各类命中数。"""
    counts: dict[str, int] = {}
    for text, setter in _session_strings(sess):
        new = text
        for kind, _label, pat in SENSITIVE_PATTERNS:
            def repl(m, kind=kind):
                counts[kind] = counts.get(kind, 0) + 1
                return _mask_secret(kind, m.group(0))
            new = pat.sub(repl, new)
        if new != text:
            setter(new)
    return counts


def redact_scan(tool: str, ref: str) -> dict:
    return _scan_sensitive(_read_tree(tool, ref))


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


def _append_history(entry: dict):
    HISTORY.parent.mkdir(parents=True, exist_ok=True)
    with HISTORY.open("a") as f:
        f.write(json.dumps(entry, ensure_ascii=False) + "\n")


def migrate(src: str, dst: str, ref: str, cwd: str | None = None,
            dry_run: bool = False, probe: bool = True,
            redact: bool = False, max_turn: int | None = None) -> dict:
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
            "max_turn": max_turn, "msg_count": len(sess.messages)}
    if dry_run:
        return {**base, "dry_run": True,
                "sensitive": _scan_sensitive(sess)}

    if redact:
        base["redacted"] = _redact_session(sess)
    sid, dest = convert.WRITERS[dst](sess, cwd=target_cwd)
    # 写回阶段可能追加新的损耗(writer 分发时才知道)
    base["loss"] = _loss_stats(sess, dst)
    result = {**base, "session_id": sid, "dest": str(dest),
              "resume": resume_command(dst, sid, target_cwd)}

    if probe:
        ok, tree_detail = validate_written_tree(
            dst, sid, dest, _tree_shape(sess))
        if ok:
            ok, runtime_detail = run_probe(dst, sid, target_cwd)
            detail = f"{tree_detail}\n{runtime_detail}"
        else:
            detail = tree_detail
        result["probe"] = {"ok": ok, "detail": detail}
        if not ok:                      # 验收失败:删除产物,不留半成品
            _cleanup_artifact(dst, sid, dest)
            result["rolled_back"] = True
    _append_history({**result, "time": int(time.time() * 1000)})
    return result


def run_probe(tool: str, sid: str, cwd: str) -> tuple[bool, str]:
    cmd = [sys.executable, str(REPO / "harness/probe.py"), tool, sid]
    if tool != "codex":
        cmd += ["--dir", cwd]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=420)
    return r.returncode == 0, (r.stdout + r.stderr)[-400:]


def _tree_shape(sess) -> tuple:
    return tuple(sorted((_tree_shape(child) for child in sess.children),
                        key=repr))


def validate_written_tree(tool: str, sid: str, dest,
                          expected_shape: tuple) -> tuple[bool, str]:
    try:
        ref = sid if tool == "opencode" else str(dest)
        restored = convert.READERS[tool](ref)
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
    if dst == "claude":
        try:
            hits = glob.glob(os.path.expanduser(
                f"~/.claude/projects/*/{sid}.jsonl"))
            for h in hits:
                os.unlink(h)
                shutil.rmtree(Path(h).with_suffix(""), ignore_errors=True)
        except OSError:
            pass
    elif dst == "codex":
        for h in glob.glob(os.path.expanduser(
                "~/.codex/sessions/*/*/*/rollout-*.jsonl")):
            try:
                with open(h) as stream:
                    meta = next((json.loads(line).get("payload", {})
                                 for line in stream if line.strip() and
                                 json.loads(line).get("type") == "session_meta"), {})
                if meta.get("id") == sid or meta.get("session_id") == sid:
                    os.unlink(h)
            except (OSError, json.JSONDecodeError):
                continue
    elif dst == "opencode":
        try:
            tree = convert.READERS["opencode"](sid)
            for node in reversed(list(tree.walk())):
                subprocess.run(["opencode", "session", "delete", node.source_id],
                               capture_output=True, text=True, timeout=30)
        except Exception:
            pass


def handoff(src: str, ref: str, dst: str, cwd: str | None = None) -> dict:
    """降级迁移:把会话渲染成上下文摘要文档 + 目标工具的开始命令。

    原生迁移不可行时的兜底(F22)。摘要是确定性浓缩,不逐轮还原。
    """
    path = convert.resolve_ref(src, ref)
    sess = convert.READERS[src](path)
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
    starter = {"claude": f'cd {target_cwd} && claude "$(cat {doc})"',
               "codex": f'cd {target_cwd} && codex "$(cat {doc})"',
               "opencode": f'cd {target_cwd} && opencode run "$(cat {doc})"'}
    return {"doc": str(doc), "preview": "\n".join(lines)[:3000],
            "command": starter[dst]}


# ---------- 迁移历史 / 快照 ----------

def history() -> list[dict]:
    if not HISTORY.exists():
        return []
    rows = [json.loads(l) for l in HISTORY.read_text().splitlines()
            if l.strip()]
    return rows[::-1]


def snapshots() -> list[dict]:
    d = edit_mod.BACKUP_DIR
    if not d.exists():
        return []
    out = []
    for f in sorted(d.glob("*.jsonl"), reverse=True):
        m = re.match(r"(.+)-(\d+)$", f.stem)
        if not m:
            continue
        out.append({"session": m.group(1), "time": int(m.group(2)) * 1000,
                    "size": f.stat().st_size, "path": str(f)})
    return out


def snapshot_restore(session_id: str, run_probe_after: bool = True) -> dict:
    path = edit_mod.resolve(session_id)
    cands = sorted(edit_mod.BACKUP_DIR.glob(f"{path.stem}-*.jsonl"))
    if not cands:
        return {"ok": False, "error": "没有该会话的快照"}
    import shutil
    cur = path.read_bytes()               # 保住现状,探针失败时回退
    shutil.copy(cands[-1], path)
    result = {"ok": True, "from": str(cands[-1])}
    if run_probe_after:
        cwd = next((json.loads(l).get("cwd") for l in
                    path.read_text().splitlines() if l.strip()), None)
        ok, detail = run_probe("claude", path.stem, cwd or ".")
        result["probe"] = {"ok": ok, "detail": detail}
        if not ok:
            path.write_bytes(cur)
            result.update(ok=False, error="还原后探针未通过,已保持现状")
    return result


def snapshot_delete(path: str) -> dict:
    p = Path(path)
    if p.parent != edit_mod.BACKUP_DIR:
        return {"ok": False, "error": "只允许删除快照目录内的文件"}
    p.unlink(missing_ok=True)
    return {"ok": True}


# ---------- 会话编辑(claude 会话) ----------

class _Args:
    def __init__(self, **kw):
        self.__dict__.update(kw)


def edit_preview(ref: str, ops: list[dict]) -> dict:
    """在内存中施加操作,返回前后统计与摘要,不落盘。"""
    path = edit_mod.resolve(ref)
    records = edit_mod.load(path)
    before = {"count": len(records),
              "size": sum(len(json.dumps(r)) for r in records)}
    records, notes = _apply_ops(records, ops)
    edit_mod.check_invariants(records)
    after = {"count": len(records),
             "size": sum(len(json.dumps(r)) for r in records)}
    return {"before": before, "after": after, "notes": notes}


def edit_apply(ref: str, ops: list[dict], probe: bool = True,
               save_as: bool = False) -> dict:
    if save_as:
        return _edit_save_as(ref, ops, probe)
    path = edit_mod.resolve(ref)
    bak = edit_mod.backup(path)
    records = edit_mod.load(path)
    records, notes = _apply_ops(records, ops)
    edit_mod.check_invariants(records)
    edit_mod.save(path, records)
    cwd = next((r.get("cwd") for r in records if r.get("cwd")), ".")
    result = {"ok": True, "snapshot": str(bak), "notes": notes,
              "resume": resume_command("claude", path.stem, cwd)}
    if probe:
        ok, detail = run_probe("claude", path.stem, cwd)
        result["probe"] = {"ok": ok, "detail": detail}
        if not ok:                       # 自动还原快照
            import shutil
            shutil.copy(bak, path)
            result.update(ok=False, error="探针未通过,已自动还原快照")
    return result


def _edit_save_as(ref: str, ops: list[dict], probe: bool = True) -> dict:
    """另存为新会话:原会话保持不变,操作施加在同目录的新副本上。"""
    import uuid
    path = edit_mod.resolve(ref)
    records = edit_mod.load(path)
    records, notes = _apply_ops(records, ops)
    edit_mod.check_invariants(records)
    new_id = str(uuid.uuid4())
    for r in records:
        if "sessionId" in r:
            r["sessionId"] = new_id
    new_path = path.with_name(f"{new_id}.jsonl")
    edit_mod.save(new_path, records)
    cwd = next((r.get("cwd") for r in records if r.get("cwd")), ".")
    result = {"ok": True, "session_id": new_id, "saved_as": str(new_path),
              "notes": notes,
              "resume": resume_command("claude", new_id, cwd)}
    if probe:
        ok, detail = run_probe("claude", new_id, cwd)
        result["probe"] = {"ok": ok, "detail": detail}
        if not ok:                       # 副本验收失败:删除,不留半成品
            new_path.unlink(missing_ok=True)
            result.update(ok=False, error="探针未通过,已删除新副本,原会话未受影响")
    return result


def _apply_ops(records, ops):
    notes = []
    for op in ops:
        kind = op["op"]
        if kind == "delete-turn":
            records = edit_mod.op_delete_turn(records, _Args(turn=op["turn"]))
            notes.append(f"删除第 {op['turn']} 轮")
        elif kind == "truncate":
            records = edit_mod.op_truncate(
                records, _Args(threshold=op.get("threshold", 4096)))
            notes.append(f"裁剪超过 {op.get('threshold', 4096)} 字符的工具输出")
        elif kind == "rewrite":
            records = edit_mod.op_rewrite(
                records, _Args(uuid=op["uuid"], text=op["text"]))
            notes.append("改写 1 条消息")
        else:
            raise ValueError(f"未知操作: {kind}")
    return records, notes


# ---------- 环境 ----------

def env() -> dict:
    out = {}
    for tool in ("claude", "codex", "opencode"):
        info = {"installed": False, "version": None, "golden": None,
                "verified": False}
        try:
            r = subprocess.run([tool, "--version"], capture_output=True,
                               text=True, timeout=20)
            m = re.search(r"\d+\.\d+\.\d+", r.stdout + r.stderr)
            info["installed"] = r.returncode == 0
            info["version"] = m.group(0) if m else None
        except (FileNotFoundError, subprocess.TimeoutExpired):
            pass
        gdir = REPO / "golden" / tool
        if gdir.exists():
            versions = sorted(p.name for p in gdir.iterdir() if p.is_dir())
            info["golden"] = versions[-1] if versions else None
        info["verified"] = (info["installed"] and info["golden"]
                            and info["version"] == info["golden"])
        out[tool] = info
    return out


# ---------- CLI ----------

RPC_METHODS = {
    "scan": lambda p: scan(),
    "env": lambda p: env(),
    "history": lambda p: history(),
    "snapshots": lambda p: snapshots(),
    "show": lambda p: show(p["tool"], p["ref"]),
    "migrate": lambda p: migrate(p["src"], p["dst"], p["ref"],
                                 cwd=p.get("cwd"),
                                 dry_run=p.get("dry_run", False),
                                 probe=p.get("probe", True),
                                 redact=p.get("redact", False),
                                 max_turn=p.get("max_turn")),
    "redact_scan": lambda p: redact_scan(p["tool"], p["ref"]),
    "handoff": lambda p: handoff(p["src"], p["ref"], p["dst"],
                                 cwd=p.get("cwd")),
    "edit_preview": lambda p: edit_preview(p["ref"], p["ops"]),
    "edit_apply": lambda p: edit_apply(p["ref"], p["ops"],
                                       probe=p.get("probe", True),
                                       save_as=p.get("save_as", False)),
    "snapshot_restore": lambda p: snapshot_restore(p["session"]),
    "snapshot_delete": lambda p: snapshot_delete(p["path"]),
}


def rpc(request: str) -> dict:
    """统一入口:{"method": ..., "params": {...}} → 结果 dict(GUI 桥)。

    执行期间 stdout 重定向到 stderr:底层 edit 操作会 print 进度,
    不能让它污染桥接层要解析的 JSON 输出。
    """
    import contextlib
    req = json.loads(request)
    fn = RPC_METHODS.get(req.get("method"))
    if fn is None:
        return {"error": f"未知 method: {req.get('method')}"}
    try:
        with contextlib.redirect_stdout(sys.stderr):
            result = fn(req.get("params") or {})
        return {"ok": True, "result": result}
    except SystemExit as e:               # 底层 CLI 工具可能 sys.exit
        return {"ok": False, "error": str(e)[:500]}
    except Exception as e:
        return {"ok": False, "error": str(e)[:500]}


def main():
    args = sys.argv[1:]
    if not args:
        sys.exit(__doc__)
    cmd, rest = args[0], args[1:]
    if cmd == "rpc":
        print(json.dumps(rpc(rest[0] if rest else sys.stdin.read()),
                         ensure_ascii=False))
        return
    if cmd == "scan":
        r = scan()
    elif cmd == "show":
        r = show(rest[0], rest[1])
    elif cmd == "migrate":
        r = migrate(rest[0], rest[1], rest[2],
                    cwd=(rest[rest.index("--cwd") + 1]
                         if "--cwd" in rest else None),
                    dry_run="--dry-run" in rest,
                    probe="--no-probe" not in rest)
    elif cmd == "history":
        r = history()
    elif cmd == "snapshots":
        r = snapshots()
    elif cmd == "env":
        r = env()
    else:
        sys.exit(f"未知命令: {cmd}")
    print(json.dumps(r, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
