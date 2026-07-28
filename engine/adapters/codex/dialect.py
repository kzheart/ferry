"""Codex 工具方言(读端归一)。

Codex 的写端是记录信封级的定制渲染(exec 事件对),不走 render;
这份方言只负责把 rollout 里的 function_call 归一成规范操作。
shell 家族四个名字共享一个解码钩子:command 可能是字符串或
["bash", "-lc", ...] 列表,timeout 字段名也有两种写法。
"""
from ...sessions.tool_ops import CanonicalOp
from ..shared.dialect import FieldMap, OpBinding, ToolDialect


def decode_shell(args: dict) -> dict | None:
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


DIALECT = ToolDialect(
    adapter="codex",
    namespace="codex",
    strict_input=True,
    bindings=(
        OpBinding(CanonicalOp.SHELL_EXEC, "shell",
                  (FieldMap("command"), FieldMap("workdir"),
                   FieldMap("timeout_ms"), FieldMap("background")),
                  read_names=("shell_command", "exec", "exec_command"),
                  decode_hook=decode_shell),
        OpBinding(CanonicalOp.FS_READ, "read_file", (
            FieldMap("file_path", "path", read_default=""),
            FieldMap("offset", "start_line"),
            FieldMap("limit"),
        ), readonly=True),
    ),
)
