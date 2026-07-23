from types import SimpleNamespace

import pytest

from engine.adapters.opencode import session as opencode_session
from engine.adapters.opencode.codec import CODEC, TURN_INDEX
from engine.domain.authoring import AssistantReply, TextItem
from engine.domain.model import (
    AgentEdge,
    Block,
    Message,
    Session,
    ToolCall,
    ToolResult,
)
from engine.domain.tool_ops import CanonicalOp


def _text_part(mid, text):
    return {"id": f"old-{mid}-{text}", "messageID": mid,
            "sessionID": "old-root", "type": "text", "text": text}


def _native_message(mid, role, created, parts=None, completed=None):
    time = created if isinstance(created, dict) else {"created": created}
    if completed is not None and isinstance(time, dict):
        time["completed"] = completed
    return {"info": {"id": mid, "sessionID": "old-root", "role": role,
                     "time": time},
            "parts": parts if parts is not None else [_text_part(mid, mid)]}


def _tree_with_children(tmp_path, count=2):
    root = Session("claude", "root", str(tmp_path), title="root")
    calls = []
    root.agent_edges = []
    for index in range(count):
        child_id = f"child-{index}"
        call_id = f"call-{index}"
        child = Session("claude", child_id, str(tmp_path), title=child_id,
                        parent_id="root")
        child.messages = [Message("assistant", [Block("text", f"result-{index}")],
                                  created_at=200 + index)]
        root.children.append(child)
        root.agent_edges.append(AgentEdge(
            "root", child_id, source_call_id=call_id,
            spawn_message_id="spawn", prompt=f"prompt-{index}"))
        calls.append(Block("tool", tool=ToolCall(
            "Task", CanonicalOp.AGENT_SPAWN, {"prompt": f"prompt-{index}"},
            result=ToolResult(status="success"),
            source_call_id=call_id)))
    root.messages = [
        Message("user", [Block("text", "before")], source_id="u1", created_at=100),
        Message("assistant", calls, source_id="spawn", created_at=200),
        Message("user", [Block("text", "after")], source_id="u2", created_at=300),
    ]
    return root


def test_remap_normalizes_missing_and_non_dict_time_fields():
    payload = {
        "info": {"id": "old-root", "directory": "/old", "time": None},
        "messages": [
            _native_message("m1", "user", None),
            _native_message("m2", "assistant", None, completed=10),
            _native_message("m3", "user", "invalid-time"),
        ],
    }

    remapped = opencode_session._remap_payload(
        payload, "new-root", "/new", None, {"old-root": "new-root"})

    created = [message["info"]["time"]["created"]
               for message in remapped["messages"]]
    assert created == list(range(created[0], created[0] + 3))
    assert remapped["info"]["time"]["updated"] == created[-1]


def test_empty_native_payload_gets_a_valid_session_time():
    payload = {"info": {"id": "old-root", "directory": "/old", "time": None},
               "messages": []}

    remapped = opencode_session._remap_payload(
        payload, "new-root", "/new", None, {"old-root": "new-root"})

    assert isinstance(remapped["info"]["time"]["created"], int)
    assert remapped["info"]["time"]["updated"] >= \
        remapped["info"]["time"]["created"]


def test_replace_reply_without_old_assistant_creates_a_complete_time_record():
    payload = {"info": {"id": "session"}, "messages": [
        _native_message("u1", "user", 100),
    ]}
    payload["messages"][0]["info"]["sessionID"] = "session"
    doc = SimpleNamespace(data=payload)

    CODEC.replace_reply(
        doc, TURN_INDEX.turns(payload)[0], AssistantReply((TextItem("answer"),)))

    reply = doc.data["messages"][1]
    assert reply["info"]["time"] == {"created": 101, "completed": 101}


def test_multiple_tasks_in_one_message_link_distinct_children(
        tmp_path, monkeypatch):
    imported = []
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", tmp_path / "opencode.db")
    monkeypatch.setattr(opencode_session, "_import_payload",
                        lambda payload, sid, cwd: imported.append((payload, sid)))
    root = _tree_with_children(tmp_path)

    opencode_session.write(root, cwd=str(tmp_path))

    payload, root_sid = imported[0]
    spawn = payload["messages"][1]
    tasks = [part for part in spawn["parts"] if part.get("tool") == "task"]
    assert len(tasks) == 2
    assert [task["callID"] for task in tasks] == ["call-0", "call-1"]
    assert [task["state"]["metadata"]["sessionId"] for task in tasks] == [
        imported[1][1], imported[2][1]]
    assert all(task["state"]["metadata"]["parentSessionId"] == root_sid
               for task in tasks)
    assert [task["id"] for task in tasks] == sorted(task["id"] for task in tasks)


def test_multiple_tasks_match_children_by_call_id_not_edge_order(
        tmp_path, monkeypatch):
    imported = []
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", tmp_path / "opencode.db")
    monkeypatch.setattr(opencode_session, "_import_payload",
                        lambda payload, sid, cwd: imported.append((payload, sid)))
    root = _tree_with_children(tmp_path)
    root.messages[1].blocks.reverse()

    opencode_session.write(root, cwd=str(tmp_path))

    tasks = [part for part in imported[0][0]["messages"][1]["parts"]
             if part.get("tool") == "task"]
    assert [task["callID"] for task in tasks] == ["call-1", "call-0"]
    assert [task["state"]["metadata"]["sessionId"] for task in tasks] == [
        imported[2][1], imported[1][1]]


def test_duplicate_edge_does_not_duplicate_task_part(tmp_path, monkeypatch):
    imported = []
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", tmp_path / "opencode.db")
    monkeypatch.setattr(opencode_session, "_import_payload",
                        lambda payload, sid, cwd: imported.append(payload))
    root = _tree_with_children(tmp_path, count=1)
    root.agent_edges.append(root.agent_edges[0])

    opencode_session.write(root, cwd=str(tmp_path))

    tasks = [part for message in imported[0]["messages"]
             for part in message["parts"] if part.get("tool") == "task"]
    assert len(tasks) == 1


def test_child_without_edge_and_empty_parent_gets_a_synthetic_user(
        tmp_path, monkeypatch):
    imported = []
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", tmp_path / "opencode.db")
    monkeypatch.setattr(opencode_session, "_import_payload",
                        lambda payload, sid, cwd: imported.append(payload))
    root = Session("claude", "root", str(tmp_path), title="root")
    child = Session("claude", "child", str(tmp_path), title="child",
                    parent_id="root")
    child.messages = [Message("assistant", [Block("text", "result")])]
    root.children = [child]

    opencode_session.write(root, cwd=str(tmp_path))

    messages = imported[0]["messages"]
    assert [message["info"]["role"] for message in messages] == [
        "user", "assistant"]
    assert messages[1]["info"]["parentID"] == messages[0]["info"]["id"]
    assert messages[1]["parts"][0]["tool"] == "task"


def test_empty_native_parent_with_missing_time_can_link_a_child(
        tmp_path, monkeypatch):
    imported = []
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", tmp_path / "opencode.db")
    monkeypatch.setattr(opencode_session, "_import_payload",
                        lambda payload, sid, cwd: imported.append(payload))
    root = Session("opencode", "root", str(tmp_path), title="root")
    root_payload = {
        "info": {"id": "root", "directory": str(tmp_path), "time": None},
        "messages": [],
    }
    child = Session("opencode", "child", str(tmp_path), title="child",
                    parent_id="root")
    child_payload = {
        "info": {"id": "child", "directory": str(tmp_path),
                 "time": {"created": 100, "updated": 100}},
        "messages": [],
    }
    root.children = [child]

    opencode_session.write(
        root,
        cwd=str(tmp_path),
        native_payloads={"root": root_payload, "child": child_payload},
    )

    assert [message["info"]["role"] for message in imported[0]["messages"]] == [
        "user", "assistant"]
    assert imported[0]["info"]["time"]["updated"] >= \
        imported[0]["info"]["time"]["created"]


def test_native_payload_keeps_multiple_tasks_without_adding_duplicates(
        tmp_path, monkeypatch):
    imported = []
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", tmp_path / "opencode.db")
    monkeypatch.setattr(opencode_session, "_import_payload",
                        lambda payload, sid, cwd: imported.append((payload, sid)))
    root = _tree_with_children(tmp_path)
    task_parts = []
    for index in range(2):
        task_parts.append({
            "id": f"old-task-{index}", "messageID": "spawn",
            "sessionID": "root", "type": "tool", "tool": "task",
            "callID": f"call-{index}",
            "state": {"status": "completed", "input": {}, "output": "",
                      "metadata": {"parentSessionId": "root",
                                   "sessionId": f"child-{index}"},
                      "time": {"start": 200, "end": 200}},
        })
    root.source_tool = "opencode"
    root_payload = {
        "info": {"id": "root", "directory": str(tmp_path),
                 "time": {"created": 100, "updated": 300}},
        "messages": [
            _native_message("u1", "user", 100),
            _native_message("spawn", "assistant", 200, task_parts, completed=200),
            _native_message("u2", "user", 300),
        ],
    }

    opencode_session.write(
        root,
        cwd=str(tmp_path),
        native_payloads={"root": root_payload},
    )

    payload, root_sid = imported[0]
    tasks = [part for part in payload["messages"][1]["parts"]
             if part.get("tool") == "task"]
    assert len(tasks) == 2
    assert all(task["state"]["metadata"]["parentSessionId"] == root_sid
               for task in tasks)
    assert [task["state"]["metadata"]["sessionId"] for task in tasks] == [
        imported[1][1], imported[2][1]]


def test_second_import_failure_rolls_back_child_then_parent(tmp_path, monkeypatch):
    attempts = []
    deleted = []
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", tmp_path / "opencode.db")

    def fail_second(payload, sid, cwd):
        attempts.append(sid)
        if len(attempts) == 2:
            raise RuntimeError("child import failed")

    monkeypatch.setattr(opencode_session, "_import_payload", fail_second)
    monkeypatch.setattr(
        opencode_session, "_oc",
        lambda args, **kwargs: deleted.append(args[2]) if args[:2] == ["session", "delete"] else "",
    )
    root = _tree_with_children(tmp_path, count=1)

    with pytest.raises(RuntimeError, match="child import failed"):
        opencode_session.write(root, cwd=str(tmp_path))

    assert deleted == list(reversed(attempts))
