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
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

from . import convert, edit as edit_mod

REPO = Path(__file__).resolve().parent.parent
HISTORY = Path.home() / ".resume-harness" / "history.jsonl"
OPENCODE_DB = Path.home() / ".local/share/opencode/opencode.db"

# 各目标已实现原生映射的规范操作(与 spec/mapping/tools.yaml 一致)
NATIVE_OPS = {"claude", "codex", "opencode"}


# ---------- 会话扫描 ----------

SCAN_CACHE = Path.home() / ".resume-harness" / "scan-cache.json"
_cache: dict | None = None


def _cache_get(path: Path, st) -> dict | None:
    global _cache
    if _cache is None:
        try:
            _cache = json.loads(SCAN_CACHE.read_text())
        except (OSError, json.JSONDecodeError):
            _cache = {}
    hit = _cache.get(str(path))
    if hit and hit["mtime"] == st.st_mtime_ns and hit["size"] == st.st_size:
        return hit["meta"]
    return None


def _cache_put(path: Path, st, meta: dict):
    _cache[str(path)] = {"mtime": st.st_mtime_ns, "size": st.st_size,
                         "meta": meta}


def _cache_flush():
    if _cache is not None:
        SCAN_CACHE.parent.mkdir(parents=True, exist_ok=True)
        tmp = SCAN_CACHE.with_suffix(".tmp")
        tmp.write_text(json.dumps(_cache))
        tmp.rename(SCAN_CACHE)


def _clip(s: str, n: int = 80) -> str:
    s = " ".join(s.split())
    return s[:n] + ("…" if len(s) > n else "")


def _scan_claude() -> list[dict]:
    out = []
    for f in glob.glob(os.path.expanduser("~/.claude/projects/*/*.jsonl")):
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
                    if not title and t == "user" and not r.get("isSidechain"):
                        c = (r.get("message") or {}).get("content")
                        if isinstance(c, str) and c.strip() and \
                                not c.strip().startswith("<"):
                            title = _clip(c)
                elif t == "ai-title":
                    title = r.get("title", "") or title
        except (json.JSONDecodeError, OSError):
            continue
        meta = {} if count == 0 else \
            {"tool": "claude", "id": p.stem, "title": title,
             "dir": cwd, "updated": int(st.st_mtime * 1000),
             "count": count, "size": st.st_size, "path": str(p)}
        _cache_put(p, st, meta)
        if meta:
            out.append(meta)
    return out


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
        sid, cwd, title, count = p.stem, "", "", 0
        try:
            for line in p.read_text().splitlines():
                if not line.strip():
                    continue
                r = json.loads(line)
                if r.get("type") == "session_meta":
                    sid = r["payload"].get("session_id", sid)
                    cwd = r["payload"].get("cwd", "")
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
             "count": count, "size": st.st_size, "path": str(p)}
        _cache_put(p, st, meta)
        if meta:
            out.append(meta)
    return out


def _scan_opencode() -> list[dict]:
    if not OPENCODE_DB.exists():
        return []
    out = []
    uri = f"file:{OPENCODE_DB}?mode=ro"
    with sqlite3.connect(uri, uri=True, timeout=5) as db:
        counts = dict(db.execute(
            "SELECT session_id, COUNT(*) FROM message GROUP BY session_id"))
        rows = db.execute(
            "SELECT id, title, directory, time_updated FROM session "
            "WHERE parent_id IS NULL").fetchall()
    for sid, title, d, upd in rows:
        n = counts.get(sid, 0)
        if n == 0:
            continue
        out.append({"tool": "opencode", "id": sid, "title": title or "",
                    "dir": d or "", "updated": upd or 0,
                    "count": n, "size": 0, "path": ""})
    return out


def scan() -> dict:
    """三家会话列表 + 各家扫描状态(未安装/出错如实报告)。"""
    tools = {}
    sessions = []
    for name, fn in (("claude", _scan_claude), ("codex", _scan_codex),
                     ("opencode", _scan_opencode)):
        try:
            rows = fn()
            sessions.extend(rows)
            tools[name] = {"ok": True, "count": len(rows)}
        except Exception as e:
            tools[name] = {"ok": False, "error": str(e)[:200]}
    _cache_flush()
    sessions.sort(key=lambda s: s["updated"], reverse=True)
    return {"tools": tools, "sessions": sessions}


# ---------- 会话详情 ----------

def show(tool: str, ref: str) -> dict:
    path = convert.resolve_ref(tool, ref)
    sess = convert.READERS[tool](path)
    msgs = []
    for i, m in enumerate(sess.messages):
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
        msgs.append({"index": i, "role": m.role, "blocks": blocks})
    return {"tool": tool, "id": sess.source_id, "title": sess.title,
            "dir": sess.cwd, "count": len(msgs), "loss": sess.loss,
            "messages": msgs}


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
    for m in sess.messages:
        for b in m.blocks:
            if b.kind == "text":
                native += 1
            elif b.kind == "tool":
                if b.tool.op:
                    native += 1
                else:
                    degrade += 1
                    details.append(f"工具 {b.tool.name} 将降级为叙述文本")
    dropped = list(sess.loss)
    return {"native": native, "degrade": degrade, "drop": len(dropped),
            "degrade_details": details, "drop_details": dropped}


def _append_history(entry: dict):
    HISTORY.parent.mkdir(parents=True, exist_ok=True)
    with HISTORY.open("a") as f:
        f.write(json.dumps(entry, ensure_ascii=False) + "\n")


def migrate(src: str, dst: str, ref: str, cwd: str | None = None,
            dry_run: bool = False, probe: bool = True) -> dict:
    path = convert.resolve_ref(src, ref)
    sess = convert.READERS[src](path)
    target_cwd = str(Path(cwd or sess.cwd or ".").resolve())
    stats = _loss_stats(sess, dst)
    base = {"src": src, "dst": dst, "source_id": sess.source_id,
            "title": sess.title, "cwd": target_cwd, "loss": stats}
    if dry_run:
        return {**base, "dry_run": True}

    sid, dest = convert.WRITERS[dst](sess, cwd=target_cwd)
    # 写回阶段可能追加新的损耗(writer 分发时才知道)
    base["loss"] = _loss_stats(sess, dst)
    result = {**base, "session_id": sid, "dest": str(dest),
              "resume": resume_command(dst, sid, target_cwd)}

    if probe:
        ok, detail = run_probe(dst, sid, target_cwd)
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


def _cleanup_artifact(dst: str, sid: str, dest):
    if dst in ("claude", "codex"):
        try:
            hits = glob.glob(os.path.expanduser(
                {"claude": f"~/.claude/projects/*/{sid}.jsonl",
                 "codex": f"~/.codex/sessions/*/*/*/rollout-*-{sid}.jsonl"}[dst]))
            for h in hits:
                os.unlink(h)
        except OSError:
            pass
    # opencode 产物删除需 server 配合,保留并在结果中标注即可


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


def edit_apply(ref: str, ops: list[dict], probe: bool = True) -> dict:
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

def main():
    args = sys.argv[1:]
    if not args:
        sys.exit(__doc__)
    cmd, rest = args[0], args[1:]
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
