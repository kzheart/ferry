from types import SimpleNamespace

from engine.adapters.opencode import session as opencode_session
from engine.adapters.opencode.codec import CODEC, TURN_INDEX
from engine.domain.authoring import AssistantReply, TextItem, ToolItem
from engine.domain.model import (
    AgentEdge,
    Block,
    Message,
    Session,
    ToolCall,
    ToolResult,
)
from engine.domain.tool_ops import CanonicalOp


def _message(mid, role, created, parts=None, completed=None):
    time = {"created": created}
    if completed is not None:
        time["completed"] = completed
    return {
        "info": {"id": mid, "sessionID": "old-session", "role": role,
                 "time": time},
        "parts": parts or [{"id": f"part-{mid}", "messageID": mid,
                            "sessionID": "old-session", "type": "text",
                            "text": mid}],
    }


def test_raw_remap_makes_tied_messages_and_parts_stably_ordered():
    task = {
        "id": "part-task", "messageID": "m1", "sessionID": "old-session",
        "type": "tool", "tool": "task", "callID": "call-1",
        "state": {"status": "completed", "input": {}, "output": "",
                  "metadata": {"parentSessionId": "old-session",
                               "sessionId": "old-child"},
                  "time": {"start": 100, "end": 100}},
    }
    payload = {
        "info": {"id": "old-session", "directory": "/old",
                 "time": {"created": 100, "updated": 100}},
        "messages": [
            _message("m1", "assistant", 100, [
                {"id": "part-z", "messageID": "m1",
                 "sessionID": "old-session", "type": "text", "text": "first"},
                task,
                {"id": "part-a", "messageID": "m1",
                 "sessionID": "old-session", "type": "text", "text": "last"},
            ], completed=100),
            _message("m2", "user", 100),
            _message("m3", "assistant", 100, completed=100),
        ],
    }

    remapped = opencode_session._remap_payload(
        payload, "new-session", "/new", None,
        {"old-session": "new-session", "old-child": "new-child"})

    created = [message["info"]["time"]["created"]
               for message in remapped["messages"]]
    assert created == [100, 101, 102]
    completed = [message["info"]["time"]["completed"]
                 for message in remapped["messages"]
                 if "completed" in message["info"]["time"]]
    assert completed == [100, 102]
    parts = remapped["messages"][0]["parts"]
    assert [part.get("text") or part.get("tool") for part in parts] == [
        "first", "task", "last"]
    assert [part["id"] for part in parts] == sorted(part["id"] for part in parts)
    metadata = parts[1]["state"]["metadata"]
    assert metadata == {"parentSessionId": "new-session",
                        "sessionId": "new-child"}


def test_replace_reply_keeps_the_original_turn_timestamp():
    payload = {
        "info": {"id": "session"},
        "messages": [
            _message("u1", "user", 100),
            _message("a1", "assistant", 110, completed=120),
            _message("u2", "user", 200),
            _message("a2", "assistant", 210, completed=220),
        ],
    }
    doc = SimpleNamespace(data=payload)
    first_turn = TURN_INDEX.turns(payload)[0]

    CODEC.replace_reply(doc, first_turn, AssistantReply((
        TextItem("replacement"), ToolItem("read", {"filePath": "/tmp/a"}, "ok"),
    )))

    replacement = doc.data["messages"][1]
    assert replacement["info"]["time"] == {"created": 110, "completed": 120}
    tool = replacement["parts"][1]
    assert tool["state"]["time"] == {"start": 110, "end": 110}
    assert [message["info"]["time"]["created"]
            for message in doc.data["messages"]] == [100, 110, 200, 210]


def test_agent_spawn_is_native_at_its_source_message_without_duplicate_text(
        tmp_path, monkeypatch):
    imported = []
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", tmp_path / "opencode.db")
    monkeypatch.setattr(
        opencode_session, "_import_payload",
        lambda payload, sid, cwd: imported.append((payload, sid)),
    )
    root = Session("claude", "root", str(tmp_path), title="root")
    root.messages = [
        Message("user", [Block("text", "before")], source_id="u1", created_at=100),
        Message("assistant", [Block("tool", tool=ToolCall(
            "Task", CanonicalOp.AGENT_SPAWN, {"prompt": "review"},
            result=ToolResult(status="success"),
            source_call_id="call-task"))], source_id="spawn-message", created_at=200),
        Message("user", [Block("text", "after")], source_id="u2", created_at=300),
        Message("assistant", [Block("text", "done")], source_id="a2", created_at=400),
    ]
    child = Session("claude", "child", str(tmp_path), title="reviewer",
                    parent_id="root")
    child.messages = [Message("assistant", [Block("text", "review complete")],
                              created_at=250)]
    root.children = [child]
    root.agent_edges = [AgentEdge(
        "root", "child", source_call_id="call-task",
        spawn_message_id="spawn-message", prompt="review",
        status="async_launched")]

    opencode_session.write(root, cwd=str(tmp_path))

    root_payload, root_sid = imported[0]
    assert len(root_payload["messages"]) == 4
    spawn = root_payload["messages"][1]
    assert spawn["info"]["time"]["created"] == 200
    assert spawn["info"]["finish"] == "tool-calls"
    assert [part.get("tool") for part in spawn["parts"]] == ["task"]
    assert not any("History: tool Task" in part.get("text", "")
                   for message in root_payload["messages"]
                   for part in message["parts"])
    task = spawn["parts"][0]
    assert task["state"]["status"] == "completed"
    assert task["state"]["metadata"]["parentSessionId"] == root_sid
    assert task["state"]["metadata"]["sessionId"] == imported[1][1]


def test_missing_native_task_link_is_inserted_at_spawn_message_before_remap(
        tmp_path, monkeypatch):
    imported = []
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", tmp_path / "opencode.db")
    monkeypatch.setattr(
        opencode_session, "_import_payload",
        lambda payload, sid, cwd: imported.append((payload, sid)),
    )
    root = Session("opencode", "old-root", str(tmp_path), title="root")
    root_payload = {
        "info": {"id": "old-root", "directory": str(tmp_path),
                 "time": {"created": 100, "updated": 300}},
        "messages": [
            _message("u1", "user", 100),
            _message("spawn", "assistant", 200, completed=200),
            _message("u2", "user", 300),
        ],
    }
    child = Session("opencode", "old-child", str(tmp_path), title="child",
                    parent_id="old-root")
    child_payload = {
        "info": {"id": "old-child", "directory": str(tmp_path),
                 "time": {"created": 200, "updated": 200}},
        "messages": [_message("ca", "assistant", 200, completed=200)],
    }
    root.children = [child]
    root.agent_edges = [AgentEdge(
        "old-root", "old-child", spawn_message_id="spawn", prompt="review")]

    opencode_session.write(
        root,
        cwd=str(tmp_path),
        native_payloads={
            "old-root": root_payload,
            "old-child": child_payload,
        },
    )

    payload, root_sid = imported[0]
    assert len(payload["messages"]) == 3
    assert [part.get("tool") for part in payload["messages"][1]["parts"]][-1] == "task"
    task = payload["messages"][1]["parts"][-1]
    assert task["state"]["metadata"] == {
        "parentSessionId": root_sid, "sessionId": imported[1][1]}
