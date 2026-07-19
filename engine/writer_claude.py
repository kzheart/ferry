"""Claude Code writer:规范化中间格式 → 项目目录下的会话 JSONL。

格式规格见 spec/formats/claude-code.md。
记录结构模板取自黄金样本;uuid→parentUuid 线性链由本 writer 生成。
shell.exec → Bash,fs.write → Write,fs.read → Read;其余降级叙述文本。
"""
import json
import re
import time
import uuid
from pathlib import Path

from .model import Session

GOLDEN = Path(__file__).resolve().parent.parent / "golden" / "claude"

_OP_TOOLS = {
    "shell.exec": ("Bash", lambda i: {"command": i.get("command", "")}),
    "fs.write": ("Write", lambda i: {"file_path": i.get("file_path", ""),
                                     "content": i.get("content", "")}),
    "fs.read": ("Read", lambda i: {"file_path": i.get("file_path", "")}),
    "fs.edit": ("Edit", lambda i: {"file_path": i.get("file_path", ""),
                                   "old_string": i.get("old", ""),
                                   "new_string": i.get("new", "")}),
}


def _slug(path: str) -> str:
    return re.sub(r"[^A-Za-z0-9]", "-", str(Path(path).resolve()))


def _load_templates():
    versions = sorted(GOLDEN.iterdir()) if GOLDEN.exists() else []
    if not versions:
        raise RuntimeError("缺少 golden/claude 样本,先运行 harness/gen_golden.py")
    sample = versions[-1] / "case-02-tools" / "session.jsonl"
    tpl = {}
    for line in sample.read_text().splitlines():
        rec = json.loads(line)
        if rec.get("type") in ("user", "assistant") and "user" not in tpl:
            if rec["type"] == "user" and isinstance(
                    rec.get("message", {}).get("content"), str):
                tpl["user"] = rec
        if rec.get("type") == "assistant" and "assistant" not in tpl:
            tpl["assistant"] = rec
    if "user" not in tpl or "assistant" not in tpl:
        raise RuntimeError("黄金样本中未找到 user/assistant 模板记录")
    return tpl


def _narration(t) -> str:
    inp = json.dumps(t.input, ensure_ascii=False)[:500] \
        if isinstance(t.input, dict) else str(t.input)[:500]
    return (f"[历史记录:此前通过工具 {t.name} 执行了操作]\n"
            f"参数: {inp}\n结果:\n{(t.output or '(无输出)')[:2000]}")


def write(sess: Session, cwd: str | None = None) -> tuple[str, Path]:
    tpl = _load_templates()
    sid = str(uuid.uuid4())
    cwd = cwd or sess.cwd
    records, parent = [], None
    ts = time.time() - len(sess.messages) * 2   # 时间戳递增,结束于当前

    def base(kind) -> dict:
        nonlocal parent, ts
        rec = json.loads(json.dumps(tpl[kind]))
        rec["uuid"] = str(uuid.uuid4())
        rec["parentUuid"] = parent
        rec["sessionId"] = sid
        rec["cwd"] = cwd
        ts += 2
        rec["timestamp"] = time.strftime(
            "%Y-%m-%dT%H:%M:%S", time.gmtime(ts)) + ".000Z"
        for k in ("toolUseResult", "sourceToolAssistantUUID", "promptSource"):
            rec.pop(k, None)
        parent = rec["uuid"]
        return rec

    def add_user_text(text):
        rec = base("user")
        rec["message"] = {"role": "user", "content": text}
        records.append(rec)

    def add_assistant_text(text):
        rec = base("assistant")
        rec["message"]["content"] = [{"type": "text", "text": text}]
        records.append(rec)

    def add_tool(t):
        tool, conv = _OP_TOOLS[t.op]
        use_id = "toolu_" + uuid.uuid4().hex[:24]
        rec = base("assistant")
        rec["message"]["content"] = [{"type": "tool_use", "id": use_id,
                                      "name": tool, "input": conv(t.input)}]
        records.append(rec)
        rec = base("user")
        out = t.output or ""
        rec["message"] = {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": use_id, "content": out}]}
        if tool == "Bash":
            rec["toolUseResult"] = {"stdout": t.meta.get("stdout", out),
                                    "stderr": t.meta.get("stderr", ""),
                                    "interrupted": False, "isImage": False}
        records.append(rec)

    for m in sess.messages:
        texts = []
        for b in m.blocks:
            if b.kind == "text":
                texts.append(b.text)
            elif b.kind == "tool":
                t = b.tool
                if t.op in _OP_TOOLS and isinstance(t.input, dict):
                    if texts:
                        (add_assistant_text if m.role == "assistant"
                         else add_user_text)("\n\n".join(texts))
                        texts = []
                    add_tool(t)
                else:
                    sess.lose(f"工具 {t.name} 降级为叙述文本")
                    texts.append(_narration(t))
        if texts:
            (add_assistant_text if m.role == "assistant"
             else add_user_text)("\n\n".join(texts))

    dest_dir = Path.home() / ".claude" / "projects" / _slug(cwd)
    dest_dir.mkdir(parents=True, exist_ok=True)
    dest = dest_dir / f"{sid}.jsonl"
    tmp = dest.with_suffix(".tmp")
    tmp.write_text("\n".join(json.dumps(r, ensure_ascii=False)
                             for r in records) + "\n")
    tmp.rename(dest)
    return sid, dest
