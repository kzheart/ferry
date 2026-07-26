"""工具调用规范化的唯一实现：op 映射表与入参归一化。

op 映射按 adapter 分表查询，不做扁平合并——各 Agent 的工具名集合不同
（如 `apply_patch`/`webfetch`/`websearch`/`glob` 只属于 opencode，`find` 只属于
pi），合并会让这些名字在别的 adapter 下从 TOOL_INVOKE 透传变成具体 op。
未知工具一律返回 None，兜底语义留给各 reader 的调用点。

入参归一化的 if 阶梯合并了 claude 与 opencode 两份实现：前者工具名为
PascalCase、后者为小写，天然不冲突。
"""
from __future__ import annotations

import re

from ...sessions.tool_ops import CanonicalOp


CLAUDE_TOOL_OPS = {
    "Bash": CanonicalOp.SHELL_EXEC,
    "Read": CanonicalOp.FS_READ,
    "Write": CanonicalOp.FS_WRITE,
    "Edit": CanonicalOp.FS_EDIT,
    "Grep": CanonicalOp.FS_SEARCH,
    "Glob": CanonicalOp.FS_GLOB,
    "WebFetch": CanonicalOp.WEB_FETCH,
    "WebSearch": CanonicalOp.WEB_SEARCH,
}

OPENCODE_TOOL_OPS = {
    "bash": CanonicalOp.SHELL_EXEC,
    "read": CanonicalOp.FS_READ,
    "write": CanonicalOp.FS_WRITE,
    "edit": CanonicalOp.FS_EDIT,
    "apply_patch": CanonicalOp.FS_PATCH,
    "grep": CanonicalOp.FS_SEARCH,
    "glob": CanonicalOp.FS_GLOB,
    "webfetch": CanonicalOp.WEB_FETCH,
    "websearch": CanonicalOp.WEB_SEARCH,
}

PI_TOOL_OPS = {
    "bash": CanonicalOp.SHELL_EXEC,
    "read": CanonicalOp.FS_READ,
    "write": CanonicalOp.FS_WRITE,
    "edit": CanonicalOp.FS_EDIT,
    "grep": CanonicalOp.FS_SEARCH,
    "find": CanonicalOp.FS_GLOB,
}

_TOOL_OPS_BY_ADAPTER = {
    "claude": CLAUDE_TOOL_OPS,
    "opencode": OPENCODE_TOOL_OPS,
    "pi": PI_TOOL_OPS,
}


def canonical_tool_op(adapter: str, tool_name: str) -> CanonicalOp | None:
    return _TOOL_OPS_BY_ADAPTER.get(adapter, {}).get(tool_name)


def patch_operations(patch: str) -> list[dict]:
    return [
        {"operation": operation.lower(), "path": path.strip()}
        for operation, path in re.findall(
            r"^\*\*\* (Add|Update|Delete) File: ([^\r\n]+)$",
            patch,
            re.MULTILINE,
        )
    ]


def canonical_tool_input(tool_name: str, raw):
    if not isinstance(raw, dict):
        return raw

    # ---- claude ----
    if tool_name == "Edit":
        value = {"file_path": raw.get("file_path", ""),
                 "old": raw.get("old_string", ""),
                 "new": raw.get("new_string", "")}
        if "replace_all" in raw:
            value["replace_all"] = raw["replace_all"]
        return value
    if tool_name == "Read":
        value = {"file_path": raw.get("file_path", "")}
        for field in ("offset", "limit"):
            if field in raw:
                value[field] = raw[field]
        return value
    if tool_name == "Write":
        return {"file_path": raw.get("file_path", ""),
                "content": raw.get("content", "")}
    if tool_name == "Bash":
        value = {"command": raw.get("command", "")}
        if "timeout" in raw:
            value["timeout_ms"] = raw["timeout"]
        if "run_in_background" in raw:
            value["background"] = raw["run_in_background"]
        if "dangerouslyDisableSandbox" in raw:
            value["sandbox_policy"] = (
                "dangerously-disable" if raw["dangerouslyDisableSandbox"]
                else "default")
        return value
    if tool_name == "Agent":
        value = {
            "description": raw.get("description", ""),
            "prompt": raw.get("prompt", ""),
            "subagent_type": raw.get("subagent_type", ""),
        }
        aliases = {
            "name": "task_name",
            "model": "model",
            "mode": "fork_mode",
            "reasoning_effort": "reasoning_effort",
        }
        for source, target in aliases.items():
            if source in raw:
                value[target] = raw[source]
        return value
    if tool_name == "Grep":
        value = {"query": raw.get("pattern", "")}
        aliases = {"path": "path", "glob": "glob", "head_limit": "max_results"}
        for source, target in aliases.items():
            if source in raw:
                value[target] = raw[source]
        return value
    if tool_name == "Glob":
        value = {"pattern": raw.get("pattern", "")}
        if "path" in raw:
            value["path"] = raw["path"]
        return value
    if tool_name == "WebFetch":
        value = {"url": raw.get("url", "")}
        if "prompt" in raw:
            value["prompt"] = raw["prompt"]
        return value
    if tool_name == "WebSearch":
        value = {"query": raw.get("query", "")}
        if "allowed_domains" in raw:
            value["domains"] = raw["allowed_domains"]
        return value

    # ---- opencode ----
    if tool_name == "task":
        return {
            "description": str(raw.get("description") or "migrated subagent"),
            "prompt": str(raw.get("prompt") or ""),
            "subagent_type": str(raw.get("subagent_type") or "general"),
        }
    if tool_name == "bash":
        value = {"command": raw.get("command", "")}
        if "workdir" in raw:
            value["workdir"] = raw["workdir"]
        if "timeout" in raw:
            value["timeout_ms"] = raw["timeout"]
        if "run_in_background" in raw:
            value["background"] = raw["run_in_background"]
        return value
    if tool_name == "read":
        value = {"file_path": raw.get("filePath", "")}
        value.update(
            {
                key: raw[key]
                for key in ("offset", "limit")
                if key in raw
            }
        )
        return value
    if tool_name == "write":
        return {
            "file_path": raw.get("filePath", ""),
            "content": raw.get("content", ""),
        }
    if tool_name == "edit":
        value = {
            "file_path": raw.get("filePath", ""),
            "old": raw.get("oldString", ""),
            "new": raw.get("newString", ""),
        }
        if "replaceAll" in raw:
            value["replace_all"] = raw["replaceAll"]
        return value
    if tool_name == "apply_patch":
        patch = str(raw.get("patchText", ""))
        return {
            "operations": patch_operations(patch),
            "raw_patch": patch,
        }
    if tool_name == "grep":
        value = {"query": raw.get("pattern", "")}
        if "path" in raw:
            value["path"] = raw["path"]
        if "include" in raw:
            value["glob"] = raw["include"]
        if "limit" in raw:
            value["max_results"] = raw["limit"]
        return value
    if tool_name == "glob":
        value = {"pattern": raw.get("pattern", "")}
        if "path" in raw:
            value["path"] = raw["path"]
        return value
    if tool_name == "webfetch":
        value = {"url": raw.get("url", "")}
        if "format" in raw:
            value["format"] = raw["format"]
        if "timeout" in raw:
            value["timeout_ms"] = raw["timeout"]
        return value
    if tool_name == "websearch":
        value = {"query": raw.get("query", "")}
        if "numResults" in raw:
            value["num_results"] = raw["numResults"]
        return value

    return raw
