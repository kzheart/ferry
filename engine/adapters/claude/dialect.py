"""Claude Code 工具方言。"""
from ...sessions.tool_ops import CanonicalOp
from ..shared.dialect import (
    FieldMap, OpBinding, ToolDialect, inline_workdir, workdir_inline_flags,
)


def _webfetch_flags(canonical: dict, _native: dict) -> dict:
    if "prompt" in canonical:
        return {}
    return {"_fidelity": "transformed",
            "_reason_codes": ("default_fetch_prompt",)}


DIALECT = ToolDialect(
    adapter="claude",
    namespace="claude",
    bindings=(
        OpBinding(CanonicalOp.SHELL_EXEC, "Bash", (
            FieldMap("command", read_default="", write_default=""),
            FieldMap("timeout_ms", "timeout"),
            FieldMap("background", "run_in_background"),
            FieldMap("sandbox_policy", "dangerouslyDisableSandbox",
                     decode="sandbox_flag", encode="sandbox_unflag"),
            FieldMap("description"),
        ), encode_post=inline_workdir, encode_post_fields=("workdir",),
           render_flags=workdir_inline_flags),
        OpBinding(CanonicalOp.FS_READ, "Read", (
            FieldMap("file_path", read_default="", write_default=""),
            FieldMap("offset"),
            FieldMap("limit"),
        )),
        OpBinding(CanonicalOp.FS_WRITE, "Write", (
            FieldMap("file_path", read_default="", write_default=""),
            FieldMap("content", read_default="", write_default=""),
        )),
        OpBinding(CanonicalOp.FS_EDIT, "Edit", (
            FieldMap("file_path", read_default="", write_default=""),
            FieldMap("old", "old_string", read_default="", write_default=""),
            FieldMap("new", "new_string", read_default="", write_default=""),
            FieldMap("replace_all"),
        )),
        OpBinding(CanonicalOp.FS_SEARCH, "Grep", (
            FieldMap("query", "pattern", read_default="", write_default=""),
            FieldMap("path"),
            FieldMap("glob"),
            FieldMap("max_results", "head_limit"),
        )),
        OpBinding(CanonicalOp.FS_GLOB, "Glob", (
            FieldMap("pattern", read_default="", write_default=""),
            FieldMap("path"),
        )),
        OpBinding(CanonicalOp.WEB_FETCH, "WebFetch", (
            FieldMap("url", read_default="", write_default=""),
            FieldMap("prompt", write_default=(
                "Fetch this URL and preserve its relevant content.")),
        ), render_flags=_webfetch_flags),
        OpBinding(CanonicalOp.WEB_SEARCH, "WebSearch", (
            FieldMap("query", read_default="", write_default=""),
            FieldMap("domains", "allowed_domains"),
        )),
        OpBinding(CanonicalOp.AGENT_SPAWN, "Agent", (
            FieldMap("description", read_default="", write_default=""),
            FieldMap("prompt", read_default="", write_default=""),
            FieldMap("subagent_type", read_default=""),
            FieldMap("task_name", "name"),
            FieldMap("model"),
            FieldMap("fork_mode", "mode"),
            FieldMap("reasoning_effort"),
        )),
    ),
)
