"""OpenCode 作为迁移目标的写入与规划能力。"""
from __future__ import annotations

from ..base.migration import MigrationTargetBase, linked_agent_edge
from ...domain.model import tool_result_text
from ...domain.tool_ops import CanonicalOp, has_valid_tool_input
from .session import OP_FIDELITY, write


class OpenCodeMigrationTarget(MigrationTargetBase):
    tool = "opencode"
    tool_fidelity = OP_FIDELITY
    tool_result_statuses = frozenset({"success", "error", "running", "pending"})

    def preview_tool(self, tool, session, message=None):
        if not has_valid_tool_input(tool.op, tool.input):
            return None
        if tool.op == CanonicalOp.AGENT_SPAWN:
            if not linked_agent_edge(session, tool, message, allow_message=True):
                return None
            supported = {"description", "prompt", "subagent_type"}
            ignored = set(tool.input) - supported
            return {"kind": "tool", "name": "task", "input": tool.input,
                    "output": tool_result_text(tool.result),
                    "conversion": "native",
                    "_consumed_fields": set(tool.input) - ignored,
                    "_ignored_fields": ignored,
                    "_reason_codes": ("unsupported_tool_fields",) if ignored else ()}
        if tool.op == CanonicalOp.TOOL_INVOKE:
            namespace = tool.input["namespace"]
            if namespace not in {"opencode", "mcp"}:
                return None
            return {
                "kind": "tool", "name": tool.input["name"],
                "input": tool.input["input"],
                "output": tool_result_text(tool.result),
                "conversion": "native", "_fidelity": "exact",
                "_consumed_fields": set(tool.input),
            }
        mapping = {
            CanonicalOp.SHELL_EXEC: (
                "bash", {"command", "workdir", "timeout_ms", "background"},
                lambda value: {key: value[key] for key in
                               ("command", "workdir", "background")
                               if key in value} | (
                                   {"timeout": value["timeout_ms"]}
                                   if "timeout_ms" in value else {})),
            CanonicalOp.FS_READ: (
                "read", {"file_path", "offset", "limit"},
                lambda value: {
                    "filePath": value["file_path"],
                    **{key: value[key] for key in ("offset", "limit")
                       if key in value},
                }),
            CanonicalOp.FS_WRITE: (
                "write", {"file_path", "content"},
                lambda value: {"filePath": value["file_path"],
                               "content": value["content"]}),
            CanonicalOp.FS_EDIT: (
                "edit", {"file_path", "old", "new"},
                lambda value: {"filePath": value["file_path"],
                               "oldString": value["old"],
                               "newString": value["new"]}),
            CanonicalOp.FS_PATCH: (
                "apply_patch", {"operations", "raw_patch"},
                lambda value: {"patchText": value["raw_patch"]}),
            CanonicalOp.FS_SEARCH: (
                "grep", {"query", "path", "glob"},
                lambda value: {
                    "pattern": value["query"],
                    **({"path": value["path"]} if "path" in value else {}),
                    **({"include": value["glob"]} if "glob" in value else {}),
                }),
            CanonicalOp.FS_GLOB: (
                "glob", {"pattern", "path"},
                lambda value: {
                    "pattern": value["pattern"],
                    **({"path": value["path"]} if "path" in value else {}),
                }),
            CanonicalOp.WEB_FETCH: (
                "webfetch", {"url", "format", "timeout_ms"},
                lambda value: {
                    "url": value["url"],
                    **({"format": value["format"]} if "format" in value else {}),
                    **({"timeout": value["timeout_ms"]}
                       if "timeout_ms" in value else {}),
                }),
            CanonicalOp.WEB_SEARCH: (
                "websearch", {"query", "num_results"},
                lambda value: {
                    "query": value["query"],
                    **({"numResults": value["num_results"]}
                       if "num_results" in value else {}),
                }),
        }
        value = mapping.get(tool.op)
        if value is None:
            return None
        name, supported, convert = value
        if tool.op == CanonicalOp.FS_PATCH and not tool.input.get("raw_patch"):
            return None
        ignored = set(tool.input) - supported
        return {"kind": "tool", "name": name, "input": convert(tool.input),
                "output": tool_result_text(tool.result),
                "conversion": "native",
                "_consumed_fields": set(tool.input) - ignored,
                "_ignored_fields": ignored,
                "_reason_codes": ("unsupported_tool_fields",) if ignored else ()}

    def write(self, session, cwd: str):
        return write(session, cwd=cwd, tool_decider=self.evaluate_tool)
