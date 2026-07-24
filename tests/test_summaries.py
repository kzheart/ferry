"""摘要底座:分段 / 内容指纹 / 缓存失效 / 摘要写回。"""

from types import SimpleNamespace

import pytest

from engine.organization import summaries
from engine.errors import SummaryBackboneMissingError
from engine.storage.database import StateDatabase


def _use_database(tmp_path, monkeypatch) -> StateDatabase:
    database = StateDatabase(
        tmp_path / "ferry-state.sqlite3", recover_interrupted=False,
    )
    monkeypatch.setattr(summaries, "_database", lambda _ports: database)
    return database


def _text_block(text):
    return SimpleNamespace(kind="text", text=text)


def _msg(role, text, source_id, with_tool=False):
    blocks = [_text_block(text)] if text else []
    if with_tool:
        blocks.append(SimpleNamespace(kind="tool", text=None))
    return SimpleNamespace(role=role, blocks=blocks, source_id=source_id, raw=[])


def _session(messages, compactions=(), source_id="sess-1", tool="claude"):
    return SimpleNamespace(source_tool=tool, source_id=source_id,
                           messages=list(messages),
                           context_compactions=list(compactions))


def test_segment_splits_by_turn_and_skips_tool_noise():
    session = _session([
        _msg("user", "帮我改支付", "u1"),
        _msg("assistant", "好的，已定位", "a1", with_tool=True),
        _msg("user", "再改下标题", "u2"),
        _msg("assistant", "改完了", "a2"),
    ])
    segments = summaries.segment_session(session)
    assert len(segments) == 2
    assert segments[0]["turn"] == 1
    assert segments[0]["anchor_locator"] == "u1"
    assert segments[0]["digest"] is None
    assert segments[0]["hash"].startswith("sha256:")
    # 工具块不计入 hash 文本
    assert segments[0]["char_count"] == len("帮我改支付\n好的，已定位")
    assert segments[1]["turn"] == 2


def test_after_compaction_marks_following_turn():
    compaction = SimpleNamespace(summary_message_id="c1", after_message_id="u1")
    session = _session([
        _msg("user", "第一轮", "u1"),
        _msg("user", "压缩后这轮", "u2"),
    ], compactions=[compaction])
    segments = summaries.segment_session(session)
    assert segments[0]["after_compaction"] is False
    assert segments[1]["after_compaction"] is True


def test_internal_compaction_summary_is_excluded():
    compaction = SimpleNamespace(summary_message_id="c1", after_message_id="u1")
    session = _session([
        _msg("user", "问题", "u1"),
        _msg("assistant", "这是压缩摘要，不该进段", "c1"),
        _msg("user", "下一问", "u2"),
    ], compactions=[compaction])
    segments = summaries.segment_session(session)
    assert len(segments) == 2
    assert "压缩摘要" not in segments[0]["hash"]
    assert segments[0]["char_count"] == len("问题")


def test_fingerprint_tracks_content():
    a = summaries.segment_session(_session([_msg("user", "a", "u1")]))
    b = summaries.segment_session(_session([_msg("user", "b", "u1")]))
    assert summaries.session_fingerprint(a) != summaries.session_fingerprint(b)


def test_build_backbone_caches_and_preserves_digests(tmp_path, monkeypatch, ports):
    database = _use_database(tmp_path, monkeypatch)
    session = _session([
        _msg("user", "改支付", "u1"),
        _msg("user", "改标题", "u2"),
    ])
    monkeypatch.setattr(summaries, "read_tree", lambda tool, ref, ports: session)

    first = summaries.build_backbone("claude", "fsr_x", ports)
    assert first["segment_count"] == 2
    assert len(first["pending"]) == 2
    assert first["pending_sources"] == [
        {"hash": first["segments"][0]["hash"], "text": "改支付"},
        {"hash": first["segments"][1]["hash"], "text": "改标题"},
    ]
    assert "_source_text" not in database.summaries.get(
        "claude", "sess-1",
    )["segments"][0]

    head = first["segments"][0]["hash"]
    written = summaries.set_summaries("claude", "sess-1", {head: "改了支付逻辑"}, ports)
    assert written["applied"] == 1

    # 续写一轮 → 指纹变;未变段的摘要按内容 hash 迁移保留
    session.messages.append(_msg("user", "第三轮", "u3"))
    rebuilt = summaries.build_backbone("claude", "fsr_x", ports)
    assert rebuilt["segment_count"] == 3
    kept = next(s for s in rebuilt["segments"] if s["hash"] == head)
    assert kept["digest"] == "改了支付逻辑"
    assert len(rebuilt["pending"]) == 2


def test_build_backbone_returns_cache_when_unchanged(tmp_path, monkeypatch, ports):
    _use_database(tmp_path, monkeypatch)
    session = _session([_msg("user", "只此一轮", "u1")])
    monkeypatch.setattr(summaries, "read_tree", lambda tool, ref, ports: session)
    first = summaries.build_backbone("claude", "fsr_x", ports)
    again = summaries.build_backbone("claude", "fsr_x", ports)
    assert first["fingerprint"] == again["fingerprint"]


def test_build_backbone_refreshes_structure_for_textless_message(
        tmp_path, monkeypatch, ports):
    _use_database(tmp_path, monkeypatch)
    session = _session([
        _msg("user", "改支付", "u1"),
        _msg("assistant", "完成", "a1"),
        _msg("user", "改标题", "u2"),
    ])
    monkeypatch.setattr(summaries, "read_tree", lambda tool, ref, ports: session)

    first = summaries.build_backbone("claude", "fsr_x", ports)
    first_hash = first["segments"][0]["hash"]
    summaries.set_summaries(
        "claude", "sess-1", {first_hash: "已修改支付逻辑"}, ports)

    session.messages.insert(2, _msg("assistant", "", "tool-1", with_tool=True))
    rebuilt = summaries.build_backbone("claude", "fsr_x", ports)

    assert rebuilt["fingerprint"] == first["fingerprint"]
    assert rebuilt["segments"][0]["message_end"] == 2
    assert rebuilt["segments"][1]["message_start"] == 3
    assert rebuilt["segments"][0]["digest"] == "已修改支付逻辑"


def test_build_backbone_refreshes_compaction_boundary(tmp_path, monkeypatch, ports):
    _use_database(tmp_path, monkeypatch)
    session = _session([
        _msg("user", "第一轮", "u1"),
        _msg("user", "第二轮", "u2"),
    ])
    monkeypatch.setattr(summaries, "read_tree", lambda tool, ref, ports: session)

    first = summaries.build_backbone("claude", "fsr_x", ports)
    second_hash = first["segments"][1]["hash"]
    summaries.set_summaries(
        "claude", "sess-1", {second_hash: "第二轮摘要"}, ports)

    session.context_compactions.append(SimpleNamespace(
        summary_message_id=None, after_message_id="u1"))
    rebuilt = summaries.build_backbone("claude", "fsr_x", ports)

    assert rebuilt["fingerprint"] == first["fingerprint"]
    assert rebuilt["segments"][1]["after_compaction"] is True
    assert rebuilt["segments"][1]["digest"] == "第二轮摘要"


def test_set_summaries_requires_backbone(tmp_path, monkeypatch, ports):
    _use_database(tmp_path, monkeypatch)
    with pytest.raises(SummaryBackboneMissingError):
        summaries.set_summaries("claude", "missing", {"sha256:x": "y"}, ports)


def test_summary_cache_is_scoped_by_tool_and_native_session_id(
        tmp_path, monkeypatch, ports):
    database = _use_database(tmp_path, monkeypatch)
    database.summaries.store({
        "tool": "claude", "id": "shared", "fingerprint": "claude-fp",
        "segments": [],
    }, 1)
    database.summaries.store({
        "tool": "codex", "id": "shared", "fingerprint": "codex-fp",
        "segments": [],
    }, 2)

    assert summaries.get_backbone("claude", "shared", ports)["fingerprint"] == "claude-fp"
    assert summaries.get_backbone("codex", "shared", ports)["fingerprint"] == "codex-fp"


def test_rpc_wiring_returns_structured_error(tmp_path, monkeypatch):
    import json

    from engine.server.rpc import PROTOCOL, rpc

    _use_database(tmp_path, monkeypatch)
    response = rpc(json.dumps({
        "protocol": PROTOCOL,
        "id": "s-1",
        "method": "session_summaries_set",
        "params": {"tool": "claude", "id": "missing", "digests": {"sha256:x": "y"}},
    }))
    assert response["ok"] is False
    assert response["error"]["code"] == "summary.backbone_missing"
    assert response["error"]["category"] == "not-found"
