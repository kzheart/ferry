import json

from engine.adapters.claude import reader as claude_reader
from engine.adapters.codex import reader as codex_reader
from engine.adapters.opencode import reader as opencode_reader
from engine.adapters.pi import reader as pi_reader
from engine.sessions.model import AgentEdge, Block, Message, Session
from engine.sessions.read import session_json


def _opencode_message(mid, role, parts, parent=None, **info):
    message_info = {
        "id": mid,
        "sessionID": "session",
        "role": role,
        "time": {"created": len(mid)},
        **info,
    }
    if parent:
        message_info["parentID"] = parent
    return {"info": message_info, "parts": [
        {"id": f"{mid}-part-{index}", "messageID": mid,
         "sessionID": "session", **part}
        for index, part in enumerate(parts)
    ]}


def test_opencode_exposes_compaction_without_repeating_summary():
    payload = {
        "info": {"id": "session", "directory": "/tmp", "time": {}},
        "messages": [
            _opencode_message("u1", "user", [{"type": "text", "text": "before"}]),
            _opencode_message("a1", "assistant", [{"type": "text", "text": "answer"}],
                              parent="u1", finish="stop"),
            _opencode_message("compact", "user", [{
                "type": "compaction", "auto": True,
                "tail_start_id": "a1",
            }]),
            _opencode_message("summary", "assistant", [{
                "type": "text", "text": "## Summary\nkept context",
            }], parent="compact", mode="compaction", summary=True, finish="stop"),
            _opencode_message("u2", "user", [{"type": "text", "text": "after"}]),
        ],
    }

    session, _ = opencode_reader.parse_session(payload)
    dto = session_json(session)

    assert len(session.messages) == 4
    assert dto["count"] == 3
    assert dto["context"] == {
        "state": "compacted",
        "compaction_count": 1,
        "summary_status": "available",
    }
    compaction = dto["context_compactions"][0]
    assert compaction["after_turn"] == 1
    assert compaction["trigger"] == "automatic"
    assert compaction["summary"]["text"] == "## Summary\nkept context"
    assert compaction["tail"]["start_message_index"] == 2
    assert all(message["locator"] != "summary" for message in dto["messages"])


def test_claude_pairs_boundary_and_internal_summary_by_uuid(tmp_path):
    path = tmp_path / "claude.jsonl"
    records = [
        {"type": "user", "uuid": "u1", "sessionId": "session",
         "timestamp": "2026-01-01T00:00:00Z",
         "message": {"role": "user", "content": "before"}},
        {"type": "assistant", "uuid": "a1", "parentUuid": "u1",
         "timestamp": "2026-01-01T00:00:01Z",
         "message": {"role": "assistant", "content": "answer"}},
        {"type": "system", "subtype": "compact_boundary", "uuid": "b1",
         "logicalParentUuid": "a1", "timestamp": "2026-01-01T00:00:03Z",
         "compactMetadata": {
             "trigger": "manual", "preTokens": 1000, "postTokens": 120,
             "cumulativeDroppedTokens": 880, "durationMs": 25,
             "preservedSegment": {"anchorUuid": "s1"},
         }},
        {"type": "user", "uuid": "s1", "parentUuid": "b1",
         "timestamp": "2026-01-01T00:00:02Z",
         "isCompactSummary": True, "isVisibleInTranscriptOnly": True,
         "message": {"role": "user", "content": "Claude summary"}},
        {"type": "user", "uuid": "u2", "parentUuid": "s1",
         "timestamp": "2026-01-01T00:00:04Z",
         "message": {"role": "user", "content": "after"}},
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))

    dto = session_json(claude_reader.read(str(path)))

    assert dto["count"] == 3
    compaction = dto["context_compactions"][0]
    assert compaction["after_message_locator"] == "a1"
    assert compaction["after_turn"] == 1
    assert compaction["trigger"] == "manual"
    assert compaction["summary"] == {
        "status": "available",
        "text": "Claude summary",
        "locator": "s1",
    }
    assert compaction["metrics"]["dropped_tokens"] == 880
    assert all(message["locator"] != "s1" for message in dto["messages"])


def test_claude_resolves_anchor_through_system_records(tmp_path):
    # 压缩边界的 logicalParentUuid 实际常指向 system 记录（turn_duration 等），
    # 需沿 parentUuid 回溯到真实消息，否则 after_turn 兜底为 0、卡片错排到会话开头。
    path = tmp_path / "claude.jsonl"
    records = [
        {"type": "user", "uuid": "u1", "sessionId": "session",
         "timestamp": "2026-01-01T00:00:00Z",
         "message": {"role": "user", "content": "before"}},
        {"type": "assistant", "uuid": "a1", "parentUuid": "u1",
         "timestamp": "2026-01-01T00:00:01Z",
         "message": {"role": "assistant", "content": "answer"}},
        {"type": "system", "subtype": "stop_hook_summary", "uuid": "sys1",
         "parentUuid": "a1", "timestamp": "2026-01-01T00:00:02Z"},
        {"type": "system", "subtype": "turn_duration", "uuid": "sys2",
         "parentUuid": "sys1", "timestamp": "2026-01-01T00:00:03Z"},
        {"type": "system", "subtype": "compact_boundary", "uuid": "b1",
         "logicalParentUuid": "sys2", "timestamp": "2026-01-01T00:00:04Z",
         "compactMetadata": {"trigger": "auto",
                             "preTokens": 1000, "postTokens": 100}},
        {"type": "user", "uuid": "s1", "parentUuid": "b1",
         "timestamp": "2026-01-01T00:00:05Z", "isCompactSummary": True,
         "message": {"role": "user", "content": "summary"}},
        {"type": "user", "uuid": "u2", "parentUuid": "s1",
         "timestamp": "2026-01-01T00:00:06Z",
         "message": {"role": "user", "content": "after"}},
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))

    dto = session_json(claude_reader.read(str(path)))

    compaction = dto["context_compactions"][0]
    assert compaction["after_message_locator"] == "a1"
    assert compaction["after_turn"] == 1


def test_codex_exposes_encrypted_compaction_once(tmp_path):
    path = tmp_path / "rollout.jsonl"
    records = [
        {"timestamp": "2026-01-01T00:00:00Z", "type": "session_meta",
         "payload": {"id": "session", "cwd": "/tmp"}},
        {"timestamp": "2026-01-01T00:00:01Z", "type": "response_item",
         "payload": {"type": "message", "role": "user",
                     "content": [{"type": "input_text", "text": "before"}]}},
        {"timestamp": "2026-01-01T00:00:02Z", "type": "compacted",
         "payload": {
             "message": "",
             "replacement_history": [
                 {"type": "compaction", "id": "cmp1",
                  "encrypted_content": "ciphertext"},
             ],
             "window_number": 2,
             "window_id": "window-2",
         }},
        {"timestamp": "2026-01-01T00:00:02Z", "type": "event_msg",
         "payload": {"type": "context_compacted"}},
        {"timestamp": "2026-01-01T00:00:03Z", "type": "response_item",
         "payload": {"type": "message", "role": "user",
                     "content": [{"type": "input_text", "text": "after"}]}},
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))

    dto = session_json(codex_reader._read_one(path))

    assert dto["count"] == 2
    assert dto["context"]["compaction_count"] == 1
    compaction = dto["context_compactions"][0]
    assert compaction["after_turn"] == 1
    assert compaction["summary"]["status"] == "protected"
    assert compaction["summary"]["text"] == ""


def test_pi_exposes_active_branch_compaction(tmp_path):
    path = tmp_path / "pi.jsonl"
    records = [
        {"type": "session", "version": 3, "id": "session",
         "timestamp": "2026-01-01T00:00:00Z", "cwd": "/tmp"},
        {"type": "message", "id": "u1", "parentId": None,
         "timestamp": "2026-01-01T00:00:01Z",
         "message": {"role": "user", "content": "before", "timestamp": 1}},
        {"type": "compaction", "id": "c1", "parentId": "u1",
         "timestamp": "2026-01-01T00:00:02Z", "summary": "Pi summary",
         "firstKeptEntryId": "u1", "tokensBefore": 123},
        {"type": "message", "id": "u2", "parentId": "c1",
         "timestamp": "2026-01-01T00:00:03Z",
         "message": {"role": "user", "content": "after", "timestamp": 2}},
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))

    dto = session_json(pi_reader.read(str(path)))
    assert dto["context"]["compaction_count"] == 1
    compaction = dto["context_compactions"][0]
    assert compaction["summary"]["text"] == "Pi summary"
    assert compaction["metrics"]["tokens_before"] == 123
    assert compaction["tail"]["start_locator"] == "u1"


def test_session_json_pages_root_messages_and_serializes_agent_edges():
    root = Session("codex", "root", "/tmp")
    root.messages = [
        Message("user", [Block("text", "first")], source_id="u1"),
        Message("assistant", [Block("text", "one")], source_id="a1"),
        Message("user", [Block("text", "second")], source_id="u2"),
        Message("assistant", [Block("text", "two")], source_id="a2"),
    ]
    root.children = [Session("codex", "child", "/tmp")]
    root.agent_edges = [AgentEdge("root", "child", status="closed")]

    first = session_json(root, message_limit=2, include_tree=False)
    second = session_json(
        root, from_message=first["next_from_message"], message_limit=2,
        include_tree=False,
    )

    assert first["next_from_message"] == 3
    assert first["turns"][0]["turn"] == 1
    assert second["next_from_message"] is None
    assert second["turns"][0]["turn"] == 2
    assert first["agent_edges"] == [{
        "parent_session_id": "root",
        "child_session_id": "child",
        "source_call_id": None,
        "spawn_message_id": None,
        "result_message_id": None,
        "agent_id": None,
        "agent_path": None,
        "agent_type": None,
        "prompt": "",
        "status": "closed",
        "association": "explicit",
        "confidence": 1.0,
    }]
