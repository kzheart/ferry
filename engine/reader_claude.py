"""Claude Code reader:JSONL 会话文件 → 规范化中间格式。

格式规格见 spec/formats/claude-code.md。
MVP 限制:按文件顺序线性读取(分支树取全部出现的消息);isSidechain 记录跳过。
"""
import json
from pathlib import Path

from .model import Block, Message, Session, ToolCall

# 工具名 → 规范操作(与 spec/mapping/tools.yaml 保持一致)
TOOL_OPS = {"Bash": "shell.exec", "Read": "fs.read",
            "Write": "fs.write", "Edit": "fs.edit"}


def _norm_input(name: str, inp: dict) -> dict:
    """原生参数 → 规范参数名(file_path/old/new/command/content)。"""
    if name == "Edit":
        return {"file_path": inp.get("file_path", ""),
                "old": inp.get("old_string", ""),
                "new": inp.get("new_string", "")}
    if name == "Read":
        return {"file_path": inp.get("file_path", "")}
    if name == "Write":
        return {"file_path": inp.get("file_path", ""),
                "content": inp.get("content", "")}
    if name == "Bash":
        return {"command": inp.get("command", "")}
    return inp


def _result_text(block) -> str:
    c = block.get("content")
    if isinstance(c, str):
        return c
    if isinstance(c, list):
        return "\n".join(b.get("text", "") for b in c
                         if isinstance(b, dict) and b.get("type") == "text")
    return ""


def read(path: str) -> Session:
    lines = [json.loads(l) for l in Path(path).read_text().splitlines() if l.strip()]
    msgs = [l for l in lines if l.get("type") in ("user", "assistant")
            and not l.get("isSidechain")]
    first = msgs[0] if msgs else {}
    sess = Session(source_tool="claude",
                   source_id=first.get("sessionId", Path(path).stem),
                   cwd=first.get("cwd", ""))
    for l in lines:
        if l.get("type") == "ai-title":
            sess.title = l.get("title", "") or sess.title

    pending: dict[str, ToolCall] = {}   # tool_use_id → ToolCall(待配对)
    for rec in msgs:
        m = rec.get("message") or {}
        content = m.get("content")
        role = m.get("role")
        if isinstance(content, str):
            sess.messages.append(Message(role=role,
                                         blocks=[Block("text", content)],
                                         raw=[rec]))
            continue
        blocks, is_tool_result_carrier = [], False
        for b in content or []:
            t = b.get("type")
            if t == "text":
                blocks.append(Block("text", b.get("text", "")))
            elif t == "thinking":
                sess.lose("thinking 块丢弃(签名绑定 Anthropic)")
            elif t == "tool_use":
                name = b.get("name", "")
                op = TOOL_OPS.get(name)
                inp = b.get("input") or {}
                tc = ToolCall(name=name, op=op,
                              input=_norm_input(name, inp) if op else inp,
                              output="")
                pending[b.get("id")] = tc
                blocks.append(Block("tool", tool=tc))
            elif t == "tool_result":
                is_tool_result_carrier = True
                tc = pending.pop(b.get("tool_use_id"), None)
                if tc is None:
                    sess.lose(f"孤儿 tool_result: {b.get('tool_use_id')}")
                    continue
                tc.output = _result_text(b)
                tur = rec.get("toolUseResult")
                if isinstance(tur, dict):
                    tc.meta = {k: tur[k] for k in ("stdout", "stderr")
                               if k in tur}
            else:
                sess.lose(f"未知内容块类型丢弃: {t}")
        if is_tool_result_carrier and not any(
                bl.kind == "text" and bl.text.strip() for bl in blocks):
            # 纯工具结果载体:输出已并入 ToolCall,不单独成消息
            continue
        if blocks:
            sess.messages.append(Message(role=role, blocks=blocks, raw=[rec]))
    for tc in pending.values():
        sess.lose(f"未配对 tool_use({tc.name})按无输出处理")
    return sess
