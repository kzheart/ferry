"""Codex reader: current rollout JSONL → canonical session model."""
import json
import re
import sqlite3
from pathlib import Path

from ...domain.errors import AgentFormatChangedError
from ...domain.model import (
    AgentEdge,
    Block,
    ContextCompaction,
    Message,
    Session,
    ToolCall,
    ToolResult,
    ToolResultBlock,
    tool_result_text,
)
from ...domain.reasoning import codex_summary_text
from ...domain.tool_ops import CanonicalOp
from ...infrastructure.scan_cache import ScanCache
from ..base.media import image_from_data_url

_META_CACHE_PATH = Path.home() / ".resume-harness" / "rollout-meta-cache.json"

_FS_PATCH = getattr(CanonicalOp, "FS_PATCH", "fs.patch")
_TOOL_INVOKE = getattr(CanonicalOp, "TOOL_INVOKE", "tool.invoke")
_LOCAL_SHELL_NAMES = frozenset({"shell", "exec", "exec_command"})
_PATCH_HEADER_RE = re.compile(
    r"^\*\*\* (Add|Update|Delete) File: ([^\r\n]+)$", re.M)
_PATCH_MOVE_RE = re.compile(r"^\*\*\* Move to: ([^\r\n]+)$", re.M)
_SKIP_USER_PREFIX = ("<environment_context>", "<user_instructions>",
                      "<ENVIRONMENT_CONTEXT>", "<turn_aborted>")
_RESULT_STATUS = {
    "success": "success",
    "completed": "success",
    "error": "error",
    "interrupted": "interrupted",
    "running": "running",
    "pending": "pending",
    "unknown": "unknown",
}


def session_id(meta: dict, fallback: str) -> str:
    del fallback
    return str(meta["id"])


def _result_status(value) -> str:
    if not isinstance(value, str):
        return "unknown"
    return _RESULT_STATUS.get(value, "unknown")


def _skip_js_string(source: str, start: int) -> int:
    """Return the first index after one JS string literal."""
    quote = source[start]
    index = start + 1
    while index < len(source):
        if source[index] == "\\":
            index += 2
            continue
        if source[index] == quote:
            return index + 1
        index += 1
    return len(source)


def _skip_js_comment(source: str, start: int) -> int:
    if source.startswith("//", start):
        newline = source.find("\n", start + 2)
        return len(source) if newline < 0 else newline + 1
    if source.startswith("/*", start):
        end = source.find("*/", start + 2)
        return len(source) if end < 0 else end + 2
    return start


def _balanced_js_argument(source: str, open_paren: int) -> tuple[str, int] | None:
    depth = 1
    index = open_paren + 1
    while index < len(source):
        char = source[index]
        if char in "\"'`":
            index = _skip_js_string(source, index)
            continue
        if source.startswith("//", index) or source.startswith("/*", index):
            index = _skip_js_comment(source, index)
            continue
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return source[open_paren + 1:index], index + 1
        index += 1
    return None


def _scan_tool_invocations(source: str) -> list[tuple[str, str]]:
    """Scan tools.<name>(...) calls without treating quoted braces as syntax."""
    calls = []
    index = 0
    while index < len(source):
        char = source[index]
        if char in "\"'`":
            index = _skip_js_string(source, index)
            continue
        if source.startswith("//", index) or source.startswith("/*", index):
            index = _skip_js_comment(source, index)
            continue
        if source.startswith("tools.", index) and (
                index == 0 or not (source[index - 1].isalnum() or
                                   source[index - 1] in "_$")):
            name_start = index + len("tools.")
            name_end = name_start
            while name_end < len(source) and (
                    source[name_end].isalnum() or source[name_end] in "_$"):
                name_end += 1
            cursor = name_end
            while cursor < len(source) and source[cursor].isspace():
                cursor += 1
            if name_end > name_start and cursor < len(source) and source[cursor] == "(":
                balanced = _balanced_js_argument(source, cursor)
                if balanced is not None:
                    argument, _end = balanced
                    calls.append((source[name_start:name_end], argument))
                    # Continue inside the argument so syntactically nested
                    # tools.* calls are also represented in a composite.
                    index = cursor + 1
                    continue
        index += 1
    return calls


def _decode_js_value(source: str):
    source = source.strip()
    try:
        return json.loads(source)
    except (json.JSONDecodeError, TypeError):
        pass
    if len(source) >= 2 and source[0] in "'`" and source[-1] == source[0]:
        quote = source[0]
        body = source[1:-1]
        replacements = {
            "\\n": "\n", "\\r": "\r", "\\t": "\t",
            "\\\\": "\\", f"\\{quote}": quote,
        }
        for escaped, value in replacements.items():
            body = body.replace(escaped, value)
        return body
    return source


def _js_string_values(source: str):
    index = 0
    while index < len(source):
        if source[index] in "\"'`":
            end = _skip_js_string(source, index)
            yield _decode_js_value(source[index:end])
            index = end
            continue
        if source.startswith("//", index) or source.startswith("/*", index):
            index = _skip_js_comment(source, index)
            continue
        index += 1


def _extract_patch_text(source, argument=None) -> str | None:
    candidates = []
    if argument is not None:
        candidates.append(_decode_js_value(argument))
    if isinstance(source, str):
        decoded_source = _decode_js_value(source)
        if decoded_source != source:
            candidates.append(decoded_source)
        candidates.extend(_js_string_values(source))
    candidates.append(source)
    for candidate in candidates:
        if isinstance(candidate, dict):
            candidate = (candidate.get("patch_text") or candidate.get("patch") or
                         candidate.get("input"))
        if isinstance(candidate, str) and "*** Begin Patch" in candidate:
            start = candidate.index("*** Begin Patch")
            end = candidate.find("*** End Patch", start)
            if end >= 0:
                end += len("*** End Patch")
                return candidate[start:end]
            return candidate[start:]
    return None


def _patch_changes(patch_text: str) -> list[dict]:
    headers = list(_PATCH_HEADER_RE.finditer(patch_text))
    changes = []
    for index, header in enumerate(headers):
        operation = header.group(1).lower()
        path = header.group(2).strip()
        end = headers[index + 1].start() if index + 1 < len(headers) else len(patch_text)
        section = patch_text[header.end():end]
        move = _PATCH_MOVE_RE.search(section) if operation == "update" else None
        change = {
            "operation": "move" if move else operation,
            "path": path,
            "hunk_count": len(re.findall(r"^@@", section, re.M)),
        }
        if move:
            change["destination"] = move.group(1).strip()
        changes.append(change)
    return changes


def _patch_call(patch_text: str) -> ToolCall:
    return ToolCall(
        name="apply_patch",
        op=_FS_PATCH,
        input={"operations": _patch_changes(patch_text), "raw_patch": patch_text},
    )


def _input_summary(name: str, argument: str) -> dict:
    value = _decode_js_value(argument)
    if isinstance(value, dict):
        return {
            "native_name": name,
            "input_kind": "object",
            "input_fields": sorted(str(key) for key in value),
        }
    return {
        "native_name": name,
        "input_kind": "string" if isinstance(value, str) else type(value).__name__,
        "input_fields": [],
    }


def _opaque_call(name: str, native_input, *, calls=()) -> ToolCall:
    input_value = {
        "namespace": "codex",
        "name": name,
        "input": native_input,
    }
    calls = list(calls)
    if calls:
        input_value["structure_summary"] = {
            "kind": "composite" if len(calls) > 1 else "single",
            "invocation_count": len(calls),
            "tool_names": [call_name for call_name, _ in calls],
        }
        input_value["children"] = [
            _input_summary(call_name, argument) for call_name, argument in calls
        ]
    return ToolCall(name=name, op=_TOOL_INVOKE, input=input_value)


def _shell_input(args: dict) -> dict | None:
    command = args.get("cmd")
    if command is None:
        command = args.get("command")
    if command is None:
        return None
    if isinstance(command, list):
        command = (" ".join(str(part) for part in command[2:])
                   if command[:2] == ["bash", "-lc"]
                   else " ".join(str(part) for part in command))
    result = {"command": str(command)}
    for field in ("workdir", "timeout_ms", "background"):
        if field in args and args[field] is not None:
            result[field] = args[field]
    if "timeout_ms" not in result and args.get("timeout") is not None:
        result["timeout_ms"] = args["timeout"]
    return result


def _parse_call(payload, sess) -> ToolCall:
    src = payload.get("input", "")
    native_name = payload.get("name", "custom_tool")
    if native_name == "apply_patch":
        patch_text = _extract_patch_text(src)
        if patch_text is not None:
            return _patch_call(patch_text)
        sess.lose("migration.apply_patch_unparsed")
        return _opaque_call(native_name, src)

    calls = _scan_tool_invocations(src) if isinstance(src, str) else []
    if len(calls) != 1:
        return _opaque_call(native_name, src, calls=calls)

    call_name, argument = calls[0]
    if call_name == "exec_command":
        args = _decode_js_value(argument)
        shell_input = _shell_input(args) if isinstance(args, dict) else None
        if shell_input is not None:
            return ToolCall(name="exec", op=CanonicalOp.SHELL_EXEC,
                            input=shell_input)
    elif call_name == "apply_patch":
        patch_text = _extract_patch_text(src, argument)
        if patch_text is not None:
            return _patch_call(patch_text)
        sess.lose("migration.apply_patch_unparsed")
    return _opaque_call(native_name, src, calls=calls)


def _parse_result(raw) -> ToolResult:
    """Decode Codex output envelopes without flattening status or rich blocks."""
    blocks = []
    stdout = stderr = None
    exit_code = None
    truncated = None
    attachments = []
    explicit_status = None
    structured_envelope = False
    wrapper_blocks = []
    try:
        native_blocks = raw if isinstance(raw, list) else json.loads(raw)
    except (json.JSONDecodeError, TypeError):
        native_blocks = raw
    if isinstance(native_blocks, dict):
        native_blocks = [native_blocks]
    elif not isinstance(native_blocks, list):
        native_blocks = [{"type": "input_text",
                          "text": native_blocks if isinstance(native_blocks, str)
                          else str(native_blocks)}]

    for native_block in native_blocks:
        if not isinstance(native_block, dict):
            blocks.append(ToolResultBlock("json", data=native_block))
            continue
        kind = native_block.get("type")
        if kind in {"input_text", "output_text", "text"}:
            text = native_block.get("text", "")
            try:
                inner = json.loads(text)
            except (json.JSONDecodeError, TypeError):
                inner = None
            if isinstance(inner, dict) and any(
                    key in inner for key in (
                        "output", "stdout", "stderr", "exit_code", "status",
                        "truncated", "attachments")):
                structured_envelope = True
                output = inner.get("output")
                stdout_value = inner.get("stdout", output)
                if isinstance(stdout_value, str):
                    stdout = stdout_value
                if isinstance(output, str) and output:
                    blocks.append(ToolResultBlock("text", text=output))
                elif output is not None:
                    blocks.append(ToolResultBlock("json", data=output))
                if isinstance(inner.get("stderr"), str):
                    stderr = inner["stderr"]
                code = inner.get("exit_code")
                if isinstance(code, int) and not isinstance(code, bool):
                    exit_code = code
                if isinstance(inner.get("truncated"), bool):
                    truncated = inner["truncated"]
                if isinstance(inner.get("attachments"), list):
                    attachments = inner["attachments"]
                explicit_status = inner.get("status")
            elif text:
                block = ToolResultBlock("text", text=text)
                blocks.append(block)
                if text.startswith("Script completed\nWall time "):
                    wrapper_blocks.append(block)
        elif kind in {"input_image", "output_image", "image"}:
            blocks.append(ToolResultBlock(
                "image",
                uri=native_block.get("image_url") or native_block.get("url"),
                data=native_block.get("data"),
                mime_type=native_block.get("mime_type"),
            ))
        elif kind == "file":
            blocks.append(ToolResultBlock(
                "file", uri=native_block.get("url"),
                filename=native_block.get("filename"),
                mime_type=native_block.get("mime_type"),
            ))
        else:
            blocks.append(ToolResultBlock("json", data=native_block))

    status = _result_status(explicit_status)
    if structured_envelope:
        wrapper_ids = {id(block) for block in wrapper_blocks}
        blocks = [block for block in blocks if id(block) not in wrapper_ids]
    if status == "unknown" and exit_code is not None:
        status = "success" if exit_code == 0 else "error"
    if stderr and status == "unknown":
        status = "error"
    return ToolResult(
        status=status, blocks=blocks, stdout=stdout, stderr=stderr,
        exit_code=exit_code, truncated=truncated, attachments=attachments,
    )


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
    result = {
        "description": str(args.get("description") or "migrated subagent"),
        "prompt": str(args.get("prompt") or args.get("message") or ""),
        "subagent_type": str(args.get("subagent_type") or
                             args.get("agent_type") or "general"),
    }
    aliases = {
        "task_name": ("task_name",),
        "model": ("model",),
        "fork_mode": ("fork_mode", "mode"),
        "fork_turns": ("fork_turns",),
        "reasoning_effort": ("reasoning_effort",),
    }
    for field, candidates in aliases.items():
        value = next((args.get(candidate) for candidate in candidates
                      if args.get(candidate) is not None), None)
        if value is not None:
            result[field] = str(value)
    return result


def _function_call(payload: dict) -> ToolCall:
    name = payload.get("name", "?")
    args = _json_args(payload.get("arguments", "{}"))
    if name == "spawn_agent":
        return ToolCall(
            name="spawn_agent",
            op=CanonicalOp.AGENT_SPAWN,
            input=_spawn_input(args),
            source_call_id=payload.get("call_id"),
        )
    if name in _LOCAL_SHELL_NAMES and isinstance(args, dict):
        shell_input = _shell_input(args)
        if shell_input is not None:
            return ToolCall(
                name=name,
                op=CanonicalOp.SHELL_EXEC,
                input=shell_input,
                source_call_id=payload.get("call_id"),
            )
    return ToolCall(
        name=name,
        op=_TOOL_INVOKE,
        input={
            "namespace": "codex",
            "name": name,
            "input": args,
        },
        source_call_id=payload.get("call_id"),
    )


def _subagent_meta(meta: dict) -> dict:
    source = meta.get("source")
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
        "agent_nickname": (
            spawn.get("agent_nickname") or meta.get("agent_nickname")
        ),
        "agent_role": spawn.get("agent_role") or meta.get("agent_role"),
        "model_provider": meta.get("model_provider"),
        "model": meta.get("model"),
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


def _load_records(path: Path) -> list[dict]:
    records = []
    for line_number, line in enumerate(path.read_text().splitlines(), start=1):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            records.append({
                "type": "__resume_harness_malformed_jsonl__",
                "line_number": line_number,
                "error": error.msg,
            })
            continue
        if isinstance(value, dict):
            records.append(value)
        else:
            records.append({
                "type": "__resume_harness_malformed_record__",
                "line_number": line_number,
                "error": "record is not an object",
            })
    return records


def _codex_compaction(record: dict, ordinal: int,
                      after_message_id: str | None) -> ContextCompaction:
    payload = record.get("payload") or {}
    summary = payload.get("message")
    summary = summary.strip() if isinstance(summary, str) else ""
    replacement = payload.get("replacement_history")
    replacement = replacement if isinstance(replacement, list) else []
    encrypted = any(
        isinstance(item, dict) and item.get("type") == "compaction"
        and isinstance(item.get("encrypted_content"), str)
        and bool(item["encrypted_content"])
        for item in replacement
    )
    summary_status = (
        "available" if summary else "protected" if encrypted else "missing"
    )
    window_id = payload.get("window_id")
    return ContextCompaction(
        id=str(window_id or f"record:{ordinal}"),
        source="codex",
        after_message_id=after_message_id,
        event_locator=f"record:{ordinal}",
        created_at=record.get("timestamp"),
        state="completed",
        summary_status=summary_status,
        summary_text=summary,
        source_meta={
            "replacement_history_present": bool(replacement),
            "replacement_item_count": len(replacement),
            "window_number": payload.get("window_number"),
            "first_window_id": payload.get("first_window_id"),
            "previous_window_id": payload.get("previous_window_id"),
            "window_id": window_id,
        },
    )


def _read_one(path: Path, meta: dict | None = None) -> Session:
    lines = _load_records(Path(path))
    response_payload_types = {
        "message", "reasoning",
        "function_call", "function_call_output",
        "custom_tool_call", "custom_tool_call_output",
    }
    for line_number, record in enumerate(lines, start=1):
        record_type = record.get("type")
        if record_type in response_payload_types:
            raise AgentFormatChangedError(
                "codex",
                f"jsonl[{line_number}].type",
                "response_item with payload.type",
                record_type,
            )
    meta = meta or next((l.get("payload") or {} for l in lines
                         if l.get("type") == "session_meta"), {})
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
    sess.agent_nickname = ident["agent_nickname"]
    sess.agent_role = ident["agent_role"]
    sess.model_provider = ident["model_provider"]
    sess.model = ident["model"]
    sess.depth = ident["depth"]
    sess.parent_association = (
        "parent-metadata" if ident["parent_id"] else None
    )
    for record in lines:
        if record.get("type") in {
                "__resume_harness_malformed_jsonl__",
                "__resume_harness_malformed_record__"}:
            sess.lose(
                "session.malformed_record",
                line_number=record["line_number"],
                error=record["error"],
            )
    pending: dict[str, ToolCall] = {}
    cur_tools: list[Block] = []          # 未落消息的工具块,附到下一条 assistant
    cur_reasoning: list[Block] = []      # 可见 reasoning 降级为 text,附到下一条 assistant

    def flush_pending_into(blocks, message_source_id: str | None = None):
        nonlocal cur_tools, cur_reasoning
        for block in cur_tools:
            if (
                block.tool
                and message_source_id
                and block.tool.source_message_id is None
            ):
                block.tool.source_message_id = message_source_id
        blocks[:0] = cur_reasoning + cur_tools
        cur_tools = []
        cur_reasoning = []

    for ordinal, l in enumerate(lines):
        record_type = l.get("type")
        if record_type == "compacted":
            after_message_id = next((
                message.source_id for message in reversed(sess.messages)
                if message.source_id), None)
            sess.context_compactions.append(
                _codex_compaction(l, ordinal, after_message_id))
            continue
        if record_type == "response_item":
            p = l.get("payload") or {}
        else:
            continue
        pt = p.get("type")
        if pt == "message":
            content = p.get("content", [])
            if isinstance(content, str):
                content = [{"type": "input_text" if p.get("role") == "user"
                            else "output_text", "text": content}]
            texts = [c.get("text", "") for c in content
                     if isinstance(c, dict) and
                     c.get("type") in ("input_text", "output_text")]
            text = "\n".join(t for t in texts if t)
            image_blocks = []
            for content_index, item in enumerate(content):
                if not isinstance(item, dict):
                    continue
                if item.get("type") != "input_image":
                    continue
                image = image_from_data_url(
                    f"record:{ordinal}:image:{content_index}", item.get("image_url", ""))
                if image is None:
                    sess.lose("migration.unknown_block_dropped", kind="input_image")
                else:
                    image_blocks.append(Block("image", image=image))
            role = p.get("role")
            if role == "user" and text.strip().startswith(_SKIP_USER_PREFIX):
                continue
            if role == "user" and (cur_tools or cur_reasoning):
                pending_blocks = []
                source_id = f"record:{ordinal}"
                flush_pending_into(pending_blocks, source_id)
                sess.messages.append(Message(role="assistant", blocks=pending_blocks,
                                             source_id=source_id,
                                             created_at=l.get("timestamp")))
            if not text.strip() and not image_blocks and not cur_tools and not cur_reasoning:
                continue
            blocks = ([Block("text", text)] if text.strip() else []) + image_blocks
            if role == "assistant":
                flush_pending_into(blocks, f"record:{ordinal}")
            sess.messages.append(Message(role=role, blocks=blocks,
                                         source_id=f"record:{ordinal}",
                                         created_at=l.get("timestamp")))
        elif pt in ("custom_tool_call", "function_call"):
            if pt == "function_call":
                tc = _function_call(p)
            elif p.get("name") == "spawn_agent":
                tc = ToolCall(name="spawn_agent", op=CanonicalOp.AGENT_SPAWN,
                              input=_spawn_input(_json_args(p.get("input", ""))))
            else:
                tc = _parse_call(p, sess)
            tc.source_call_id = p.get("call_id")
            if tc.op == CanonicalOp.AGENT_SPAWN:
                tc.source_message_id = next((
                    message.source_id for message in reversed(sess.messages)
                    if message.role in {"user", "assistant"}), None)
            pending[p.get("call_id")] = tc
            cur_tools.append(Block("tool", tool=tc))
        elif pt in ("custom_tool_call_output", "function_call_output"):
            tc = pending.pop(p.get("call_id"), None)
            if tc is not None:
                tc.result = _parse_result(p.get("output", ""))
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
        sess.messages.append(Message(role="assistant", blocks=blocks))
    candidates = [
        compaction for compaction in sess.context_compactions
        if compaction.source_meta.get("replacement_history_present")
    ]
    if candidates:
        candidates[-1].source_meta["active"] = True
    return sess


def _spawn_calls(sess: Session) -> list[ToolCall]:
    return [block.tool for message in sess.messages for block in message.blocks
            if block.kind == "tool" and block.tool and
            block.tool.op == CanonicalOp.AGENT_SPAWN]


def _contains_identity(tool: ToolCall, child: Session) -> bool:
    values = [child.source_id, child.agent_id, child.agent_path]
    haystack = json.dumps({
        "input": tool.input,
        "output": tool_result_text(tool.result),
    },
                          ensure_ascii=False)
    return any(value and value in haystack for value in values)


def _attach_tree(sess: Session, by_parent: dict[str, list[Session]], seen: set[str],
                 edge_statuses: dict[str, str]):
    if sess.source_id in seen:
        return
    seen.add(sess.source_id)
    spawn_calls = _spawn_calls(sess)
    candidates = list(by_parent.get(sess.source_id, []))
    ordered_children = []
    selected_children: set[int] = set()
    # 优先按父 rollout 中 spawn_agent 的物理顺序恢复 siblings。writer 的
    # tool output 包含 agent_path，因此即使目录遍历顺序不同也能
    # 找回原顺序；无法关联的旧记录再走稳定排序兜底。
    for tool in spawn_calls:
        child = next((candidate for candidate in candidates
                      if id(candidate) not in selected_children and
                      _contains_identity(tool, candidate)), None)
        if child is not None:
            selected_children.add(id(child))
            ordered_children.append(child)
    ordered_children.extend(candidate for candidate in candidates
                            if id(candidate) not in selected_children)
    used_calls: set[int] = set()
    for child in ordered_children:
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
            spawn_message_id=matched.source_message_id if matched else None,
            result_message_id=matched.source_result_id if matched else None,
            agent_id=child.agent_id,
            agent_path=child.agent_path,
            agent_type=child.agent_type,
            prompt=prompt,
            status=(
                _canonical_edge_status(matched.result.status)
                if matched and matched.result else None
            ) or edge_statuses.get(child.source_id),
            association=(
                "spawn-call" if matched else
                child.parent_association or "parent-metadata"),
            confidence=(
                1.0 if matched else 0.95
                if child.parent_association == "sqlite-parent"
                else 0.75),
        )
        sess.children.append(child)
        sess.agent_edges.append(edge)
        _attach_tree(child, by_parent, seen, edge_statuses)


def _registry_edges(sessions_root: Path) -> dict[str, tuple[str, str]]:
    db_path = sessions_root.parent / "state_5.sqlite"
    if not db_path.exists():
        return {}
    try:
        with sqlite3.connect(f"file:{db_path.resolve()}?mode=ro", uri=True) as db:
            return {
                str(child): (str(parent), str(status))
                for parent, child, status in db.execute(
                    "SELECT parent_thread_id, child_thread_id, status "
                    "FROM thread_spawn_edges")
            }
    except sqlite3.Error:
        return {}


def _canonical_edge_status(value: str | None) -> str | None:
    if value in {"open", "closed"}:
        return value
    if value in {"completed", "failed", "cancelled", "canceled"}:
        return "closed"
    if value in {"in_progress", "queued"}:
        return "open"
    return None


def read(path: str, sessions_dir: str | Path | None = None) -> Session:
    """Read one rollout and recursively load its descendants from the same root."""
    rollout = Path(path).expanduser().resolve()
    index = _rollout_index(rollout, sessions_dir)
    root = _read_one(rollout)
    registry_edges = _registry_edges(_sessions_root(rollout))
    sessions = {root.source_id: root}
    reachable = {root.source_id}
    while True:
        added = False
        for current_id, (candidate, ident) in index.items():
            registry_parent = registry_edges.get(current_id, (None, None))[0]
            parent_id = ident["parent_id"] or registry_parent
            if current_id in reachable or parent_id not in reachable:
                continue
            reachable.add(current_id)
            child = _read_one(candidate)
            if child.parent_id is None and registry_parent:
                child.parent_id = registry_parent
                child.parent_association = "sqlite-parent"
            sessions[current_id] = child
            added = True
        if not added:
            break
    by_parent: dict[str, list[Session]] = {}
    for candidate in sessions.values():
        if candidate.parent_id:
            by_parent.setdefault(candidate.parent_id, []).append(candidate)
    for children in by_parent.values():
        children.sort(key=lambda child: (child.agent_path or "", child.source_id))
    _attach_tree(
        root, by_parent, set(),
        {child: status for child, (_parent, status) in registry_edges.items()},
    )
    return root
