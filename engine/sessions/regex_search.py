"""正则检索:必然字面量预过滤 + 原始转录扫描。

trigram 索引无法加速任意正则,但绝大多数实用正则含有必然出现的字面
片段(ghp_[A-Za-z0-9]{36} 必含 ghp_)。提取这些片段打到 FTS 索引缩小
候选集,再对候选会话的**原始转录**跑正则引擎——原文没有 16KB 截断,
一次实现同时补掉"无正则"与"索引盲区"两个覆盖缺口。提不出字面量的
纯高熵模式(如 [0-9a-f]{64})退化为预算内的线性扫描,这是倒排索引的
理论边界,正确答案是可用、有界、如实报告的慢路径。
"""
from __future__ import annotations

import re
from re import _parser as sre_parse

from ..errors import AgentRequestError
from .model import tool_result_text

# 与 content_index._MIN_TRIGRAM_CHARS 同源:短于 3 字符走不了 trigram。
_MIN_LITERAL_CHARS = 3
_SNIPPET_BEFORE = 120
_SNIPPET_AFTER = 240
_MATCHES_PER_SESSION = 3


def compile_regex(pattern) -> re.Pattern:
    if not isinstance(pattern, str) or not pattern.strip():
        raise AgentRequestError(
            "regex 必须是非空字符串", {"field": "regex"},
        )
    if len(pattern) > 500:
        raise AgentRequestError(
            "regex 不能超过 500 字符", {"field": "regex"},
        )
    try:
        return re.compile(pattern)
    except re.error as error:
        raise AgentRequestError(
            f"regex 无法编译: {error}", {"field": "regex"},
        )


def required_literals(pattern: str) -> list[str]:
    """提取正则里必然出现的字面片段(≥3 字符),供 trigram 预过滤。

    提取是保守的:只收必经串接路径上的连续 LITERAL;分支、可选重复、
    字符类、环视一律不贡献。提不出来返回空表,调用方退化为全量扫描
    ——预过滤只许缩小候选集,绝不许制造漏报。
    """
    try:
        tree = sre_parse.parse(pattern)
    except Exception:
        return []
    literals: list[str] = []

    def walk(nodes) -> None:
        run: list[str] = []

        def flush() -> None:
            if len(run) >= _MIN_LITERAL_CHARS:
                literals.append("".join(run))
            run.clear()

        for op, arg in nodes:
            name = str(op)
            if name == "LITERAL":
                run.append(chr(arg))
            elif name == "SUBPATTERN":
                flush()
                walk(arg[3])
            elif name in ("MAX_REPEAT", "MIN_REPEAT"):
                flush()
                minimum, _maximum, subnodes = arg
                if minimum >= 1:
                    walk(subnodes)
            else:
                # BRANCH/IN/ANY/AT/ASSERT…:不贡献,只作为字面量运行的边界。
                flush()
        flush()

    walk(tree)
    return literals


def _sources(message, include_tool_outputs: bool) -> list[str]:
    """与 content_index._extract 同口径抽正文/工具输出,但不截断。"""
    texts, tools = [], []
    for block in message.blocks:
        if block.kind == "text" and block.text:
            texts.append(block.text)
        elif include_tool_outputs and block.kind == "tool" and block.tool:
            tools.append(f"[tool {block.tool.name}]")
            output = tool_result_text(block.tool.result)
            if output:
                tools.append(output)
    sources = []
    if texts:
        sources.append("\n".join(texts))
    if tools:
        sources.append("\n".join(tools))
    return sources


def _snippet(source: str, start: int, end: int) -> str:
    left = max(0, start - _SNIPPET_BEFORE)
    right = min(len(source), end + _SNIPPET_AFTER)
    return (
        ("…" if left else "")
        + source[left:right]
        + ("…" if right < len(source) else "")
    )


def scan_session(session, compiled: re.Pattern,
                 include_tool_outputs: bool) -> tuple[int, list[dict]]:
    """对一个会话的原始消息跑正则;返回 (命中消息数, 至多 3 条命中行)。

    message/turn 编号与 content_index._session_rows 完全同口径,命中
    可直接交给 session_read from_message 跳读。
    """
    count, rows, turn = 0, [], 0
    for message_index, message in enumerate(session.messages):
        if message.role == "user":
            turn += 1
        hit = None
        for source in _sources(message, include_tool_outputs):
            hit = compiled.search(source)
            if hit is not None:
                break
        if hit is None:
            continue
        count += 1
        if len(rows) < _MATCHES_PER_SESSION:
            rows.append({
                "message": message_index + 1,
                "turn": turn,
                "role": message.role,
                "snippet": _snippet(source, hit.start(), hit.end()),
            })
    return count, rows
