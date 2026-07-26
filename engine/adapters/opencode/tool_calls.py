"""Canonical ToolCall 到 OpenCode 当前原生工具 part 的转换。"""

from __future__ import annotations

from ...sessions.model import tool_result_text
from ...sessions.tool_ops import CanonicalOp


def _write_shell_exec(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    command = inputs.get("command")
    if not command:
        return False
    native_input = {"command": command}
    if "workdir" in inputs:
        native_input["workdir"] = inputs["workdir"]
    if "timeout_ms" in inputs:
        native_input["timeout"] = inputs["timeout_ms"]
    if "background" in inputs:
        native_input["run_in_background"] = inputs["background"]
    output = tool_result_text(tool.result)
    return add_tool_part(
        "bash",
        native_input,
        output,
        command,
        {
            "output": output,
            "exit": (
                tool.result.exit_code
                if (tool.result and tool.result.exit_code is not None)
                else 0
            ),
            "truncated": bool(tool.result and tool.result.truncated),
        },
        tool,
    )


def _write_fs_read(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    path = inputs.get("file_path")
    if not path:
        return False
    native_input = {"filePath": path}
    native_input.update(
        {key: inputs[key] for key in ("offset", "limit") if key in inputs}
    )
    return add_tool_part(
        "read",
        native_input,
        tool_result_text(tool.result),
        path,
        {"truncated": bool(tool.result and tool.result.truncated)},
        tool,
    )


def _write_fs_write(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    path = inputs.get("file_path")
    if not path:
        return False
    return add_tool_part(
        "write",
        {
            "filePath": path,
            "content": inputs.get("content", ""),
        },
        tool_result_text(tool.result) or "Wrote file successfully.",
        path,
        {
            "filepath": path,
            "exists": False,
            "truncated": False,
            "diagnostics": {},
        },
        tool,
    )


def _write_fs_edit(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    path = inputs.get("file_path")
    if not path:
        return False
    return add_tool_part(
        "edit",
        {
            "filePath": path,
            "oldString": inputs.get("old", ""),
            "newString": inputs.get("new", ""),
        },
        tool_result_text(tool.result) or "Edited file.",
        path,
        {"truncated": False},
        tool,
    )


def _write_fs_patch(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    patch = inputs.get("raw_patch")
    if not patch:
        return False
    return add_tool_part(
        "apply_patch",
        {"patchText": patch},
        tool_result_text(tool.result) or "Applied patch.",
        "apply patch",
        {"truncated": False},
        tool,
    )


def _renaming_writer(native_name: str, required: tuple[str, str],
                     optional: tuple[tuple[str, str], ...] = ()):
    """canonical 输入到 opencode 原生输入只差字段改名的一类工具。"""
    source_key, native_key = required

    def write(add_tool_part, tool) -> bool:
        inputs = tool.input if isinstance(tool.input, dict) else {}
        value = inputs.get(source_key)
        if not value:
            return False
        native_input = {native_key: value}
        for name, native in optional:
            if name in inputs:
                native_input[native] = inputs[name]
        return add_tool_part(
            native_name,
            native_input,
            tool_result_text(tool.result),
            str(value),
            {"truncated": False},
            tool,
        )

    return write


_write_fs_search = _renaming_writer(
    "grep", ("query", "pattern"), (("path", "path"), ("glob", "include")))
_write_fs_glob = _renaming_writer(
    "glob", ("pattern", "pattern"), (("path", "path"),))
_write_web_fetch = _renaming_writer(
    "webfetch", ("url", "url"),
    (("format", "format"), ("timeout_ms", "timeout")))
_write_web_search = _renaming_writer(
    "websearch", ("query", "query"), (("num_results", "numResults"),))


def _write_tool_invoke(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    name = inputs.get("name")
    native_input = inputs.get("input")
    if not name or not isinstance(native_input, (dict, str)):
        return False
    return add_tool_part(
        str(name),
        native_input,
        tool_result_text(tool.result),
        str(name),
        {"historical": True, "truncated": False},
        tool,
    )


OP_WRITERS = {
    CanonicalOp.SHELL_EXEC: _write_shell_exec,
    CanonicalOp.FS_READ: _write_fs_read,
    CanonicalOp.FS_WRITE: _write_fs_write,
    CanonicalOp.FS_EDIT: _write_fs_edit,
    CanonicalOp.FS_PATCH: _write_fs_patch,
    CanonicalOp.FS_SEARCH: _write_fs_search,
    CanonicalOp.FS_GLOB: _write_fs_glob,
    CanonicalOp.WEB_FETCH: _write_web_fetch,
    CanonicalOp.WEB_SEARCH: _write_web_search,
    CanonicalOp.TOOL_INVOKE: _write_tool_invoke,
}

OP_FIDELITY = {operation: "native" for operation in OP_WRITERS} | {
    CanonicalOp.AGENT_SPAWN: "native",
}
