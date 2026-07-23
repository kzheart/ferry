import json

from engine.adapters.claude import reader as claude_reader
from engine.adapters.opencode import session as opencode_session
from engine.domain.tool_ops import CanonicalOp


def _tools(session):
    return [
        block.tool
        for message in session.messages
        for block in message.blocks
        if block.kind == "tool" and block.tool
    ]


def test_claude_reader_normalizes_search_web_and_opaque_tools(tmp_path):
    path = tmp_path / "claude.jsonl"
    records = [
        {
            "type": "assistant", "uuid": "assistant", "sessionId": "session",
            "cwd": "/tmp", "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "grep", "name": "Grep",
                 "input": {"pattern": "needle", "path": "src",
                           "glob": "*.py", "head_limit": 20}},
                {"type": "tool_use", "id": "fetch", "name": "WebFetch",
                 "input": {"url": "https://example.com", "prompt": "summarize"}},
                {"type": "tool_use", "id": "opaque", "name": "mcp__demo",
                 "input": {"value": 1}},
            ]},
        },
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))

    grep, fetch, opaque = _tools(claude_reader._read_transcript(path))

    assert grep.op == CanonicalOp.FS_SEARCH
    assert grep.input == {
        "query": "needle", "path": "src", "glob": "*.py", "max_results": 20,
    }
    assert fetch.op == CanonicalOp.WEB_FETCH
    assert fetch.input == {
        "url": "https://example.com", "prompt": "summarize",
    }
    assert opaque.op == CanonicalOp.TOOL_INVOKE
    assert opaque.input == {
        "namespace": "claude", "name": "mcp__demo", "input": {"value": 1},
    }


def test_opencode_reader_normalizes_patch_search_web_and_opaque_tools():
    def part(name, native_input, ordinal):
        return {
            "id": f"part-{ordinal}", "messageID": "assistant",
            "sessionID": "session", "type": "tool", "tool": name,
            "callID": f"call-{ordinal}",
            "state": {
                "status": "completed", "input": native_input, "output": "ok",
                "title": name, "metadata": {},
                "time": {"start": ordinal, "end": ordinal},
            },
        }

    payload = {
        "info": {"id": "session", "directory": "/tmp", "title": "fixture"},
        "messages": [{
            "info": {
                "id": "assistant", "sessionID": "session", "role": "assistant",
                "time": {"created": 1, "completed": 1},
            },
            "parts": [
                part("apply_patch", {
                    "patchText": "*** Begin Patch\n*** Add File: a.txt\n+x\n*** End Patch",
                }, 1),
                part("grep", {
                    "pattern": "needle", "path": "src", "include": "*.py",
                }, 2),
                part("websearch", {"query": "example", "numResults": 5}, 3),
                part("plugin_tool", {"secret": "kept-in-opaque-input"}, 4),
            ],
        }],
    }

    patch, grep, search, opaque = _tools(
        opencode_session._parse_session(payload)[0])

    assert patch.op == CanonicalOp.FS_PATCH
    assert patch.input["operations"] == [
        {"operation": "add", "path": "a.txt"}]
    assert grep.op == CanonicalOp.FS_SEARCH
    assert grep.input == {
        "query": "needle", "path": "src", "glob": "*.py",
    }
    assert search.op == CanonicalOp.WEB_SEARCH
    assert search.input == {"query": "example", "num_results": 5}
    assert opaque.op == CanonicalOp.TOOL_INVOKE
    assert opaque.input["input"] == {"secret": "kept-in-opaque-input"}


def test_opencode_repeated_task_calls_remain_distinct_from_one_tree_child(
        monkeypatch):
    def task_part(call_id, ordinal):
        return {
            "id": f"part-{ordinal}", "messageID": "assistant",
            "sessionID": "root", "type": "tool", "tool": "task",
            "callID": call_id,
            "state": {
                "status": "completed",
                "input": {
                    "description": "review", "prompt": f"pass {ordinal}",
                    "subagent_type": "general",
                },
                "output": "done", "title": "task",
                "metadata": {"sessionId": "child"},
                "time": {"start": ordinal, "end": ordinal},
            },
        }

    root = {
        "info": {"id": "root", "directory": "/tmp", "title": "root"},
        "messages": [{
            "info": {
                "id": "assistant", "sessionID": "root", "role": "assistant",
                "time": {"created": 1, "completed": 2},
            },
            "parts": [task_part("call-one", 1), task_part("call-two", 2)],
        }],
    }
    child = {
        "info": {
            "id": "child", "parentID": "root",
            "directory": "/tmp", "title": "child",
        },
        "messages": [],
    }
    exports = {"root": root, "child": child}

    class Connection:
        def execute(self, _query, params):
            sid = params[0]
            return [("child",)] if sid == "root" else []

        def close(self):
            return None

    monkeypatch.setattr(opencode_session, "_db_conn", Connection)
    monkeypatch.setattr(
        opencode_session,
        "_db_export",
        lambda _connection, session_id: exports.get(session_id),
    )

    session = opencode_session._read("root")

    assert [child.source_id for child in session.children] == ["child"]
    assert [edge.source_call_id for edge in session.agent_edges] == [
        "call-one", "call-two"]
    assert all(edge.association == "task-metadata"
               and edge.confidence == 1.0 for edge in session.agent_edges)
