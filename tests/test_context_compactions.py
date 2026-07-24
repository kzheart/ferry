import json

from engine.adapters.claude import reader as claude_reader
from engine.adapters.codex import reader as codex_reader
from engine.adapters.opencode import session as opencode_session
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

    session, _ = opencode_session._parse_session(payload)
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
