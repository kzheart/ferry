"""Codex reader:rollout JSONL → 规范化中间格式。

格式规格见 spec/formats/codex.md。
支持 0.144 的 custom_tool_call(exec/apply_patch,JS 源码 input)与旧版 function_call。
"""
import json
import re
from pathlib import Path

from .model import Block, Message, Session, ToolCall

_EXEC_RE = re.compile(r"tools\.exec_command\((\{.*?\})\)", re.S)
_PATCH_RE = re.compile(r"tools\.apply_patch\((.*?)\)\s*;?", re.S)
_ADD_FILE_RE = re.compile(r"\*\*\* Add File: (.+?)\n(.*?)\n?\*\*\* End Patch", re.S)
_UPD_FILE_RE = re.compile(r"\*\*\* Update File: (.+?)\n(.*?)\n?\*\*\* End Patch", re.S)
_SKIP_USER_PREFIX = ("<environment_context>", "<user_instructions>",
                     "<ENVIRONMENT_CONTEXT>", "<turn_aborted>")


def _parse_call(payload, sess) -> ToolCall:
    src = payload.get("input", "")
    m = _EXEC_RE.search(src)
    if m:
        try:
            args = json.loads(m.group(1))
            return ToolCall(name="exec", op="shell.exec",
                            input={"command": args.get("cmd", ""),
                                   "workdir": args.get("workdir")}, output="")
        except json.JSONDecodeError:
            pass
    m = _PATCH_RE.search(src)
    if m:
        # patch 文本存在 JS 字符串字面量/变量里,尝试从整段源码提取
        text = src.encode().decode("unicode_escape") if "\\n" in src else src
        m2 = _ADD_FILE_RE.search(text)
        if m2:
            body = "\n".join(l[1:] for l in m2.group(2).splitlines()
                             if l.startswith("+"))
            return ToolCall(name="apply_patch", op="fs.write",
                            input={"file_path": m2.group(1).strip(),
                                   "content": body}, output="")
        m3 = _UPD_FILE_RE.search(text)
        if m3:
            lines = m3.group(2).splitlines()
            old = "\n".join(l[1:] for l in lines if l.startswith("-"))
            new = "\n".join(l[1:] for l in lines if l.startswith("+"))
            return ToolCall(name="apply_patch", op="fs.edit",
                            input={"file_path": m3.group(1).strip(),
                                   "old": old, "new": new}, output="")
        sess.lose("apply_patch 无法解析出 Add/Update File,降级")
    return ToolCall(name=payload.get("name", "custom_tool"), op=None,
                    input=src, output="")


def _parse_output(raw: str) -> str:
    """custom_tool_call_output.output 是 JSON 数组包装,提取真实 stdout。"""
    try:
        blocks = json.loads(raw)
        texts = [b.get("text", "") for b in blocks
                 if isinstance(b, dict) and b.get("type") == "input_text"]
        for t in texts:
            try:
                inner = json.loads(t)
                if isinstance(inner, dict) and "output" in inner:
                    return inner["output"]
            except json.JSONDecodeError:
                continue
        return "\n".join(texts)
    except (json.JSONDecodeError, TypeError):
        return raw if isinstance(raw, str) else str(raw)


def read(path: str) -> Session:
    lines = [json.loads(l) for l in Path(path).read_text().splitlines()
             if l.strip()]
    meta = next((l["payload"] for l in lines if l["type"] == "session_meta"), {})
    sess = Session(source_tool="codex",
                   source_id=meta.get("session_id", Path(path).stem),
                   cwd=meta.get("cwd", ""))
    pending: dict[str, ToolCall] = {}
    cur_tools: list[Block] = []          # 未落消息的工具块,附到下一条 assistant

    def flush_tools_into(blocks):
        nonlocal cur_tools
        blocks[:0] = cur_tools
        cur_tools = []

    for l in lines:
        if l["type"] != "response_item":
            continue
        p = l["payload"]
        pt = p.get("type")
        if pt == "message":
            texts = [c.get("text", "") for c in p.get("content", [])
                     if c.get("type") in ("input_text", "output_text")]
            text = "\n".join(t for t in texts if t)
            if p["role"] == "user" and text.strip().startswith(_SKIP_USER_PREFIX):
                continue
            if not text.strip() and not cur_tools:
                continue
            blocks = [Block("text", text)] if text.strip() else []
            if p["role"] == "assistant":
                flush_tools_into(blocks)
            sess.messages.append(Message(role=p["role"], blocks=blocks, raw=[l]))
        elif pt in ("custom_tool_call", "function_call"):
            if pt == "function_call":
                try:
                    args = json.loads(p.get("arguments", "{}"))
                    cmd = args.get("command")
                    cmd = " ".join(cmd[2:]) if isinstance(cmd, list) and \
                        cmd[:2] == ["bash", "-lc"] else \
                        (" ".join(cmd) if isinstance(cmd, list) else str(cmd))
                    tc = ToolCall(name="shell", op="shell.exec",
                                  input={"command": cmd}, output="")
                except json.JSONDecodeError:
                    tc = ToolCall(name=p.get("name", "?"), op=None,
                                  input=p.get("arguments", ""), output="")
            else:
                tc = _parse_call(p, sess)
            pending[p.get("call_id")] = tc
            cur_tools.append(Block("tool", tool=tc))
        elif pt in ("custom_tool_call_output", "function_call_output"):
            tc = pending.pop(p.get("call_id"), None)
            if tc is not None:
                tc.output = _parse_output(p.get("output", ""))
            else:
                sess.lose(f"孤儿 tool output: {p.get('call_id')}")
        elif pt == "reasoning":
            sess.lose("reasoning 块丢弃(加密/不可迁移)")
        else:
            sess.lose(f"未知 response_item 丢弃: {pt}")
    if cur_tools:
        sess.messages.append(Message(role="assistant", blocks=cur_tools, raw=[]))
    return sess
