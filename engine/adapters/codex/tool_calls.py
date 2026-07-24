"""Codex 当前工具调用联合类型的输入解析。"""

from __future__ import annotations

import json
import re

from ...sessions.model import Session, ToolCall
from ...sessions.tool_ops import CanonicalOp

_FS_PATCH = getattr(CanonicalOp, "FS_PATCH", "fs.patch")
_TOOL_INVOKE = getattr(CanonicalOp, "TOOL_INVOKE", "tool.invoke")
_LOCAL_SHELL_NAMES = frozenset({"shell", "exec", "exec_command"})
_PATCH_HEADER_RE = re.compile(
    r"^\*\*\* (Add|Update|Delete) File: ([^\r\n]+)$",
    re.M,
)
_PATCH_MOVE_RE = re.compile(r"^\*\*\* Move to: ([^\r\n]+)$", re.M)


def _skip_js_string(source: str, start: int) -> int:
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


def _balanced_js_argument(
    source: str,
    open_paren: int,
) -> tuple[str, int] | None:
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
                return source[open_paren + 1 : index], index + 1
        index += 1
    return None


def _scan_tool_invocations(source: str) -> list[tuple[str, str]]:
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
            index == 0 or not (source[index - 1].isalnum() or source[index - 1] in "_$")
        ):
            name_start = index + len("tools.")
            name_end = name_start
            while name_end < len(source) and (
                source[name_end].isalnum() or source[name_end] in "_$"
            ):
                name_end += 1
            cursor = name_end
            while cursor < len(source) and source[cursor].isspace():
                cursor += 1
            if name_end > name_start and cursor < len(source) and source[cursor] == "(":
                balanced = _balanced_js_argument(source, cursor)
                if balanced is not None:
                    argument, _end = balanced
                    calls.append((source[name_start:name_end], argument))
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
            "\\n": "\n",
            "\\r": "\r",
            "\\t": "\t",
            "\\\\": "\\",
            f"\\{quote}": quote,
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
            candidate = (
                candidate.get("patch_text")
                or candidate.get("patch")
                or candidate.get("input")
            )
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
        end = (
            headers[index + 1].start() if index + 1 < len(headers) else len(patch_text)
        )
        section = patch_text[header.end() : end]
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
        input={
            "operations": _patch_changes(patch_text),
            "raw_patch": patch_text,
        },
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
        "input_kind": ("string" if isinstance(value, str) else type(value).__name__),
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
    return ToolCall(
        name=name,
        op=_TOOL_INVOKE,
        input=input_value,
    )


def _shell_input(args: dict) -> dict | None:
    command = args.get("cmd")
    if command is None:
        command = args.get("command")
    if command is None:
        return None
    if isinstance(command, list):
        command = (
            " ".join(str(part) for part in command[2:])
            if command[:2] == ["bash", "-lc"]
            else " ".join(str(part) for part in command)
        )
    result = {"command": str(command)}
    for field in ("workdir", "timeout_ms", "background"):
        if field in args and args[field] is not None:
            result[field] = args[field]
    if "timeout_ms" not in result and args.get("timeout") is not None:
        result["timeout_ms"] = args["timeout"]
    return result


def parse_custom_call(payload, session: Session) -> ToolCall:
    source = payload.get("input", "")
    native_name = payload.get("name", "custom_tool")
    if native_name == "apply_patch":
        patch_text = _extract_patch_text(source)
        if patch_text is not None:
            return _patch_call(patch_text)
        session.lose("migration.apply_patch_unparsed")
        return _opaque_call(native_name, source)

    calls = _scan_tool_invocations(source) if isinstance(source, str) else []
    if len(calls) != 1:
        return _opaque_call(native_name, source, calls=calls)

    call_name, argument = calls[0]
    if call_name == "exec_command":
        args = _decode_js_value(argument)
        shell_input = _shell_input(args) if isinstance(args, dict) else None
        if shell_input is not None:
            return ToolCall(
                name="exec",
                op=CanonicalOp.SHELL_EXEC,
                input=shell_input,
            )
    elif call_name == "apply_patch":
        patch_text = _extract_patch_text(source, argument)
        if patch_text is not None:
            return _patch_call(patch_text)
        session.lose("migration.apply_patch_unparsed")
    return _opaque_call(native_name, source, calls=calls)


def json_args(raw) -> dict | str:
    if isinstance(raw, dict):
        return raw
    try:
        value = json.loads(raw or "{}")
        return value if isinstance(value, dict) else raw
    except (json.JSONDecodeError, TypeError):
        return raw or ""


def spawn_input(raw) -> dict:
    args = raw if isinstance(raw, dict) else {}
    result = {
        "description": str(args.get("description") or "migrated subagent"),
        "prompt": str(args.get("prompt") or args.get("message") or ""),
        "subagent_type": str(
            args.get("subagent_type") or args.get("agent_type") or "general"
        ),
    }
    aliases = {
        "task_name": ("task_name",),
        "model": ("model",),
        "fork_mode": ("fork_mode", "mode"),
        "fork_turns": ("fork_turns",),
        "reasoning_effort": ("reasoning_effort",),
    }
    for field, candidates in aliases.items():
        value = next(
            (
                args.get(candidate)
                for candidate in candidates
                if args.get(candidate) is not None
            ),
            None,
        )
        if value is not None:
            result[field] = str(value)
    return result


def parse_function_call(payload: dict) -> ToolCall:
    name = payload.get("name", "?")
    args = json_args(payload.get("arguments", "{}"))
    if name == "spawn_agent":
        return ToolCall(
            name="spawn_agent",
            op=CanonicalOp.AGENT_SPAWN,
            input=spawn_input(args),
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
