"""OpenCode 工具方言。"""
from ...sessions.tool_ops import CanonicalOp
from ..shared.dialect import FieldMap, OpBinding, ToolDialect
from ..shared.tool_canon import patch_operations


def _decode_patch(raw: dict) -> dict:
    patch = str(raw.get("patchText", ""))
    return {"operations": patch_operations(patch), "raw_patch": patch}


def _encode_patch(canonical: dict) -> dict | None:
    patch = canonical.get("raw_patch")
    if not patch:
        return None
    return {"patchText": patch}


def _decode_task(raw: dict) -> dict:
    return {
        "description": str(raw.get("description") or "migrated subagent"),
        "prompt": str(raw.get("prompt") or ""),
        "subagent_type": str(raw.get("subagent_type") or "general"),
    }


def _encode_task(canonical: dict) -> dict:
    return {key: canonical[key]
            for key in ("description", "prompt", "subagent_type")
            if key in canonical}


DIALECT = ToolDialect(
    adapter="opencode",
    namespace="opencode",
    bindings=(
        OpBinding(CanonicalOp.SHELL_EXEC, "bash", (
            FieldMap("command", read_default="", write_default=""),
            FieldMap("workdir"),
            FieldMap("timeout_ms", "timeout"),
            FieldMap("background", "run_in_background"),
            FieldMap("description"),
        )),
        OpBinding(CanonicalOp.FS_READ, "read", (
            FieldMap("file_path", "filePath", read_default="",
                     write_default=""),
            FieldMap("offset"),
            FieldMap("limit"),
        )),
        OpBinding(CanonicalOp.FS_WRITE, "write", (
            FieldMap("file_path", "filePath", read_default="",
                     write_default=""),
            FieldMap("content", read_default="", write_default=""),
        )),
        OpBinding(CanonicalOp.FS_EDIT, "edit", (
            FieldMap("file_path", "filePath", read_default="",
                     write_default=""),
            FieldMap("old", "oldString", read_default="", write_default=""),
            FieldMap("new", "newString", read_default="", write_default=""),
            FieldMap("replace_all", "replaceAll"),
        )),
        OpBinding(CanonicalOp.FS_PATCH, "apply_patch",
                  (FieldMap("operations"), FieldMap("raw_patch")),
                  decode_hook=_decode_patch, encode_hook=_encode_patch),
        OpBinding(CanonicalOp.FS_SEARCH, "grep", (
            FieldMap("query", "pattern", read_default="", write_default=""),
            FieldMap("path"),
            FieldMap("glob", "include"),
            FieldMap("max_results", "limit"),
        )),
        OpBinding(CanonicalOp.FS_GLOB, "glob", (
            FieldMap("pattern", read_default="", write_default=""),
            FieldMap("path"),
        )),
        OpBinding(CanonicalOp.WEB_FETCH, "webfetch", (
            FieldMap("url", read_default="", write_default=""),
            FieldMap("format"),
            FieldMap("timeout_ms", "timeout"),
        )),
        OpBinding(CanonicalOp.WEB_SEARCH, "websearch", (
            FieldMap("query", read_default="", write_default=""),
            FieldMap("num_results", "numResults"),
        )),
        OpBinding(CanonicalOp.AGENT_SPAWN, "task",
                  (FieldMap("description"), FieldMap("prompt"),
                   FieldMap("subagent_type")),
                  decode_hook=_decode_task, encode_hook=_encode_task),
    ),
)
