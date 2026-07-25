"""跨会话正文的持久化全文索引。

SQLite FTS5 + trigram 分词:中英文与代码都按任意子串命中(与 Signal /
微信 / JetBrains 同路线)。索引以 revision 对账做增量——每次搜索只重建
内容真正变过的会话,其余零 IO;首次全量构建在后台线程完成,期间搜索
返回部分结果并如实上报覆盖度。
"""
from __future__ import annotations

import logging
import os
import re
import sqlite3
import threading
import time
from pathlib import Path

from .agent_read import read_indexed_session
from .index import AgentSessionIndex, IndexedSession
from .model import tool_result_text

log = logging.getLogger(__name__)

_SCHEMA_VERSION = 1
# trigram 至少要 3 个字符才能走倒排;更短的查询回退为子串扫描。
_MIN_TRIGRAM_CHARS = 3
# 单条消息两列各自的入索引上限。超长的工具输出(整文件 dump)截断进索引,
# 会话内穷尽检索仍可用 session_read terms= 全文扫描兜底。
_RECORD_TEXT_CAP = 16_000
# 待更新集小于这个规模就同步补完再查:agent 刚写完的会话必须马上可搜,
# 陈旧索引会让模型断言"没提过"然后白跑(Cursor 对 agent 检索的同一结论)。
_SYNC_SESSION_LIMIT = 32
_SYNC_BYTE_LIMIT = 32 * 1024 * 1024
# 病态高频词的行数上限;命中即在 DTO 里标注 rows_capped。
_MAX_MATCH_ROWS = 50_000
_SNIPPET_BEFORE = 120
_SNIPPET_AFTER = 240
_MATCHES_PER_SESSION = 3


def _extract(message) -> tuple[str, str]:
    """一条消息拆成正文列与工具输出列,分列才能支持检索范围过滤。"""
    texts, tools = [], []
    for block in message.blocks:
        if block.kind == "text" and block.text:
            texts.append(block.text)
        elif block.kind == "tool" and block.tool:
            tools.append(f"[tool {block.tool.name}]")
            output = tool_result_text(block.tool.result)
            if output:
                tools.append(output)
    return (
        "\n".join(texts)[:_RECORD_TEXT_CAP],
        "\n".join(tools)[:_RECORD_TEXT_CAP],
    )


def _session_rows(session) -> list[tuple]:
    """message/turn 编号与 session_read 完全同口径,命中可直接跳读。"""
    rows, turn = [], 0
    for message_index, message in enumerate(session.messages):
        if message.role == "user":
            turn += 1
        text, tool_text = _extract(message)
        if not text and not tool_text:
            continue
        rows.append((message_index + 1, turn, message.role, text, tool_text))
    return rows


def _like_pattern(needle: str) -> str:
    escaped = (
        needle.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")
    )
    return f"%{escaped}%"


def parse_query_terms(query: str) -> list[str]:
    """把查询拆成词级 AND 的检索词;支持 "..." 精确短语。

    实测里模型会很自然地敲 `全文检索 content index` 或 `a OR b` 这样的
    查询——按整串子串匹配必然 0 命中,然后模型据此给出错误的否定结论。
    这里按空白拆词(同一条消息内全部命中才算数),布尔操作符不支持,
    裸的 OR/AND/NOT 按噪声丢弃。
    """
    terms = []
    for quoted, plain in re.findall(r'"([^"]+)"|(\S+)', query):
        term = quoted or plain
        if term in ("OR", "AND", "NOT"):
            continue
        terms.append(term)
    return terms or [query.strip()]


class ContentIndex:
    """内容索引的生命周期:建库、增量同步、查询与摘要。

    连接懒打开;FTS5/trigram 不可用时整个特性显式降级,搜索照常返回
    元数据结果并在 content_index 状态里说明原因。
    """

    def __init__(self, path: Path | None = None):
        self._path = (
            Path(path) if path is not None
            else Path.home() / ".resume-harness" / "content-index.sqlite3"
        )
        self._connection: sqlite3.Connection | None = None
        self._unavailable: str | None = None
        self._db_lock = threading.RLock()
        self._worker: threading.Thread | None = None
        self._queued: dict[tuple[str, str], IndexedSession] = {}
        self._worker_lock = threading.Lock()
        self._closed = False

    # ---------- 连接与 schema ----------

    def _db(self) -> sqlite3.Connection | None:
        with self._db_lock:
            if self._unavailable is not None or self._closed:
                return None
            if self._connection is None:
                try:
                    self._path.parent.mkdir(parents=True, exist_ok=True)
                    connection = sqlite3.connect(
                        self._path, check_same_thread=False, timeout=5.0,
                    )
                    os.chmod(self._path, 0o600)
                    connection.execute("PRAGMA journal_mode=WAL")
                    self._ensure_schema(connection)
                except (OSError, sqlite3.Error) as error:
                    self._unavailable = f"content index unavailable: {error}"
                    log.warning("内容索引不可用: %s", error)
                    return None
                self._connection = connection
            return self._connection

    @staticmethod
    def _ensure_schema(connection: sqlite3.Connection) -> None:
        version = connection.execute("PRAGMA user_version").fetchone()[0]
        if version != _SCHEMA_VERSION:
            connection.executescript(
                "DROP TABLE IF EXISTS records_fts;"
                "DROP TABLE IF EXISTS records;"
                "DROP TABLE IF EXISTS indexed_sessions;"
            )
        connection.executescript(
            """
            CREATE TABLE IF NOT EXISTS indexed_sessions(
                tool TEXT NOT NULL,
                ref TEXT NOT NULL,
                revision TEXT NOT NULL,
                record_rows INTEGER NOT NULL DEFAULT 0,
                failed INTEGER NOT NULL DEFAULT 0,
                indexed_at INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(tool, ref)
            );
            CREATE TABLE IF NOT EXISTS records(
                id INTEGER PRIMARY KEY,
                tool TEXT NOT NULL,
                ref TEXT NOT NULL,
                message INTEGER NOT NULL,
                turn INTEGER NOT NULL,
                role TEXT NOT NULL,
                text TEXT NOT NULL DEFAULT '',
                tool_text TEXT NOT NULL DEFAULT ''
            );
            CREATE INDEX IF NOT EXISTS records_by_session
                ON records(tool, ref);
            CREATE VIRTUAL TABLE IF NOT EXISTS records_fts USING fts5(
                text, tool_text,
                content='records', content_rowid='id',
                tokenize='trigram'
            );
            CREATE TRIGGER IF NOT EXISTS records_fts_insert
            AFTER INSERT ON records BEGIN
                INSERT INTO records_fts(rowid, text, tool_text)
                VALUES (new.id, new.text, new.tool_text);
            END;
            CREATE TRIGGER IF NOT EXISTS records_fts_delete
            AFTER DELETE ON records BEGIN
                INSERT INTO records_fts(records_fts, rowid, text, tool_text)
                VALUES ('delete', old.id, old.text, old.tool_text);
            END;
            """
        )
        connection.execute(f"PRAGMA user_version = {_SCHEMA_VERSION}")
        connection.commit()

    def close(self) -> None:
        with self._worker_lock:
            self._queued.clear()
            worker = self._worker
        if worker is not None and worker.is_alive():
            worker.join(timeout=5)
        with self._db_lock:
            self._closed = True
            if self._connection is not None:
                self._connection.close()
                self._connection = None

    # ---------- 增量同步 ----------

    def sync(self, index: AgentSessionIndex,
             records: list[IndexedSession], *,
             prefer_background: bool = False) -> dict:
        """按 revision 对账并返回覆盖度状态。

        小变更同步补完(保证刚写入的内容立即可搜);大变更/首建转后台,
        调用方拿到 pending_sessions 用于告知模型内容结果是部分的。
        """
        if self._db() is None:
            return {"ready": False, "reason": self._unavailable}
        stored = self._stored_revisions()
        current: set[tuple[str, str]] = set()
        pending: list[IndexedSession] = []
        for record in records:
            key = (record.tool, record.canonical_ref)
            current.add(key)
            if stored.get(key) != record.revision:
                pending.append(record)
        stale = [key for key in stored if key not in current]
        if stale:
            self._drop_sessions(stale)
        if pending:
            total_bytes = sum(
                int(record.row.get("size") or 0) for record in pending
            )
            if (not prefer_background
                    and not self._building()
                    and len(pending) <= _SYNC_SESSION_LIMIT
                    and total_bytes <= _SYNC_BYTE_LIMIT):
                for record in pending:
                    self._index_session(index, record)
                pending = []
            else:
                self._enqueue(index, pending)
        return {
            "ready": not pending and not self._building(),
            "indexed_sessions": len(records) - len(pending),
            "pending_sessions": len(pending),
        }

    def wait_until_idle(self, timeout: float = 60.0) -> bool:
        """等后台构建收敛;测试与预热用,请求路径不等。"""
        deadline = time.monotonic() + timeout
        while self._building():
            if time.monotonic() >= deadline:
                return False
            time.sleep(0.02)
        return True

    def _stored_revisions(self) -> dict[tuple[str, str], str]:
        with self._db_lock:
            connection = self._db()
            if connection is None:
                return {}
            rows = connection.execute(
                "SELECT tool, ref, revision FROM indexed_sessions",
            ).fetchall()
        return {(tool, ref): revision for tool, ref, revision in rows}

    def _drop_sessions(self, keys: list[tuple[str, str]]) -> None:
        with self._db_lock:
            connection = self._db()
            if connection is None:
                return
            with connection:
                for tool, ref in keys:
                    connection.execute(
                        "DELETE FROM records WHERE tool = ? AND ref = ?",
                        (tool, ref),
                    )
                    connection.execute(
                        "DELETE FROM indexed_sessions"
                        " WHERE tool = ? AND ref = ?",
                        (tool, ref),
                    )

    def _index_session(self, index: AgentSessionIndex,
                       record: IndexedSession) -> None:
        # 解析在锁外做,大会话的读取不能阻塞并发查询。
        try:
            rows = _session_rows(read_indexed_session(index, record))
            failed = 0
        except Exception:
            # 读取失败(会话正在被写入等)按当前 revision 记账,不空转重试;
            # 文件再变化时 revision 变了自然会重进队列。
            rows, failed = [], 1
        with self._db_lock:
            connection = self._db()
            if connection is None:
                return
            with connection:
                connection.execute(
                    "DELETE FROM records WHERE tool = ? AND ref = ?",
                    (record.tool, record.canonical_ref),
                )
                connection.executemany(
                    "INSERT INTO records"
                    "(tool, ref, message, turn, role, text, tool_text)"
                    " VALUES (?, ?, ?, ?, ?, ?, ?)",
                    [
                        (record.tool, record.canonical_ref, *row)
                        for row in rows
                    ],
                )
                connection.execute(
                    "INSERT OR REPLACE INTO indexed_sessions"
                    "(tool, ref, revision, record_rows, failed, indexed_at)"
                    " VALUES (?, ?, ?, ?, ?, ?)",
                    (record.tool, record.canonical_ref, record.revision,
                     len(rows), failed, int(time.time())),
                )

    def _enqueue(self, index: AgentSessionIndex,
                 records: list[IndexedSession]) -> None:
        with self._worker_lock:
            if self._closed:
                return
            for record in records:
                self._queued[(record.tool, record.canonical_ref)] = record
            if self._worker is None or not self._worker.is_alive():
                self._worker = threading.Thread(
                    target=self._drain, args=(index,),
                    daemon=True, name="content-index",
                )
                self._worker.start()

    def _drain(self, index: AgentSessionIndex) -> None:
        while True:
            with self._worker_lock:
                if not self._queued or self._closed:
                    return
                _key, record = self._queued.popitem()
            try:
                self._index_session(index, record)
            except sqlite3.Error:
                if self._closed:
                    return
                log.exception("内容索引后台构建失败: %s", record.canonical_ref)

    def _building(self) -> bool:
        with self._worker_lock:
            worker = self._worker
            return bool(self._queued) or (
                worker is not None and worker.is_alive()
            )

    # ---------- 查询 ----------

    def search(self, needle: str,
               include_tool_outputs: bool = False) -> tuple[dict, dict]:
        """返回 (按 (tool, canonical_ref) 聚合的命中, 查询元信息)。

        每个命中桶带 count/best_rank 与至多 3 条按相关性排序的消息行;
        摘要文本延迟到最终返回页再取。
        """
        if self._db() is None:
            return {}, {"match_mode": None, "rows_capped": False}
        terms = parse_query_terms(needle)
        long_terms = [t for t in terms if len(t) >= _MIN_TRIGRAM_CHARS]
        short_terms = [t for t in terms if len(t) < _MIN_TRIGRAM_CHARS]
        if long_terms:
            rows, mode = self._match_trigram(
                long_terms, short_terms, include_tool_outputs,
            )
        else:
            rows, mode = self._match_substring(
                short_terms, include_tool_outputs,
            )
        capped = len(rows) > _MAX_MATCH_ROWS
        hits: dict[tuple[str, str], dict] = {}
        for row_id, tool, ref, message, turn, role, rank in \
                rows[:_MAX_MATCH_ROWS]:
            bucket = hits.setdefault(
                (tool, ref),
                {"count": 0, "best_rank": rank, "rows": []},
            )
            bucket["count"] += 1
            if len(bucket["rows"]) < _MATCHES_PER_SESSION:
                bucket["rows"].append({
                    "id": row_id,
                    "message": message,
                    "turn": turn,
                    "role": role,
                })
        return hits, {"match_mode": mode, "rows_capped": capped}

    @staticmethod
    def _short_term_filter(short_terms: list[str],
                           include_tool_outputs: bool,
                           alias: str = "") -> tuple[str, list]:
        """不够 trigram 长度的词以 LIKE 补充 AND 条件。"""
        clauses, params = [], []
        for term in short_terms:
            pattern = _like_pattern(term)
            if include_tool_outputs:
                clauses.append(
                    f"({alias}text LIKE ? ESCAPE '\\'"
                    f" OR {alias}tool_text LIKE ? ESCAPE '\\')"
                )
                params.extend((pattern, pattern))
            else:
                clauses.append(f"{alias}text LIKE ? ESCAPE '\\'")
                params.append(pattern)
        return "".join(f" AND {clause}" for clause in clauses), params

    def _match_trigram(self, long_terms: list[str], short_terms: list[str],
                       include_tool_outputs: bool) -> tuple[list, str]:
        columns = "{text tool_text}" if include_tool_outputs else "{text}"
        # 多个短语在 FTS5 里默认 AND:同一条消息内全部命中才算数。
        phrases = " ".join(
            '"' + term.replace('"', '""') + '"' for term in long_terms
        )
        match = f"{columns}: {phrases}"
        extra, extra_params = self._short_term_filter(
            short_terms, include_tool_outputs, alias="r.",
        )
        with self._db_lock:
            connection = self._db()
            if connection is None:
                return [], "trigram"
            rows = connection.execute(
                "SELECT r.id, r.tool, r.ref, r.message, r.turn, r.role,"
                " bm25(records_fts) AS rank"
                " FROM records_fts"
                " JOIN records r ON r.id = records_fts.rowid"
                f" WHERE records_fts MATCH ?{extra}"
                " ORDER BY rank LIMIT ?",
                (match, *extra_params, _MAX_MATCH_ROWS + 1),
            ).fetchall()
        return rows, "trigram"

    def _match_substring(self, terms: list[str],
                         include_tool_outputs: bool) -> tuple[list, str]:
        extra, params = self._short_term_filter(terms, include_tool_outputs)
        with self._db_lock:
            connection = self._db()
            if connection is None:
                return [], "substring_scan"
            rows = connection.execute(
                "SELECT id, tool, ref, message, turn, role, NULL"
                f" FROM records WHERE 1=1{extra}"
                " ORDER BY id LIMIT ?",
                (*params, _MAX_MATCH_ROWS + 1),
            ).fetchall()
        return rows, "substring_scan"

    def snippet(self, row_id: int, needle: str,
                include_tool_outputs: bool) -> str:
        """定位命中上下文;返回原文窗口,脱敏由 DTO 边界负责。"""
        with self._db_lock:
            connection = self._db()
            if connection is None:
                return ""
            row = connection.execute(
                "SELECT text, tool_text FROM records WHERE id = ?",
                (row_id,),
            ).fetchone()
        if row is None:
            return ""
        sources = [row[0], row[1]] if include_tool_outputs else [row[0]]
        folded_terms = [
            term.casefold() for term in parse_query_terms(needle)
        ]
        for source in sources:
            folded = source.casefold()
            for term in folded_terms:
                position = folded.find(term)
                if position < 0:
                    continue
                start = max(0, position - _SNIPPET_BEFORE)
                end = min(len(source), position + len(term) + _SNIPPET_AFTER)
                return (
                    ("…" if start else "")
                    + source[start:end]
                    + ("…" if end < len(source) else "")
                )
        for source in sources:
            if source:
                clipped = len(source) > _SNIPPET_AFTER
                return source[:_SNIPPET_AFTER] + ("…" if clipped else "")
        return ""
