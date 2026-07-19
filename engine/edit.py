#!/usr/bin/env python3
"""会话编辑(会话手术),MVP 支持 Claude Code 会话的原地编辑。

用法:
    python3 -m engine.edit truncate <会话ref> [--threshold 4096]   裁剪超大工具输出
    python3 -m engine.edit redact   <会话ref> --find X [--replace 文本]  全会话脱敏
    python3 -m engine.edit delete-turn <会话ref> --turn N          删除第 N 轮(从 1 数)
    python3 -m engine.edit rewrite  <会话ref> --uuid U --text 新文本   改写单条消息
    python3 -m engine.edit restore  <会话ref>                      还原最近一次快照

安全约定(见 README「关键决策」):
- 编辑前自动快照到 ~/.resume-harness/backups/,restore 可还原;
- 原子写入(temp+rename);
- 编辑后必须用 harness/probe.py 验收(本工具会打印验收命令)。
"""
import argparse
import glob
import json
import os
import shutil
import sys
import time
from pathlib import Path

BACKUP_DIR = Path.home() / ".resume-harness" / "backups"
REDACTED = "[REDACTED]"


def resolve(ref: str) -> Path:
    if os.path.exists(ref):
        return Path(ref)
    hits = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{ref}.jsonl"))
    if not hits:
        sys.exit(f"找不到 Claude 会话: {ref}")
    return Path(hits[0])


def backup(path: Path) -> Path:
    BACKUP_DIR.mkdir(parents=True, exist_ok=True)
    dest = BACKUP_DIR / f"{path.stem}-{int(time.time())}.jsonl"
    shutil.copy(path, dest)
    return dest


def load(path: Path) -> list[dict]:
    return [json.loads(l) for l in path.read_text().splitlines() if l.strip()]


def save(path: Path, records: list[dict]):
    tmp = path.with_suffix(".tmp")
    tmp.write_text("\n".join(json.dumps(r, ensure_ascii=False)
                             for r in records) + "\n")
    tmp.rename(path)


def relink(records: list[dict], removed_uuids: set[str]):
    """删除消息后重连 parentUuid 链:指向被删节点的,改指其最近存活祖先。"""
    parent_of = {r["uuid"]: r.get("parentUuid") for r in records
                 if "uuid" in r}
    def nearest_alive(u):
        while u is not None and u in removed_uuids:
            u = parent_of.get(u)
        return u
    for r in records:
        if r.get("parentUuid") in removed_uuids:
            r["parentUuid"] = nearest_alive(r["parentUuid"])


def _walk_strings(obj, fn):
    if isinstance(obj, dict):
        return {k: _walk_strings(v, fn) for k, v in obj.items()}
    if isinstance(obj, list):
        return [_walk_strings(v, fn) for v in obj]
    if isinstance(obj, str):
        return fn(obj)
    return obj


# ---------- 操作 ----------

def op_truncate(records, args):
    n = 0
    keep = args.threshold // 2
    for r in records:
        content = (r.get("message") or {}).get("content")
        if not isinstance(content, list):
            continue
        for b in content:
            if b.get("type") == "tool_result" and \
                    isinstance(b.get("content"), str) and \
                    len(b["content"]) > args.threshold:
                s = b["content"]
                b["content"] = (s[:keep] + f"\n\n[...已裁剪 {len(s)-2*keep} 字符...]\n\n"
                                + s[-keep:])
                n += 1
        tur = r.get("toolUseResult")
        if isinstance(tur, dict) and isinstance(tur.get("stdout"), str) \
                and len(tur["stdout"]) > args.threshold:
            tur["stdout"] = tur["stdout"][:keep] + "...[truncated]"
    print(f"裁剪了 {n} 条超过 {args.threshold} 字符的工具输出")
    return records


def op_redact(records, args):
    count = 0
    def fn(s):
        nonlocal count
        if args.find in s:
            count += s.count(args.find)
            return s.replace(args.find, args.replace)
        return s
    out = [_walk_strings(r, fn) for r in records]
    print(f"替换了 {count} 处")
    if count == 0:
        sys.exit("未命中任何内容,不写入")
    return out


def _turns(records):
    """轮 = 真实用户消息(非 tool_result 载体)到下一条真实用户消息之前。"""
    starts = []
    for i, r in enumerate(records):
        if r.get("type") != "user" or r.get("isSidechain"):
            continue
        c = (r.get("message") or {}).get("content")
        is_tool_carrier = isinstance(c, list) and any(
            b.get("type") == "tool_result" for b in c)
        if not is_tool_carrier:
            starts.append(i)
    return starts


def op_delete_turn(records, args):
    starts = _turns(records)
    if not 1 <= args.turn <= len(starts):
        sys.exit(f"轮次超界:共 {len(starts)} 轮")
    lo = starts[args.turn - 1]
    hi = starts[args.turn] if args.turn < len(starts) else len(records)
    removed = records[lo:hi]
    removed_uuids = {r["uuid"] for r in removed if "uuid" in r}
    kept = records[:lo] + records[hi:]
    relink(kept, removed_uuids)
    print(f"删除第 {args.turn} 轮:{len(removed)} 条记录")
    return kept


def op_rewrite(records, args):
    hit = False
    for r in records:
        if r.get("uuid") != args.uuid:
            continue
        m = r.get("message") or {}
        if isinstance(m.get("content"), str):
            m["content"] = args.text
        elif isinstance(m.get("content"), list):
            m["content"] = [{"type": "text", "text": args.text}]
        hit = True
    if not hit:
        sys.exit(f"未找到 uuid={args.uuid}")
    print("已改写 1 条消息")
    return records


def op_restore(path: Path):
    cands = sorted(BACKUP_DIR.glob(f"{path.stem}-*.jsonl"))
    if not cands:
        sys.exit("没有该会话的快照")
    shutil.copy(cands[-1], path)
    print(f"已从 {cands[-1].name} 还原")


# ---------- 校验 ----------

def check_invariants(records):
    uuids = [r["uuid"] for r in records if "uuid" in r]
    assert len(uuids) == len(set(uuids)), "uuid 重复"
    uset = set(uuids)
    for r in records:
        p = r.get("parentUuid")
        assert p is None or p in uset, f"parentUuid 悬空: {p}"
    uses, results = set(), set()
    for r in records:
        c = (r.get("message") or {}).get("content")
        if isinstance(c, list):
            for b in c:
                if b.get("type") == "tool_use":
                    uses.add(b["id"])
                elif b.get("type") == "tool_result":
                    results.add(b["tool_use_id"])
    assert results <= uses, f"孤儿 tool_result: {results - uses}"
    assert uses <= results, f"未配对 tool_use: {uses - results}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("op", choices=["truncate", "redact", "delete-turn",
                                   "rewrite", "restore"])
    ap.add_argument("ref")
    ap.add_argument("--threshold", type=int, default=4096)
    ap.add_argument("--find")
    ap.add_argument("--replace", default=REDACTED)
    ap.add_argument("--turn", type=int)
    ap.add_argument("--uuid")
    ap.add_argument("--text")
    args = ap.parse_args()
    path = resolve(args.ref)

    if args.op == "restore":
        op_restore(path)
        return
    if args.op == "redact" and not args.find:
        ap.error("redact 需要 --find")
    if args.op == "delete-turn" and not args.turn:
        ap.error("delete-turn 需要 --turn")
    if args.op == "rewrite" and not (args.uuid and args.text):
        ap.error("rewrite 需要 --uuid 和 --text")

    bak = backup(path)
    records = load(path)
    fn = {"truncate": op_truncate, "redact": op_redact,
          "delete-turn": op_delete_turn, "rewrite": op_rewrite}[args.op]
    records = fn(records, args)
    check_invariants(records)
    save(path, records)
    sid = path.stem
    proj = None
    for r in records:
        if r.get("cwd"):
            proj = r["cwd"]
            break
    print(f"快照: {bak}")
    print(f"验收: python3 harness/probe.py claude {sid} --dir {proj}")


if __name__ == "__main__":
    main()
