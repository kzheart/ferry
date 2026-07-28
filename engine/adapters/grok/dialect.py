"""Grok 工具方言。

本机实测(2026-07 会话)Grok 有两代工具集:
- 当前代(小写):run_terminal_command / read_file / write / search_replace /
  grep / web_fetch / web_search,参数值一律是字符串("limit": "150")。
- 旧代(PascalCase):Shell / Read / Write / StrReplace / Grep / Glob /
  WebFetch / WebSearch。
写端只产出当前代;旧代以 readonly 绑定收进来。数值字段读端 int 纠偏、
写端转回字符串以匹配原生格式。守卫策略统一 fallback:没见过的键
(如 grep 的 -A/-i 旗标)整体保留为私有调用,不做有损猜测。
"""
from ...sessions.tool_ops import CanonicalOp
from ..shared.dialect import (
    FieldMap, OpBinding, ToolDialect, inline_workdir, workdir_inline_flags,
)


DIALECT = ToolDialect(
    adapter="grok",
    namespace="grok",
    strict_input=True,
    # updates 流的 rawInput 带 variant 判别符(chat 行没有),是格式痕迹不是参数。
    drop_native=("variant",),
    bindings=(
        OpBinding(CanonicalOp.SHELL_EXEC, "run_terminal_command", (
            FieldMap("command", read_default="", write_default=""),
            FieldMap("description"),
            FieldMap("timeout_ms", "timeout", decode="int", encode="str"),
            FieldMap("background", "is_background", decode="bool"),
        ), extras="fallback",
           encode_post=inline_workdir, encode_post_fields=("workdir",),
           render_flags=workdir_inline_flags),
        OpBinding(CanonicalOp.SHELL_EXEC, "Shell", (
            FieldMap("command", read_default=""),
            FieldMap("description"),
            FieldMap("workdir", "working_directory"),
            FieldMap("timeout_ms", "block_until_ms", decode="int"),
        ), extras="fallback", readonly=True),
        OpBinding(CanonicalOp.FS_READ, "read_file", (
            FieldMap("file_path", "target_file", read_default="",
                     write_default=""),
            FieldMap("offset", decode="int", encode="str"),
            FieldMap("limit", decode="int", encode="str"),
        ), extras="fallback"),
        OpBinding(CanonicalOp.FS_READ, "Read", (
            FieldMap("file_path", "path", read_default=""),
            FieldMap("offset", decode="int"),
            FieldMap("limit", decode="int"),
        ), extras="fallback", readonly=True),
        OpBinding(CanonicalOp.FS_WRITE, "write", (
            FieldMap("file_path", read_default="", write_default=""),
            FieldMap("content", read_default="", write_default=""),
        ), extras="fallback"),
        OpBinding(CanonicalOp.FS_WRITE, "Write", (
            FieldMap("file_path", "path", read_default=""),
            FieldMap("content", "contents", read_default=""),
        ), extras="fallback", readonly=True),
        OpBinding(CanonicalOp.FS_EDIT, "search_replace", (
            FieldMap("file_path", read_default="", write_default=""),
            FieldMap("old", "old_string", read_default="", write_default=""),
            FieldMap("new", "new_string", read_default="", write_default=""),
            FieldMap("replace_all", decode="bool"),
        ), extras="fallback"),
        OpBinding(CanonicalOp.FS_EDIT, "StrReplace", (
            FieldMap("file_path", "path", read_default=""),
            FieldMap("old", "old_string", read_default=""),
            FieldMap("new", "new_string", read_default=""),
            FieldMap("replace_all"),
        ), extras="fallback", readonly=True),
        OpBinding(CanonicalOp.FS_SEARCH, "grep", (
            FieldMap("query", "pattern", read_default="", write_default=""),
            FieldMap("path"),
            FieldMap("glob"),
            FieldMap("max_results", "head_limit", decode="int",
                     encode="str"),
        ), extras="fallback"),
        OpBinding(CanonicalOp.FS_SEARCH, "Grep", (
            FieldMap("query", "pattern", read_default=""),
            FieldMap("path"),
            FieldMap("glob"),
            FieldMap("max_results", "head_limit", decode="int"),
        ), extras="fallback", readonly=True),
        OpBinding(CanonicalOp.FS_GLOB, "Glob", (
            FieldMap("pattern", "glob_pattern", read_default=""),
            FieldMap("path", "target_directory"),
        ), extras="fallback", readonly=True),
        OpBinding(CanonicalOp.WEB_FETCH, "web_fetch", (
            FieldMap("url", read_default="", write_default=""),
        ), extras="fallback"),
        OpBinding(CanonicalOp.WEB_FETCH, "WebFetch", (
            FieldMap("url", read_default=""),
        ), extras="fallback", readonly=True),
        OpBinding(CanonicalOp.WEB_SEARCH, "web_search", (
            FieldMap("query", read_default="", write_default=""),
            FieldMap("domains", "allowed_domains"),
        ), extras="fallback"),
        # 旧代 WebSearch 的 explanation 是给人看的检索意图说明,丢弃不算损失。
        OpBinding(CanonicalOp.WEB_SEARCH, "WebSearch", (
            FieldMap("query", "search_term", read_default=""),
        ), readonly=True),
    ),
)
