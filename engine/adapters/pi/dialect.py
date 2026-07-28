"""Pi 工具方言。

pi 的守卫语义:bash/grep/find 遇到表外字段整体退回 TOOL_INVOKE(fallback),
read/write 只取已知字段(ignore)。edit 的原生形态是 edits 列表,只有单元素
且仅含 oldText/newText 时可无损归一,其余保留原样。
"""
from ...sessions.tool_ops import CanonicalOp
from ..shared.dialect import (
    FieldMap, OpBinding, ToolDialect, inline_workdir, workdir_inline_flags,
)


def _decode_edit(raw: dict) -> dict | None:
    edits = raw.get("edits")
    if (set(raw) <= {"path", "edits"} and isinstance(edits, list)
            and len(edits) == 1 and isinstance(edits[0], dict)
            and set(edits[0]) <= {"oldText", "newText"}):
        return {
            "file_path": raw.get("path", ""),
            "old": edits[0].get("oldText", ""),
            "new": edits[0].get("newText", ""),
        }
    return None


def _encode_edit(canonical: dict) -> dict:
    return {"path": canonical.get("file_path", ""), "edits": [{
        "oldText": canonical.get("old", ""),
        "newText": canonical.get("new", ""),
    }]}


DIALECT = ToolDialect(
    adapter="pi",
    namespace="pi",
    strict_input=True,
    bindings=(
        OpBinding(CanonicalOp.SHELL_EXEC, "bash", (
            FieldMap("command", read_default="", write_default=""),
            FieldMap("timeout_ms", "timeout",
                     decode="s_to_ms", encode="ms_to_s"),
        ), extras="fallback",
           encode_post=inline_workdir, encode_post_fields=("workdir",),
           render_flags=workdir_inline_flags),
        OpBinding(CanonicalOp.FS_READ, "read", (
            FieldMap("file_path", "path", read_alt=("file_path",),
                     read_default="", write_default=""),
            FieldMap("offset"),
            FieldMap("limit"),
        )),
        OpBinding(CanonicalOp.FS_WRITE, "write", (
            FieldMap("file_path", "path", read_alt=("file_path",),
                     read_default="", write_default=""),
            FieldMap("content"),
        )),
        OpBinding(CanonicalOp.FS_EDIT, "edit",
                  (FieldMap("file_path"), FieldMap("old"), FieldMap("new")),
                  decode_hook=_decode_edit, encode_hook=_encode_edit),
        OpBinding(CanonicalOp.FS_SEARCH, "grep", (
            FieldMap("query", "pattern", read_default="", write_default=""),
            FieldMap("path"),
            FieldMap("glob"),
            FieldMap("max_results", "limit"),
        ), extras="fallback"),
        OpBinding(CanonicalOp.FS_GLOB, "find", (
            FieldMap("pattern", read_default="", write_default=""),
            FieldMap("path"),
        ), extras="fallback"),
    ),
)
