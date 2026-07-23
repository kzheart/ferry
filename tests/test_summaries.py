"""摘要底座:分段 / 内容指纹 / 缓存失效 / 摘要写回。"""

from types import SimpleNamespace

import pytest

from engine.application import summaries
from engine.domain.errors import SummaryBackboneMissingError


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


def test_build_backbone_caches_and_preserves_digests(tmp_path, monkeypatch):
    monkeypatch.setattr(summaries, "SUMMARIES", tmp_path / "summaries.json")
    session = _session([
        _msg("user", "改支付", "u1"),
        _msg("user", "改标题", "u2"),
    ])
    monkeypatch.setattr(summaries, "read_tree", lambda tool, ref: session)

    first = summaries.build_backbone("claude", "fsr_x")
    assert first["segment_count"] == 2
    assert len(first["pending"]) == 2

    head = first["segments"][0]["hash"]
    written = summaries.set_summaries("claude", "sess-1", {head: "改了支付逻辑"})
    assert written["applied"] == 1

    # 续写一轮 → 指纹变;未变段的摘要按内容 hash 迁移保留
    session.messages.append(_msg("user", "第三轮", "u3"))
    rebuilt = summaries.build_backbone("claude", "fsr_x")
    assert rebuilt["segment_count"] == 3
    kept = next(s for s in rebuilt["segments"] if s["hash"] == head)
    assert kept["digest"] == "改了支付逻辑"
    assert len(rebuilt["pending"]) == 2


def test_build_backbone_returns_cache_when_unchanged(tmp_path, monkeypatch):
    monkeypatch.setattr(summaries, "SUMMARIES", tmp_path / "summaries.json")
    session = _session([_msg("user", "只此一轮", "u1")])
    monkeypatch.setattr(summaries, "read_tree", lambda tool, ref: session)
    first = summaries.build_backbone("claude", "fsr_x")
    again = summaries.build_backbone("claude", "fsr_x")
    assert first["fingerprint"] == again["fingerprint"]


def test_set_summaries_requires_backbone(tmp_path, monkeypatch):
    monkeypatch.setattr(summaries, "SUMMARIES", tmp_path / "summaries.json")
    with pytest.raises(SummaryBackboneMissingError):
        summaries.set_summaries("claude", "missing", {"sha256:x": "y"})


def test_rpc_wiring_returns_structured_error(tmp_path, monkeypatch):
    import json

    from engine.interfaces.rpc import rpc

    monkeypatch.setattr(summaries, "SUMMARIES", tmp_path / "summaries.json")
    response = rpc(json.dumps({
        "method": "session_summaries_set", "request_id": "s-1",
        "params": {"tool": "claude", "id": "missing", "digests": {"sha256:x": "y"}},
    }))
    assert response["ok"] is False
    assert response["error"]["code"] == "summary.backbone_missing"
    assert response["error"]["category"] == "not-found"
