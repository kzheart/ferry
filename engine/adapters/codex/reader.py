"""Codex reader: current rollout JSONL → canonical session model.

Rollout 是 append-only 的 JSONL。解析器把「字节偏移 + 解析器状态」一起缓存:
文件追加时只解析新尾部并增量并入已有 Session,活跃大会话的重读从整文件
重解析(秒级)降到只处理增量(毫秒级)。任何前缀不一致(inode 更换、
截断、尾部窗口比对失败)都会回退全量重解析。
"""

import json
import os
import threading
from collections import OrderedDict
from pathlib import Path

from ...errors import AgentFormatChangedError
from ...sessions.model import (
    Block,
    ContextCompaction,
    Message,
    Session,
    ToolCall,
)
from ...sessions.reasoning import visible_text
from ...sessions.tool_ops import CanonicalOp
from ..shared.media import image_from_data_url
from . import tool_calls, tool_results, topology


def _summary_text(payload: dict) -> str | None:
    """从 Codex 私有的 reasoning.summary 结构提取可读摘要。"""
    summary = payload.get("summary") or []
    if isinstance(summary, str):
        return visible_text(summary)
    if not isinstance(summary, list):
        return None
    parts = []
    for item in summary:
        if isinstance(item, dict):
            text = item.get("text") or ""
            if isinstance(text, str) and text.strip():
                parts.append(text)
        elif isinstance(item, str) and item.strip():
            parts.append(item)
    return "\n".join(parts) if parts else None

_SKIP_USER_PREFIX = (
    "<environment_context>",
    "<user_instructions>",
    "<ENVIRONMENT_CONTEXT>",
    "<turn_aborted>",
)

_RESPONSE_PAYLOAD_TYPES = {
    "message",
    "reasoning",
    "function_call",
    "function_call_output",
    "custom_tool_call",
    "custom_tool_call_output",
}


class _RestartParse(Exception):
    """增量前提被打破(前缀变化/迟到的 session_meta),需要全量重解析。"""


def _complete_span(data: bytes) -> int:
    """可安全消费的字节数:最后一个换行之后若是完整 JSON 也一并消费。

    写入中的半行(JSON 对象的前缀必然解析失败)留给下一轮,避免把
    正在落盘的记录误报成 malformed。
    """
    end = data.rfind(b"\n") + 1
    tail = data[end:]
    if tail:
        try:
            json.loads(tail)
        except ValueError:
            return end
        return len(data)
    return end


def _batch_records(chunk: bytes, start_line: int) -> tuple[list[dict], int]:
    """把完整字节段切成记录;返回 (records, 行数)。"""
    lines = chunk.split(b"\n")
    if chunk.endswith(b"\n"):
        lines.pop()
    records = []
    for line_number, raw in enumerate(lines, start=start_line):
        if not raw.strip():
            continue
        try:
            value = json.loads(raw)
        except json.JSONDecodeError as error:
            records.append(
                {
                    "type": "__ferry_malformed_jsonl__",
                    "line_number": line_number,
                    "error": error.msg,
                }
            )
            continue
        if isinstance(value, dict):
            records.append(value)
        else:
            records.append(
                {
                    "type": "__ferry_malformed_record__",
                    "line_number": line_number,
                    "error": "record is not an object",
                }
            )
    return records, len(lines)


def _codex_compaction(
    record: dict, ordinal: int, after_message_id: str | None
) -> ContextCompaction:
    payload = record.get("payload") or {}
    summary = payload.get("message")
    summary = summary.strip() if isinstance(summary, str) else ""
    replacement = payload.get("replacement_history")
    replacement = replacement if isinstance(replacement, list) else []
    encrypted = any(
        isinstance(item, dict)
        and item.get("type") == "compaction"
        and isinstance(item.get("encrypted_content"), str)
        and bool(item["encrypted_content"])
        for item in replacement
    )
    summary_status = "available" if summary else "protected" if encrypted else "missing"
    window_id = payload.get("window_id")
    return ContextCompaction(
        id=str(window_id or f"record:{ordinal}"),
        source="codex",
        after_message_id=after_message_id,
        event_locator=f"record:{ordinal}",
        created_at=record.get("timestamp"),
        state="completed",
        summary_status=summary_status,
        summary_text=summary,
        source_meta={
            "replacement_history_present": bool(replacement),
            "replacement_item_count": len(replacement),
            "window_number": payload.get("window_number"),
            "first_window_id": payload.get("first_window_id"),
            "previous_window_id": payload.get("previous_window_id"),
            "window_id": window_id,
        },
    )


class _RolloutParser:
    """可增量喂入记录批次的 rollout 解析器。

    view() 返回同一个 Session 对象:先按 baseline 撤销上一次视图化
    (合成的尾部 assistant 消息)与树装配的 append 累积,再重新视图化。
    """

    def __init__(self, path: Path):
        self._path = Path(path)
        self.sess: Session | None = None
        self._saw_meta = False
        self._pending: dict[str, ToolCall] = {}
        self._cur_tools: list[Block] = []
        self._cur_reasoning: list[Block] = []
        self._ordinal = 0
        self._line_count = 0
        self._baseline = None
        # 增量缓存簿记(由 _RolloutCache 维护)
        self.offset = 0
        self.node: tuple[int, int] | None = None
        self.mtime_ns = 0
        self.size = 0
        self.window = b""

    # ---- 批次喂入 ----

    def feed_bytes(self, chunk: bytes, meta_override: dict | None = None) -> None:
        records, lines = _batch_records(chunk, self._line_count + 1)
        self._line_count += lines
        self.feed(records, meta_override=meta_override)

    def feed(self, records: list[dict], meta_override: dict | None = None) -> None:
        for record in records:
            if record.get("type") in _RESPONSE_PAYLOAD_TYPES:
                raise AgentFormatChangedError(
                    "codex",
                    "jsonl[].type",
                    "response_item with payload.type",
                    record.get("type"),
                )
        batch_has_meta = any(
            record.get("type") == "session_meta" for record in records
        )
        if self.sess is None:
            meta = meta_override or next(
                (
                    record.get("payload") or {}
                    for record in records
                    if record.get("type") == "session_meta"
                ),
                {},
            )
            self._saw_meta = batch_has_meta or meta_override is not None
            self._create_session(meta)
        elif batch_has_meta and not self._saw_meta:
            # 首批没等到 meta 却先建了会话:身份可能算错,推倒重来。
            raise _RestartParse
        for record in records:
            if record.get("type") in {
                "__ferry_malformed_jsonl__",
                "__ferry_malformed_record__",
            }:
                self.sess.lose(
                    "session.malformed_record",
                    line_number=record["line_number"],
                    error=record["error"],
                )
        for record in records:
            self._apply(record)
            self._ordinal += 1

    def _create_session(self, meta: dict) -> None:
        ident = topology.identity(meta, self._path.stem)
        sess = Session(
            source_tool="codex", source_id=ident["id"], cwd=meta.get("cwd", ""),
        )
        sess.root_id = ident["root_id"]
        sess.parent_id = ident["parent_id"]
        sess.forked_from_id = ident["forked_from_id"]
        sess.agent_id = ident["agent_id"]
        sess.agent_path = ident["agent_path"]
        sess.agent_type = ident["agent_type"]
        sess.agent_nickname = ident["agent_nickname"]
        sess.agent_role = ident["agent_role"]
        sess.model_provider = ident["model_provider"]
        sess.model = ident["model"]
        sess.depth = ident["depth"]
        sess.parent_association = "parent-metadata" if ident["parent_id"] else None
        self.sess = sess

    def _flush_pending_into(self, blocks, message_source_id: str | None = None):
        for block in self._cur_tools:
            if (
                block.tool
                and message_source_id
                and block.tool.source_message_id is None
            ):
                block.tool.source_message_id = message_source_id
        blocks[:0] = self._cur_reasoning + self._cur_tools
        self._cur_tools = []
        self._cur_reasoning = []

    def _apply(self, record: dict) -> None:
        sess = self.sess
        ordinal = self._ordinal
        record_type = record.get("type")
        if record_type == "compacted":
            after_message_id = next(
                (
                    message.source_id
                    for message in reversed(sess.messages)
                    if message.source_id
                ),
                None,
            )
            sess.context_compactions.append(
                _codex_compaction(record, ordinal, after_message_id)
            )
            return
        if record_type == "response_item":
            p = record.get("payload") or {}
        else:
            return
        pt = p.get("type")
        if pt == "message":
            content = p.get("content", [])
            if isinstance(content, str):
                content = [
                    {
                        "type": "input_text"
                        if p.get("role") == "user"
                        else "output_text",
                        "text": content,
                    }
                ]
            texts = [
                c.get("text", "")
                for c in content
                if isinstance(c, dict)
                and c.get("type") in ("input_text", "output_text")
            ]
            text = "\n".join(t for t in texts if t)
            image_blocks = []
            for content_index, item in enumerate(content):
                if not isinstance(item, dict):
                    continue
                if item.get("type") != "input_image":
                    continue
                image = image_from_data_url(
                    f"record:{ordinal}:image:{content_index}", item.get("image_url", "")
                )
                if image is None:
                    sess.lose("migration.unknown_block_dropped", kind="input_image")
                else:
                    image_blocks.append(Block("image", image=image))
            role = p.get("role")
            if role == "user" and text.strip().startswith(_SKIP_USER_PREFIX):
                return
            if role == "user" and (self._cur_tools or self._cur_reasoning):
                pending_blocks = []
                source_id = f"record:{ordinal}"
                self._flush_pending_into(pending_blocks, source_id)
                sess.messages.append(
                    Message(
                        role="assistant",
                        blocks=pending_blocks,
                        source_id=source_id,
                        created_at=record.get("timestamp"),
                    )
                )
            if (
                not text.strip()
                and not image_blocks
                and not self._cur_tools
                and not self._cur_reasoning
            ):
                return
            blocks = ([Block("text", text)] if text.strip() else []) + image_blocks
            if role == "assistant":
                self._flush_pending_into(blocks, f"record:{ordinal}")
            sess.messages.append(
                Message(
                    role=role,
                    blocks=blocks,
                    source_id=f"record:{ordinal}",
                    created_at=record.get("timestamp"),
                )
            )
        elif pt in ("custom_tool_call", "function_call"):
            if pt == "function_call":
                tc = tool_calls.parse_function_call(p)
            elif p.get("name") == "spawn_agent":
                tc = ToolCall(
                    name="spawn_agent",
                    op=CanonicalOp.AGENT_SPAWN,
                    input=tool_calls.spawn_input(
                        tool_calls.json_args(p.get("input", ""))
                    ),
                )
            else:
                tc = tool_calls.parse_custom_call(p, sess)
            tc.source_call_id = p.get("call_id")
            if tc.op == CanonicalOp.AGENT_SPAWN:
                tc.source_message_id = next(
                    (
                        message.source_id
                        for message in reversed(sess.messages)
                        if message.role in {"user", "assistant"}
                    ),
                    None,
                )
            self._pending[p.get("call_id")] = tc
            self._cur_tools.append(Block("tool", tool=tc))
        elif pt in ("custom_tool_call_output", "function_call_output"):
            tc = self._pending.pop(p.get("call_id"), None)
            if tc is not None:
                tc.result = tool_results.parse_result(p.get("output", ""))
                tc.source_result_id = p.get("id")
            else:
                sess.lose("session.orphan_tool_result", call_id=p.get("call_id"))
        elif pt == "reasoning":
            text = _summary_text(p)
            if text is not None:
                self._cur_reasoning.append(Block("text", text))
                sess.lose(
                    "migration.reasoning_metadata_dropped",
                    metadata_kind="encrypted_content",
                )
            else:
                sess.lose(
                    "migration.reasoning_dropped", metadata_kind="encrypted_content"
                )
        else:
            sess.lose("migration.unknown_block_dropped", kind=pt)

    # ---- 视图化 ----

    def snapshot(self) -> None:
        """记录纯解析基线:视图化与树装配的可变痕迹都能由此撤销。"""
        sess = self.sess
        self._baseline = (
            len(sess.messages),
            list(sess.children),
            list(sess.agent_edges),
            list(sess.loss),
            sess.parent_id,
            sess.parent_association,
            sess.root_id,
        )

    def restore(self) -> None:
        (
            messages_len, children, edges, loss,
            parent_id, association, root_id,
        ) = self._baseline
        sess = self.sess
        del sess.messages[messages_len:]
        sess.children = list(children)
        sess.agent_edges = list(edges)
        sess.loss = list(loss)
        sess.parent_id = parent_id
        sess.parent_association = association
        sess.root_id = root_id

    def view(self) -> Session:
        """把解析状态定稿成可对外返回的 Session(可重复调用)。

        尾部未落消息的工具/思考块合成一条 assistant 消息追加在末尾;
        该消息属于视图,restore() 时会被截掉,后续增量不会重复累积。
        """
        sess = self.sess
        if self._cur_tools or self._cur_reasoning:
            sess.messages.append(
                Message(
                    role="assistant",
                    blocks=list(self._cur_reasoning) + list(self._cur_tools),
                )
            )
        candidates = [
            compaction
            for compaction in sess.context_compactions
            if compaction.source_meta.get("replacement_history_present")
        ]
        for compaction in candidates:
            compaction.source_meta.pop("active", None)
        if candidates:
            candidates[-1].source_meta["active"] = True
        return sess


def _read_one(path: Path, meta: dict | None = None) -> Session:
    """单文件全量解析(无缓存):测试与回退路径使用。"""
    path = Path(path)
    parser = _RolloutParser(path)
    parser.feed_bytes(path.read_bytes(), meta_override=meta)
    return parser.view()


_WINDOW = 4096
_CACHE_MAX_ENTRIES = 256
_CACHE_MAX_TOTAL_BYTES = 512 * 1024 * 1024


class _RolloutCache:
    """rollout → 增量解析器的 LRU 缓存。

    命中判定基于 stat:同 inode 且只增长才走增量(追加前先比对偏移前的
    尾部窗口,防截断重写);其余一律全量重解析。同长度的中段原地改写无法
    从 stat 察觉,但没有已知写入方这么做,且编辑路径始终走严格指纹校验。
    """

    def __init__(self):
        self._entries: OrderedDict[str, _RolloutParser] = OrderedDict()
        self._lock = threading.Lock()

    def clear(self) -> None:
        with self._lock:
            self._entries.clear()

    def read(self, path: Path) -> Session:
        key = str(path)
        try:
            stat = os.stat(key)
        except OSError:
            # 路径不可 stat(消失中/测试桩):不缓存,直接读。
            with self._lock:
                self._entries.pop(key, None)
            return _read_one(path)
        with self._lock:
            parser = self._entries.get(key)
        if parser is not None:
            try:
                return self._advance(parser, key, stat)
            except _RestartParse:
                pass
        parser = self._full_parse(path, stat)
        with self._lock:
            self._entries[key] = parser
            self._entries.move_to_end(key)
            self._evict()
        return parser.view()

    def _advance(self, parser: _RolloutParser, key: str, stat) -> Session:
        node = (stat.st_dev, stat.st_ino)
        if node != parser.node or stat.st_size < parser.offset:
            raise _RestartParse
        if stat.st_size == parser.size and stat.st_mtime_ns == parser.mtime_ns:
            parser.restore()
            with self._lock:
                if key in self._entries:
                    self._entries.move_to_end(key)
            return parser.view()
        if stat.st_size == parser.offset:
            # 只有 mtime 变了(touch/元数据变更),内容前提不可信。
            raise _RestartParse
        with open(key, "rb") as stream:
            stream.seek(parser.offset - len(parser.window))
            if stream.read(len(parser.window)) != parser.window:
                raise _RestartParse
            data = stream.read()
        span = _complete_span(data)
        parser.restore()
        if span:
            parser.feed_bytes(data[:span])
            parser.offset += span
            tail = data[:span][-_WINDOW:]
            parser.window = (parser.window + tail)[-_WINDOW:]
            parser.snapshot()
        parser.mtime_ns = stat.st_mtime_ns
        parser.size = stat.st_size
        return parser.view()

    def _full_parse(self, path: Path, stat) -> _RolloutParser:
        parser = _RolloutParser(path)
        data = Path(path).read_bytes()
        span = _complete_span(data)
        parser.feed_bytes(data[:span])
        parser.offset = span
        parser.node = (stat.st_dev, stat.st_ino)
        parser.mtime_ns = stat.st_mtime_ns
        parser.size = stat.st_size
        parser.window = data[:span][-_WINDOW:]
        parser.snapshot()
        return parser

    def _evict(self) -> None:
        while len(self._entries) > _CACHE_MAX_ENTRIES or (
            sum(entry.size for entry in self._entries.values())
            > _CACHE_MAX_TOTAL_BYTES
            and len(self._entries) > 1
        ):
            self._entries.popitem(last=False)


_PARSE_CACHE = _RolloutCache()
# 树装配会修改共享的缓存对象,同一根会话的并发读取必须互斥。
_TREE_LOCKS: dict[str, threading.Lock] = {}
_TREE_LOCKS_GUARD = threading.Lock()


def _cached_read_one(path: Path) -> Session:
    return _PARSE_CACHE.read(Path(path))


def read(path: str, sessions_dir: str | Path | None = None) -> Session:
    """Read one rollout and recursively load its descendants from the same root."""
    rollout = Path(path).expanduser().resolve()
    with _TREE_LOCKS_GUARD:
        lock = _TREE_LOCKS.setdefault(str(rollout), threading.Lock())
    with lock:
        return topology.read_tree(rollout, _cached_read_one, sessions_dir)
