"""Codex reader:rollout JSONL → 规范化中间格式。

读取 Codex 原生 JSONL 会话记录。
支持 0.144 的 custom_tool_call(exec/apply_patch,JS 源码 input)与旧版 function_call。
"""
import json
import re
from pathlib import Path

from ...domain.model import AgentEdge, Block, Message, RawRecord, Session, ToolCall
from ...domain.reasoning import codex_summary_text
from ...domain.tool_ops import CanonicalOp
from ...infrastructure.scan_cache import ScanCache
from ..base.media import image_from_data_url

_META_CACHE_PATH = Path.home() / ".resume-harness" / "rollout-meta-cache.json"

_EXEC_RE = re.compile(r"tools\.exec_command\((\{.*?\})\)", re.S)
_PATCH_RE = re.compile(r"tools\.apply_patch\((.*?)\)\s*;?", re.S)
_ADD_FILE_RE = re.compile(r"\*\*\* Add File: (.+?)\n(.*?)\n?\*\*\* End Patch", re.S)
_UPD_FILE_RE = re.compile(r"\*\*\* Update File: (.+?)\n(.*?)\n?\*\*\* End Patch", re.S)
_SKIP_USER_PREFIX = ("<environment_context>", "<user_instructions>",
                      "<ENVIRONMENT_CONTEXT>", "<turn_aborted>")


def session_id(meta: dict, fallback: str) -> str:
    # 子代理的 session_id 指向父会话，id 才是当前 rollout 的身份。
    return meta.get("id") or meta.get("session_id") or fallback


def _parse_call(payload, sess) -> ToolCall:
    src = payload.get("input", "")
    m = _EXEC_RE.search(src)
    if m:
        try:
            args = json.loads(m.group(1))
            return ToolCall(name="exec", op=CanonicalOp.SHELL_EXEC,
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
            return ToolCall(name="apply_patch", op=CanonicalOp.FS_WRITE,
                            input={"file_path": m2.group(1).strip(),
                                   "content": body}, output="")
        m3 = _UPD_FILE_RE.search(text)
        if m3:
            lines = m3.group(2).splitlines()
            old = "\n".join(l[1:] for l in lines if l.startswith("-"))
            new = "\n".join(l[1:] for l in lines if l.startswith("+"))
            return ToolCall(name="apply_patch", op=CanonicalOp.FS_EDIT,
                            input={"file_path": m3.group(1).strip(),
                                   "old": old, "new": new}, output="")
        sess.lose("migration.apply_patch_unparsed")
    return ToolCall(name=payload.get("name", "custom_tool"), op=None,
                    input=src, output="")


def _parse_output(raw) -> str:
    """custom_tool_call_output.output 是 JSON 数组包装,提取真实 stdout。"""
    try:
        blocks = raw if isinstance(raw, list) else json.loads(raw)
        if not isinstance(blocks, list):
            return raw if isinstance(raw, str) else str(raw)
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


def _json_args(raw) -> dict | str:
    if isinstance(raw, dict):
        return raw
    try:
        value = json.loads(raw or "{}")
        return value if isinstance(value, dict) else raw
    except (json.JSONDecodeError, TypeError):
        return raw or ""


def _spawn_input(raw) -> dict:
    args = raw if isinstance(raw, dict) else {}
    return {
        "description": str(args.get("description") or "migrated subagent"),
        "prompt": str(args.get("prompt") or args.get("message") or ""),
        "subagent_type": str(args.get("subagent_type") or
                             args.get("agent_type") or "general"),
    }


def _subagent_meta(meta: dict) -> dict:
    source = meta.get("source")
    if not isinstance(source, dict):
        source = meta.get("thread_source")
    if not isinstance(source, dict):
        return {}
    subagent = source.get("subagent", {})
    return subagent if isinstance(subagent, dict) else {}


def _identity(meta: dict, fallback: str) -> dict:
    subagent = _subagent_meta(meta)
    spawn = subagent.get("thread_spawn", {})
    if not isinstance(spawn, dict):
        spawn = {}
    current_id = session_id(meta, fallback)
    root_id = meta.get("session_id") or spawn.get("session_id") or current_id
    parent_id = (meta.get("parent_thread_id") or
                 spawn.get("parent_thread_id") or
                 subagent.get("parent_thread_id"))
    return {
        "id": current_id,
        "root_id": root_id,
        "parent_id": parent_id,
        "forked_from_id": (meta.get("forked_from_id") or
                           spawn.get("forked_from_id") or parent_id),
        "agent_id": (subagent.get("agent_id") or spawn.get("agent_id") or
                     meta.get("agent_id")),
        "agent_path": (subagent.get("agent_path") or spawn.get("agent_path") or
                       meta.get("agent_path")),
        "agent_type": (subagent.get("agent_type") or spawn.get("agent_type") or
                       meta.get("agent_type")),
        "depth": subagent.get("depth", spawn.get("depth")),
    }


def _first_meta(path: Path) -> dict:
    try:
        with path.open() as stream:
            for line in stream:
                if not line.strip():
                    continue
                record = json.loads(line)
                if record.get("type") == "session_meta":
                    return record.get("payload") or {}
    except (OSError, json.JSONDecodeError):
        pass
    return {}


def _sessions_root(path: Path) -> Path:
    for parent in (path.parent, *path.parents):
        if parent.name == "sessions":
            return parent
    return path.parent


def _rollout_index(path: Path, sessions_dir: str | Path | None) -> dict[str, tuple[Path, dict, dict]]:
    """Scan the sessions tree once; recursive session loading only uses this index."""
    root = Path(sessions_dir).expanduser() if sessions_dir else _sessions_root(path)
    candidates = list(root.rglob("rollout*.jsonl")) if root.exists() else []
    if path not in candidates:
        candidates.append(path)
    cache = ScanCache(_META_CACHE_PATH, version=2)
    dirty = False
    index = {}
    for candidate in candidates:
        try:
            stat = candidate.stat()
        except OSError:
            continue
        ident = cache.get(candidate, stat)
        if ident is None:
            meta = _first_meta(candidate)
            ident = _identity(meta, candidate.stem) if meta else {}
            cache.put(candidate, stat, ident)
            dirty = True
        if not ident:
            continue
        index[ident["id"]] = (candidate, ident)
    if dirty:
        try:
            cache.flush()
        except OSError:
            pass
    return index


def _raw_record(record: dict, ordinal: int, path: Path) -> RawRecord:
    payload = record.get("payload") or {}
    subtype = payload.get("type") if isinstance(payload, dict) else None
    record_type = record.get("type", "unknown")
    if subtype:
        record_type += "." + str(subtype)
    return RawRecord(source="codex", record_type=record_type,
                     payload=record, ordinal=ordinal,
                     timestamp=record.get("timestamp"), location=str(path))


def _read_one(path: Path, meta: dict | None = None) -> Session:
    lines = [json.loads(l) for l in Path(path).read_text().splitlines()
             if l.strip()]
    meta = meta or next((l["payload"] for l in lines
                         if l["type"] == "session_meta"), {})
    ident = _identity(meta, path.stem)
    sess = Session(source_tool="codex",
                   source_id=ident["id"],
                   cwd=meta.get("cwd", ""))
    sess.root_id = ident["root_id"]
    sess.parent_id = ident["parent_id"]
    sess.forked_from_id = ident["forked_from_id"]
    sess.agent_id = ident["agent_id"]
    sess.agent_path = ident["agent_path"]
    sess.agent_type = ident["agent_type"]
    sess.meta = dict(meta)
    if ident["depth"] is not None:
        sess.meta["depth"] = ident["depth"]
    pending: dict[str, ToolCall] = {}
    cur_tools: list[Block] = []          # 未落消息的工具块,附到下一条 assistant
    cur_reasoning: list[Block] = []      # 可见 reasoning 降级为 text,附到下一条 assistant

    def flush_pending_into(blocks):
        nonlocal cur_tools, cur_reasoning
        blocks[:0] = cur_reasoning + cur_tools
        cur_tools = []
        cur_reasoning = []

    for ordinal, l in enumerate(lines):
        sess.raw_records.append(_raw_record(l, ordinal, path))
        if l["type"] != "response_item":
            continue
        p = l["payload"]
        pt = p.get("type")
        if pt == "message":
            content = p.get("content", [])
            texts = [c.get("text", "") for c in content
                     if c.get("type") in ("input_text", "output_text")]
            text = "\n".join(t for t in texts if t)
            image_blocks = []
            for content_index, item in enumerate(content):
                if item.get("type") != "input_image":
                    continue
                image = image_from_data_url(
                    f"record:{ordinal}:image:{content_index}", item.get("image_url", ""))
                if image is None:
                    sess.lose("migration.unknown_block_dropped", kind="input_image")
                else:
                    image_blocks.append(Block("image", image=image))
            if p["role"] == "user" and text.strip().startswith(_SKIP_USER_PREFIX):
                continue
            if p["role"] == "user" and (cur_tools or cur_reasoning):
                pending_blocks = []
                flush_pending_into(pending_blocks)
                sess.messages.append(Message(role="assistant", blocks=pending_blocks,
                                             raw=[]))
            if not text.strip() and not image_blocks and not cur_tools and not cur_reasoning:
                continue
            blocks = ([Block("text", text)] if text.strip() else []) + image_blocks
            if p["role"] == "assistant":
                flush_pending_into(blocks)
            sess.messages.append(Message(role=p["role"], blocks=blocks, raw=[l],
                                         source_id=f"record:{ordinal}"))
        elif pt in ("custom_tool_call", "function_call"):
            if pt == "function_call":
                args = _json_args(p.get("arguments", "{}"))
                if p.get("name") == "spawn_agent":
                    tc = ToolCall(name="spawn_agent", op=CanonicalOp.AGENT_SPAWN,
                                  input=_spawn_input(args), output="",
                                  meta={"source_id": p.get("id")},
                                  source_call_id=p.get("call_id"),
                                  status=p.get("status"))
                else:
                    command_args = args if isinstance(args, dict) else {}
                    if (p.get("name") in {"shell", "exec_command"} or
                            command_args.get("command") is not None):
                        cmd = command_args.get("command")
                        cmd = " ".join(cmd[2:]) if isinstance(cmd, list) and \
                            cmd[:2] == ["bash", "-lc"] else \
                            (" ".join(cmd) if isinstance(cmd, list) else str(cmd))
                        tc = ToolCall(name=p.get("name", "shell"), op=CanonicalOp.SHELL_EXEC,
                                      input={"command": cmd}, output="",
                                      source_call_id=p.get("call_id"))
                    else:
                        tc = ToolCall(name=p.get("name", "?"), op=None,
                                      input=_json_args(p.get("arguments", "")), output="",
                                      source_call_id=p.get("call_id"))
            else:
                if p.get("name") == "spawn_agent":
                    tc = ToolCall(name="spawn_agent", op=CanonicalOp.AGENT_SPAWN,
                                  input=_spawn_input(_json_args(p.get("input", ""))), output="",
                                  meta={"source_id": p.get("id")},
                                  status=p.get("status"))
                else:
                    tc = _parse_call(p, sess)
                tc.source_call_id = p.get("call_id")
            pending[p.get("call_id")] = tc
            cur_tools.append(Block("tool", tool=tc))
        elif pt in ("custom_tool_call_output", "function_call_output"):
            tc = pending.pop(p.get("call_id"), None)
            if tc is not None:
                tc.output = _parse_output(p.get("output", ""))
                tc.source_result_id = p.get("id")
            else:
                sess.lose("session.orphan_tool_result", call_id=p.get("call_id"))
        elif pt == "reasoning":
            text = codex_summary_text(p)
            if text is not None:
                cur_reasoning.append(Block("text", text))
                sess.lose("migration.reasoning_metadata_dropped", metadata_kind="encrypted_content")
            else:
                sess.lose("migration.reasoning_dropped", metadata_kind="encrypted_content")
        else:
            sess.lose("migration.unknown_block_dropped", kind=pt)
    if cur_tools or cur_reasoning:
        blocks = []
        flush_pending_into(blocks)
        sess.messages.append(Message(role="assistant", blocks=blocks, raw=[]))
    return sess


def _spawn_calls(sess: Session) -> list[ToolCall]:
    return [block.tool for message in sess.messages for block in message.blocks
            if block.kind == "tool" and block.tool and
            block.tool.op == CanonicalOp.AGENT_SPAWN]


def _contains_identity(tool: ToolCall, child: Session) -> bool:
    values = [child.source_id, child.agent_id, child.agent_path]
    haystack = json.dumps({"input": tool.input, "output": tool.output},
                          ensure_ascii=False)
    return any(value and value in haystack for value in values)


def _attach_tree(sess: Session, by_parent: dict[str, list[Session]], seen: set[str]):
    if sess.source_id in seen:
        return
    seen.add(sess.source_id)
    spawn_calls = _spawn_calls(sess)
    used_calls: set[int] = set()
    for child in by_parent.get(sess.source_id, []):
        if child.source_id in seen:
            continue
        matched = next((tool for tool in spawn_calls
                        if id(tool) not in used_calls and
                        _contains_identity(tool, child)), None)
        if matched:
            used_calls.add(id(matched))
        elif spawn_calls:
            sess.lose("session.subagent_unlinked", child_id=child.source_id)
        prompt = ""
        if matched and isinstance(matched.input, dict):
            prompt = str(matched.input.get("prompt") or "")
        edge = AgentEdge(
            parent_session_id=sess.source_id,
            child_session_id=child.source_id,
            source_call_id=matched.source_call_id if matched else None,
            spawn_message_id=(matched.meta.get("source_id")
                              if matched else None),
            result_message_id=matched.source_result_id if matched else None,
            agent_id=child.agent_id,
            agent_path=child.agent_path,
            agent_type=child.agent_type,
            prompt=prompt,
            status=matched.status if matched else None,
            meta={
                "parent_thread_id": child.parent_id,
                "forked_from_id": child.forked_from_id,
                "depth": child.meta.get("depth"),
            },
        )
        sess.children.append(child)
        sess.agent_edges.append(edge)
        _attach_tree(child, by_parent, seen)


def read(path: str, sessions_dir: str | Path | None = None) -> Session:
    """Read one rollout and recursively load its descendants from the same root."""
    rollout = Path(path).expanduser().resolve()
    index = _rollout_index(rollout, sessions_dir)
    root = _read_one(rollout)
    sessions = {root.source_id: root}
    reachable = {root.source_id}
    while True:
        added = False
        for current_id, (candidate, ident) in index.items():
            if current_id in reachable or ident["parent_id"] not in reachable:
                continue
            reachable.add(current_id)
            sessions[current_id] = _read_one(candidate)
            added = True
        if not added:
            break
    by_parent: dict[str, list[Session]] = {}
    for candidate in sessions.values():
        if candidate.parent_id:
            by_parent.setdefault(candidate.parent_id, []).append(candidate)
    _attach_tree(root, by_parent, set())
    return root
