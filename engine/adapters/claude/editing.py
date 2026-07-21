"""Claude Code 会话文件原语：解析、快照、原子写入与结构校验。

轮次/编辑语义统一由 ``claude.codec`` 持有；跨工具编排见 application 层。
"""
import glob
import json
import os
from pathlib import Path

from ...domain.errors import SessionNotFoundError
from ...infrastructure.snapshots import snapshot_file


def resolve(ref: str) -> Path:
    if os.path.exists(ref):
        return Path(ref)
    hits = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{ref}.jsonl"))
    if not hits:
        raise SessionNotFoundError("claude", ref)
    return Path(hits[0])


def backup(path: Path, reason_code: str = "snapshot.before_edit",
           tool: str = "claude", extra: dict | None = None) -> Path:
    return snapshot_file(path, reason_code, tool, extra)


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
