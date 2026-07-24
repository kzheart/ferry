import copy
import json
from pathlib import Path

import pytest

from engine.adapters.shared.editing import EditDocument
from engine.adapters.claude.editor import ClaudeBackend
from engine.adapters.claude.editing import check_invariants
from engine.adapters.claude.reader import read as read_claude
from engine.adapters.codex.editor import CodexBackend
from engine.adapters.codex.reader import read as read_codex
from engine.adapters.opencode.editor import OpenCodeBackend
from engine.adapters.opencode.probe import OpenCodeVerifier
from engine.adapters.opencode.reader import parse_session
from engine.operations.edit import apply_mutation
from engine.sessions.read import session_json
from engine.operations.types import AssistantReply
from engine.errors import ConcurrentModificationError
from engine.sessions.model import tool_result_text
from engine.sessions.tool_ops import CanonicalOp


ROOT = Path(__file__).parents[1]
FORMAT_FIXTURES = ROOT / "tests" / "fixtures" / "agent_formats"


def _jsonl(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def _opencode_payload(case):
    fixture = json.loads((FORMAT_FIXTURES / "opencode" /
                          case / "session.json").read_text())
    messages = []
    parts = {}
    for row in fixture["parts"]:
        parts.setdefault(row["message_id"], []).append(json.loads(row["data"]))
    for row in fixture["messages"]:
        messages.append({"info": json.loads(row["data"]),
                         "parts": parts.get(row["id"], [])})
    return {"info": fixture["session"], "messages": messages}


def _native(tool, case="case-01-plain"):
    if tool == "claude":
        path = FORMAT_FIXTURES / "claude" / case / "session.jsonl"
        return _jsonl(path)
    if tool == "codex":
        path = FORMAT_FIXTURES / "codex" / case / "session.jsonl"
        return _jsonl(path)
    return _opencode_payload(case)


def _editor(tool):
    return {"claude": ClaudeBackend, "codex": CodexBackend}[tool]()


def _document(tool, data):
    return EditDocument(tool, "fixture", Path("fixture"), copy.deepcopy(data), "rev")


def _validate(tool, data):
    if tool == "claude":
        check_invariants(data)
    elif tool == "codex":
        CodexBackend().validate(_document(tool, data))
    else:
        OpenCodeBackend().validate(_document(tool, data))


def _roundtrip(tool, data, tmp_path):
    if tool == "claude":
        path = tmp_path / "claude.jsonl"
        path.write_text("\n".join(json.dumps(row) for row in data) + "\n")
        return read_claude(str(path))
    if tool == "codex":
        path = tmp_path / "codex.jsonl"
        path.write_text("\n".join(json.dumps(row) for row in data) + "\n")
        return read_codex(str(path), sessions_dir=tmp_path)
    return parse_session(data)[0]


def _items(session):
    result = []
    for message in session.messages:
        if message.role != "assistant":
            continue
        for block in message.blocks:
            if block.kind == "text":
                result.append({"kind": "text", "text": block.text})
            elif block.kind == "tool":
                name = block.tool.name
                tool_input = block.tool.input
                if (block.tool.op == CanonicalOp.TOOL_INVOKE and
                        isinstance(tool_input, dict)):
                    name = tool_input.get("name") or name
                    tool_input = tool_input.get("input", tool_input)
                result.append({"kind": "tool", "name": name,
                               "input": tool_input,
                               "output": tool_result_text(block.tool.result)})
    return result


@pytest.mark.parametrize("tool", ["claude", "codex"])
def test_pure_text_replacement_roundtrips(tool, tmp_path):
    doc = _document(tool, _native(tool))
    reply = AssistantReply.from_dict({"items": [{"kind": "text", "text": "new reply"}]})

    _editor(tool).replace_reply(doc, 1, reply)

    _validate(tool, doc.data)
    assert _items(_roundtrip(tool, doc.data, tmp_path)) == reply.to_dict()["items"]


@pytest.mark.parametrize("tool", ["claude", "codex"])
def test_tool_insertion_generates_native_pairs_and_foreign_keys(tool, tmp_path):
    doc = _document(tool, _native(tool))
    reply = AssistantReply.from_dict({"items": [{
        "kind": "tool", "name": "lookup", "input": {"query": "x"}, "output": "found",
    }]})

    _editor(tool).replace_reply(doc, 1, reply)
    _validate(tool, doc.data)

    if tool == "claude":
        uses = [block["id"] for row in doc.data for block in
                ((row.get("message") or {}).get("content") or [])
                if isinstance(block, dict) and block.get("type") == "tool_use"]
        results = [block["tool_use_id"] for row in doc.data for block in
                   ((row.get("message") or {}).get("content") or [])
                   if isinstance(block, dict) and block.get("type") == "tool_result"]
        assert uses == results and uses[0].startswith("toolu_")
    elif tool == "codex":
        calls = [row["payload"]["call_id"] for row in doc.data
                 if (row.get("payload") or {}).get("type") == "function_call"]
        outputs = [row["payload"]["call_id"] for row in doc.data
                   if (row.get("payload") or {}).get("type") == "function_call_output"]
        assert calls == outputs and calls[0].startswith("call_")
    assert _items(_roundtrip(tool, doc.data, tmp_path)) == reply.to_dict()["items"]


@pytest.mark.parametrize("tool", ["claude", "codex"])
def test_text_tool_text_order_roundtrips(tool, tmp_path):
    value = {"items": [
        {"kind": "text", "text": "before"},
        {"kind": "tool", "name": "lookup", "input": {"key": 1}, "output": "ok"},
        {"kind": "text", "text": "after"},
    ]}
    reply = AssistantReply.from_dict(value)
    doc = _document(tool, _native(tool))

    _editor(tool).replace_reply(doc, 1, reply)

    _validate(tool, doc.data)
    assert _items(_roundtrip(tool, doc.data, tmp_path)) == value["items"]


@pytest.mark.parametrize("tool", ["claude", "codex"])
def test_string_tool_input_roundtrips(tool, tmp_path):
    value = {"items": [{"kind": "tool", "name": "opaque",
                        "input": "raw input", "output": "raw output"}]}
    doc = _document(tool, _native(tool))

    _editor(tool).replace_reply(doc, 1, AssistantReply.from_dict(value))

    assert _items(_roundtrip(tool, doc.data, tmp_path)) == value["items"]


@pytest.mark.parametrize("tool", ["claude", "codex"])
def test_replacement_deletes_existing_tools(tool, tmp_path):
    doc = _document(tool, _native(tool, "case-02-tools"))
    reply = AssistantReply.from_dict({"items": [{"kind": "text", "text": "no tools"}]})

    _editor(tool).replace_reply(doc, 1, reply)

    _validate(tool, doc.data)
    session = _roundtrip(tool, doc.data, tmp_path)
    assert _items(session) == [{"kind": "text", "text": "no tools"}]


@pytest.mark.parametrize("value", [
    {}, {"items": []}, {"items": [{"kind": "text", "text": "", "id": "user-id"}]},
    {"items": [{"kind": "tool", "name": "x", "input": [], "output": ""}]},
    {"items": [{"kind": "tool", "name": "x", "input": {}, "output": "", "call_id": "x"}]},
    {"items": [{"kind": "thinking", "text": "hidden"}]},
])
def test_invalid_or_structural_input_is_rejected(value):
    with pytest.raises(ValueError):
        AssistantReply.from_dict(value)


def test_turn_bounds_and_opencode_operations_are_explicit():
    reply = AssistantReply.from_dict({"items": [{"kind": "text", "text": "x"}]})
    editor = ClaudeBackend()
    with pytest.raises(ValueError, match="轮次超界"):
        editor.replace_reply(_document("claude", _native("claude")), 2, reply)
    assert "replace-assistant-reply" not in OpenCodeBackend().operations


def test_show_dto_exposes_ordered_reply_draft(tmp_path):
    session = _roundtrip("claude", _native("claude", "case-02-tools"), tmp_path)
    dto = session_json(session)

    draft = dto["turns"][0]["assistant_reply"]
    assert [item["kind"] for item in draft["items"]] == ["tool", "tool", "tool", "text"]
    assert set(draft["items"][0]) == {"kind", "name", "input", "output"}
    assert dto["turns"][0]["turn_locator"] == "fixture-message-user-tools"


class _TransactionEditor:
    name = "fake"

    def __init__(self, error=None):
        self.error = error
        self.restored = False

    def load(self, ref):
        return EditDocument("fake", ref, Path(ref), [], "before")

    def validate(self, doc):
        pass

    def stats(self, doc):
        return {"count": len(doc.data), "size": 0}

    def snapshot(self, doc, reason_code="snapshot.before_edit", extra=None):
        self.extra = extra
        return Path("snapshot")

    def commit(self, doc):
        if self.error:
            raise self.error("commit failed")
        return {"saved_as": "result"}

    def restore_snapshot(self, snapshot, doc):
        self.restored = True

    def saved_revision(self, result, doc):
        return "sha256:authored"


def test_cas_conflict_never_restores_stale_snapshot():
    editor = _TransactionEditor(ConcurrentModificationError)

    with pytest.raises(ConcurrentModificationError):
        apply_mutation(editor, "source", lambda doc: [])

    assert editor.restored is False


def test_non_cas_commit_failure_still_restores_snapshot():
    editor = _TransactionEditor(RuntimeError)

    with pytest.raises(RuntimeError):
        apply_mutation(editor, "source", lambda doc: [])

    assert editor.restored is True


def test_success_revision_identifies_authored_result():
    editor = _TransactionEditor()

    result, _, _ = apply_mutation(editor, "source", lambda doc: [])

    assert result["revision"] == "sha256:authored"
    assert result["revision"] != "before"


def test_opencode_probe_clones_authored_result(monkeypatch):
    loaded = []
    deleted = []
    tree = type("Tree", (), {"source_id": "authored-copy"})()
    shadow_tree = type(
        "ShadowTree",
        (),
        {"walk": lambda self: [
            type("Node", (), {"source_id": "probe-shadow"})(),
        ]},
    )()

    class Editor:
        def load(self, ref):
            loaded.append(ref)
            return type(
                "Doc",
                (),
                {"ref": ref, "tree": tree,
                 "data": {"info": {"id": ref}}},
            )()

    monkeypatch.setattr(
        "engine.adapters.opencode.probe._probe",
        lambda sid, cwd, model: {"status": "passed", "code": None,
                                 "params": {}, "diagnostic": {}})
    monkeypatch.setattr(
        "engine.adapters.opencode.probe.opencode_writer.write",
        lambda authored_tree, **kwargs: ("probe-shadow", "unused"),
    )
    monkeypatch.setattr(
        "engine.adapters.opencode.probe.opencode_reader.read",
        lambda _sid: shadow_tree,
    )
    monkeypatch.setattr(
        "engine.adapters.opencode.probe.opencode_store.delete_session",
        lambda session_id: deleted.append(["session", "delete", session_id]),
    )
    doc = type("Doc", (), {"ref": "original",
                            "data": {"info": {"directory": "/work"}}})()

    report = OpenCodeVerifier().probe_edited(
        Editor(), doc, {"session_id": "authored-copy"})

    assert report["status"] == "passed"
    assert report["isolation"] == {"kind": "shadow_session",
                                   "id": "probe-shadow", "cleaned": True}
    assert loaded == ["authored-copy"]
    assert deleted == [["session", "delete", "probe-shadow"]]


def test_claude_retained_records_keep_order_and_valid_parents():
    records = [
        {"type": "user", "uuid": "user-1", "parentUuid": None,
         "message": {"role": "user", "content": "question"}},
        {"type": "progress", "uuid": "progress", "parentUuid": "user-1"},
        {"type": "assistant", "uuid": "old-reply", "parentUuid": "user-1",
         "message": {"role": "assistant", "content": [{"type": "text", "text": "old"}]}},
        {"type": "file-history-snapshot", "uuid": "retained", "parentUuid": "old-reply"},
        {"type": "user", "uuid": "user-2", "parentUuid": "old-reply",
         "message": {"role": "user", "content": "next"}},
    ]
    doc = _document("claude", records)
    reply = AssistantReply.from_dict({"items": [{"kind": "text", "text": "new"}]})

    ClaudeBackend().replace_reply(doc, "user-1", reply)

    types = [record["type"] for record in doc.data]
    assert types.index("progress") < types.index("assistant") < types.index("file-history-snapshot")
    assert "old-reply" not in {record.get("uuid") for record in doc.data}
    assert doc.data[-2]["parentUuid"] == doc.data[-1]["parentUuid"]
    check_invariants(doc.data)


def test_claude_text_before_tool_is_one_assistant_message():
    doc = _document("claude", _native("claude"))
    reply = AssistantReply.from_dict({"items": [
        {"kind": "text", "text": "I will check"},
        {"kind": "tool", "name": "Lookup", "input": {}, "output": "ok"},
        {"kind": "text", "text": "finished"},
    ]})

    ClaudeBackend().replace_reply(doc, 1, reply)

    authored = doc.data[1:]
    assert [item["type"] for item in authored[0]["message"]["content"]] == [
        "text", "tool_use"]
    assert authored[1]["message"]["content"][0]["type"] == "tool_result"
    assert authored[2]["message"]["content"] == [{"type": "text", "text": "finished"}]


def test_codex_preserves_reasoning_unknown_items_and_event_positions():
    records = _native("codex")
    old_reply = records.pop()
    preserved = [
        {"type": "response_item", "payload": {"type": "reasoning", "summary": []}},
        {"type": "event_msg", "payload": {"type": "marker", "value": "before"}},
        {"type": "response_item", "payload": {"type": "future_item", "value": 1}},
    ]
    trailing = [
        {"type": "event_msg", "payload": {"type": "marker", "value": "after"}},
        {"type": "turn_context", "payload": {"turn_id": "turn-2"}},
        {"type": "response_item", "payload": {"type": "message", "role": "user",
                                                "content": [{"type": "input_text", "text": "next"}]}},
        {"type": "response_item", "payload": {"type": "message", "role": "assistant",
                                                "content": [{"type": "output_text", "text": "next-old"}]}},
    ]
    records.extend(preserved + [old_reply] + trailing)
    doc = _document("codex", records)

    CodexBackend().replace_reply(
        doc, "record:2", AssistantReply.from_dict(
            {"items": [{"kind": "text", "text": "authored"}]}))

    assert doc.data[3:6] == preserved
    authored_index = next(index for index, record in enumerate(doc.data)
                          if ((record.get("payload") or {}).get("content") or [{}])[0].get("text") == "authored")
    after_index = doc.data.index(trailing[0])
    context_index = doc.data.index(trailing[1])
    next_user_index = doc.data.index(trailing[2])
    assert authored_index < after_index < context_index < next_user_index


def test_codex_reader_flushes_incomplete_tool_before_next_user(tmp_path):
    records = _native("codex")[:-1]
    records.extend([
        {"type": "response_item", "payload": {"type": "custom_tool_call",
                                                "name": "lookup", "input": "{}",
                                                "call_id": "pending"}},
        {"type": "response_item", "payload": {"type": "message", "role": "user",
                                                "content": [{"type": "input_text", "text": "second"}]}},
        {"type": "response_item", "payload": {"type": "message", "role": "assistant",
                                                "content": [{"type": "output_text", "text": "answer"}]}},
    ])

    dto = session_json(_roundtrip("codex", records, tmp_path))

    assert dto["turns"][0]["assistant_reply"]["items"][0]["name"] == "lookup"
    assert dto["turns"][1]["assistant_reply"]["items"] == [
        {"kind": "text", "text": "answer"}]


@pytest.mark.parametrize("tool", ["claude", "codex"])
def test_dto_turn_locator_selects_same_editor_turn(tool, tmp_path):
    data = _native(tool)
    locator = session_json(_roundtrip(tool, data, tmp_path))["turns"][0]["turn_locator"]
    doc = _document(tool, data)

    _editor(tool).replace_reply(doc, locator, AssistantReply.from_dict(
        {"items": [{"kind": "text", "text": "by locator"}]}))

    assert _items(_roundtrip(tool, doc.data, tmp_path)) == [
        {"kind": "text", "text": "by locator"}]


@pytest.mark.parametrize("tool", ["claude", "codex"])
def test_replacement_agent_spawn_is_rejected(tool):
    doc = _document(tool, _native(tool))
    reply = AssistantReply.from_dict({"items": [{"kind": "tool", "name": "spawn_agent",
                                                  "input": {}, "output": "done"}]})

    with pytest.raises(ValueError, match="会话树"):
        _editor(tool).replace_reply(doc, 1, reply)


@pytest.mark.parametrize("tool", ["claude", "codex"])
def test_existing_agent_spawn_is_rejected(tool):
    data = _native(tool, "case-02-tools")
    if tool == "claude":
        data[1]["message"]["content"][0]["name"] = "Agent"
    elif tool == "codex":
        data[3]["payload"]["name"] = "spawn_agent"
    else:
        data["messages"][1]["parts"][0]["tool"] = "task"

    with pytest.raises(ValueError, match="目标回复包含"):
        _editor(tool).replace_reply(
            _document(tool, data), 1,
            AssistantReply.from_dict({"items": [{"kind": "text", "text": "safe"}]}))


@pytest.mark.parametrize("tool", ["claude", "codex"])
def test_hidden_user_carriers_do_not_change_dto_or_editor_turns(tool, tmp_path):
    if tool == "claude":
        data = [
            {"type": "user", "uuid": "u1", "parentUuid": None,
             "message": {"role": "user", "content": "one"}},
            {"type": "assistant", "uuid": "a1", "parentUuid": "u1",
             "message": {"role": "assistant", "content": [{"type": "text", "text": "old1"}]}},
            {"type": "user", "uuid": "hidden", "parentUuid": "a1",
             "message": {"role": "user", "content": [{"type": "internal", "data": 1}]}},
            {"type": "user", "uuid": "u2", "parentUuid": "a1",
             "message": {"role": "user", "content": "two"}},
            {"type": "assistant", "uuid": "a2", "parentUuid": "u2",
             "message": {"role": "assistant", "content": [{"type": "text", "text": "old2"}]}},
        ]
    elif tool == "codex":
        data = [
            {"type": "session_meta", "payload": {"id": "sid", "cwd": "/tmp"}},
            {"type": "response_item", "payload": {"type": "message", "role": "user",
                                                    "content": [{"type": "input_text", "text": "one"}]}},
            {"type": "response_item", "payload": {"type": "message", "role": "assistant",
                                                    "content": [{"type": "output_text", "text": "old1"}]}},
            {"type": "response_item", "payload": {"type": "message", "role": "user",
                                                    "content": [{"type": "input_text",
                                                                 "text": "<environment_context>x"}]}},
            {"type": "response_item", "payload": {"type": "message", "role": "user",
                                                    "content": [{"type": "input_text", "text": "two"}]}},
            {"type": "response_item", "payload": {"type": "message", "role": "assistant",
                                                    "content": [{"type": "output_text", "text": "old2"}]}},
        ]
    else:
        sid = "sid"
        def message(mid, role, parts):
            return {"info": {"id": mid, "sessionID": sid, "role": role,
                              **({"finish": "stop"} if role == "assistant" else {})},
                    "parts": [{"id": f"p-{mid}-{index}", "messageID": mid,
                               "sessionID": sid, **part}
                              for index, part in enumerate(parts)]}
        data = {"info": {"id": sid, "directory": "/tmp"}, "messages": [
            message("u1", "user", [{"type": "text", "text": "one"}]),
            message("a1", "assistant", [{"type": "text", "text": "old1"}]),
            message("hidden", "user", [{"type": "step-start"}]),
            message("u2", "user", [{"type": "text", "text": "two"}]),
            message("a2", "assistant", [{"type": "text", "text": "old2"}]),
        ]}
    dto = session_json(_roundtrip(tool, data, tmp_path))
    assert len(dto["turns"]) == 2
    doc = _document(tool, data)

    _editor(tool).replace_reply(
        doc, dto["turns"][1]["turn_locator"],
        AssistantReply.from_dict({"items": [{"kind": "text", "text": "new2"}]}))

    assert [item["text"] for item in _items(_roundtrip(tool, doc.data, tmp_path))
            if item["kind"] == "text"] == ["old1", "new2"]


def test_claude_ismeta_image_companion_not_a_turn(tmp_path):
    """粘贴图片时 Claude 会写 human user + isMeta companion；应合并为一轮。"""
    from engine.adapters.claude.codec import TURN_INDEX

    data = [
        {"type": "user", "uuid": "u1", "parentUuid": None, "promptId": "p1",
         "isSidechain": False,
         "message": {"role": "user", "content": [
             {"type": "text", "text": "看这张图 [Image #1]"},
             {"type": "image", "source": {"type": "base64", "media_type": "image/png",
                                          "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ"}},
         ]}},
        {"type": "user", "uuid": "u-meta", "parentUuid": "u1", "promptId": "p1",
         "isSidechain": False, "isMeta": True,
         "message": {"role": "user", "content": [
             {"type": "text",
              "text": "[Image: source: /tmp/.claude/image-cache/p1/1.png]"},
         ]}},
        {"type": "assistant", "uuid": "a1", "parentUuid": "u-meta",
         "isSidechain": False,
         "message": {"role": "assistant", "content": [
             {"type": "text", "text": "看到了"},
         ]}},
    ]
    dto = session_json(_roundtrip("claude", data, tmp_path))
    assert len(dto["turns"]) == 1
    assert dto["turns"][0]["turn_locator"] == "u1"
    assert [item["text"] for item in dto["turns"][0]["assistant_reply"]["items"]] == \
        ["看到了"]
    assert "u-meta" not in {m.get("uuid") for m in dto["messages"]}
    image = dto["turns"][0]["user"]["blocks"][1]["image"]
    assert image == {"id": "u1:image:1", "mime_type": "image/png", "filename": None}
    assert "iVBORw0KGgo" not in str(dto)

    spans = TURN_INDEX.turns(data)
    assert len(spans) == 1
    assert spans[0].locator == "u1"
    assert spans[0].start == 0
    assert spans[0].end == 3

    doc = _document("claude", data)
    _editor("claude").replace_reply(
        doc, dto["turns"][0]["turn_locator"],
        AssistantReply.from_dict({"items": [{"kind": "text", "text": "新回复"}]}))
    assert [item["text"] for item in _items(_roundtrip("claude", doc.data, tmp_path))
            if item["kind"] == "text"] == ["新回复"]


def test_image_blocks_normalize_codex_and_opencode(tmp_path):
    image_url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ"
    codex = [
        {"type": "session_meta", "payload": {"id": "codex-image", "cwd": "/tmp"}},
        {"type": "response_item", "payload": {"type": "message", "role": "user",
         "content": [{"type": "input_image", "image_url": image_url}]}},
    ]
    opencode = {"info": {"id": "opencode-image", "directory": "/tmp"}, "messages": [
        {"info": {"id": "message-image", "role": "user"}, "parts": [
            {"type": "file", "mime": "image/png", "filename": "diagram.png", "url": image_url},
        ]},
    ]}
    for tool, data, expected in [
        ("codex", codex, {"id": "record:1:image:0", "mime_type": "image/png", "filename": None}),
        ("opencode", opencode, {"id": "message-image:image:0", "mime_type": "image/png", "filename": "diagram.png"}),
    ]:
        dto = session_json(_roundtrip(tool, data, tmp_path))
        assert len(dto["turns"]) == 1
        assert dto["turns"][0]["user"]["blocks"] == [{"kind": "image", "image": expected}]
        assert "iVBORw0KGgo" not in str(dto)


def test_session_asset_returns_image_only_on_demand(tmp_path, monkeypatch):
    from engine.sessions import read as sessions

    data = [
        {"type": "user", "uuid": "u1", "message": {"role": "user", "content": [
            {"type": "image", "source": {"type": "base64", "media_type": "image/png",
             "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ"}},
        ]}},
    ]
    session = _roundtrip("claude", data, tmp_path)
    monkeypatch.setattr(sessions, "read_tree", lambda tool, ref, ports: session)
    assert sessions.session_asset("claude", "fixture", "u1:image:0", object()) == {
        "mime_type": "image/png", "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ",
        "filename": None,
    }
