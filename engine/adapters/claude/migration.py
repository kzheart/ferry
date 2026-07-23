"""Claude 作为迁移目标的写入与规划能力。"""
from __future__ import annotations

from ..base.migration import MigrationTargetBase, linked_agent_edge
from ...domain.tool_ops import CanonicalOp, has_valid_tool_input
from .writer import OP_FIDELITY, write


class ClaudeMigrationTarget(MigrationTargetBase):
    tool = "claude"
    tool_fidelity = OP_FIDELITY
    tool_result_statuses = frozenset({"success", "error", "interrupted"})

    def preview_tool(self, tool, session, message=None):
        if not has_valid_tool_input(tool.op, tool.input):
            return None
        if tool.op == CanonicalOp.AGENT_SPAWN:
            if not linked_agent_edge(session, tool):
                return None
            return {"kind": "tool", "name": "Agent", "input": tool.input,
                    "output": tool.output or "", "conversion": "native",
                    "_consumed_fields": set(tool.input)}
        if tool.op == CanonicalOp.TOOL_INVOKE:
            namespace = tool.input["namespace"]
            if namespace not in {"claude", "mcp"}:
                return None
            return {
                "kind": "tool", "name": tool.input["name"],
                "input": tool.input["input"], "output": tool.output or "",
                "conversion": "native", "_fidelity": "exact",
                "_consumed_fields": set(tool.input),
            }
        mapping = {
            CanonicalOp.SHELL_EXEC: (
                "Bash", {"command", "timeout_ms", "background"},
                lambda value: {
                    "command": value["command"],
                    **({"timeout": value["timeout_ms"]}
                       if "timeout_ms" in value else {}),
                    **({"run_in_background": value["background"]}
                       if "background" in value else {}),
                }),
            CanonicalOp.FS_READ: (
                "Read", {"file_path", "offset", "limit"},
                lambda value: {key: value[key] for key in
                               ("file_path", "offset", "limit") if key in value}),
            CanonicalOp.FS_WRITE: (
                "Write", {"file_path", "content"},
                lambda value: {"file_path": value["file_path"],
                               "content": value["content"]}),
            CanonicalOp.FS_EDIT: (
                "Edit", {"file_path", "old", "new", "replace_all"},
                lambda value: {
                    "file_path": value["file_path"],
                    "old_string": value["old"], "new_string": value["new"],
                    **({"replace_all": value["replace_all"]}
                       if "replace_all" in value else {}),
                }),
            CanonicalOp.FS_SEARCH: (
                "Grep", {"query", "path", "glob"},
                lambda value: {
                    "pattern": value["query"],
                    **{key: value[key] for key in ("path", "glob")
                       if key in value},
                }),
            CanonicalOp.FS_GLOB: (
                "Glob", {"pattern", "path"},
                lambda value: {
                    "pattern": value["pattern"],
                    **({"path": value["path"]} if "path" in value else {}),
                }),
            CanonicalOp.WEB_FETCH: (
                "WebFetch", {"url", "prompt"},
                lambda value: {
                    "url": value["url"],
                    "prompt": value.get(
                        "prompt",
                        "Fetch this URL and preserve its relevant content."),
                }),
            CanonicalOp.WEB_SEARCH: (
                "WebSearch", {"query"},
                lambda value: {"query": value["query"]}),
        }
        value = mapping.get(tool.op)
        if value is None:
            return None
        name, supported, convert = value
        ignored = set(tool.input) - supported
        rendered = {"kind": "tool", "name": name, "input": convert(tool.input),
                "output": tool.output or "", "conversion": "native",
                "_consumed_fields": set(tool.input) - ignored,
                "_ignored_fields": ignored,
                "_reason_codes": ("unsupported_tool_fields",) if ignored else ()}
        if tool.op == CanonicalOp.WEB_FETCH and "prompt" not in tool.input:
            rendered["_fidelity"] = "transformed"
            rendered["_reason_codes"] = ("default_fetch_prompt",)
        return rendered

    def write(self, session, cwd: str):
        return write(session, cwd=cwd, tool_decider=self.evaluate_tool)
