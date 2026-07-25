import json
from pathlib import Path

import pytest

from engine.adapters.pi.reader import read
from engine.errors import AgentFormatChangedError
from engine.sessions.model import tool_result_text


FIXTURES = Path(__file__).parent / "fixtures" / "agent_formats" / "pi"


def _write(path, records, tail=""):
    path.write_text("\n".join(json.dumps(record) for record in records) + tail)


def _header():
    return {"type": "session", "version": 3, "id": "s",
            "timestamp": "2026-07-25T00:00:00Z", "cwd": "/private/raw"}


def _message(mid, parent, role, content):
    return {"type": "message", "id": mid, "parentId": parent,
            "timestamp": f"2026-07-25T00:00:0{len(mid)}Z",
            "message": {"role": role, "content": content,
                        "timestamp": 1784937600000}}


def test_reads_plain_fixture_without_mutating_bytes():
    path = FIXTURES / "case-01-plain" / "session.jsonl"
    before = path.read_bytes()
    session = read(str(path))

    assert path.read_bytes() == before
    assert session.source_id == "fixture-pi-plain"
    assert session.cwd == "/fixture/pi/plain"
    assert [message.role for message in session.messages] == ["user", "assistant"]
    assert session.messages[0].blocks[0].text.endswith("sk-test-fixture unchanged.")


def test_pairs_parallel_tools_images_thinking_and_missing_result(tmp_path):
    records = [
        _header(),
        _message("u", None, "user", [
            {"type": "text", "text": "/Users/raw sk-test-token"},
            {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"},
        ]),
        _message("a", "u", "assistant", [
            {"type": "thinking", "thinking": "parallel"},
            {"type": "toolCall", "id": "c1", "name": "bash",
             "arguments": {"command": "pwd", "timeout": 3}},
            {"type": "toolCall", "id": "c2", "name": "read",
             "arguments": {"path": "/Users/raw/a.txt"}},
        ]),
        _message("r", "a", "toolResult", [
            {"type": "text", "text": "/Users/raw\n"},
            {"type": "image", "data": "AA==", "mimeType": "image/png"},
        ]),
    ]
    records[-1]["message"].update(
        toolCallId="c1", toolName="bash", isError=False,
    )
    path = tmp_path / "tools.jsonl"
    _write(path, records)

    session = read(str(path))
    tools = [block.tool for message in session.messages
             for block in message.blocks if block.kind == "tool"]
    assert [tool.source_call_id for tool in tools] == ["c1", "c2"]
    assert tools[0].input == {"command": "pwd", "timeout_ms": 3000}
    assert tool_result_text(tools[0].result) == "/Users/raw\n"
    assert tools[1].result is None
    assert any(block.kind == "image" for block in session.messages[0].blocks)
    assert any(loss["code"] == "session.unpaired_tool_use" for loss in session.loss)


def test_preserves_assistant_content_order(tmp_path):
    path = tmp_path / "order.jsonl"
    _write(path, [
        _header(),
        _message("u", None, "user", "go"),
        _message("a", "u", "assistant", [
            {"type": "text", "text": "before"},
            {"type": "toolCall", "id": "c", "name": "read",
             "arguments": {"path": "/raw"}},
            {"type": "text", "text": "after"},
        ]),
    ])
    blocks = read(str(path)).messages[-1].blocks
    assert [block.kind for block in blocks] == ["text", "tool", "text"]


def test_selects_last_leaf_branch_and_reports_inactive_entries(tmp_path):
    path = tmp_path / "branch.jsonl"
    _write(path, [
        _header(),
        _message("u", None, "user", "root"),
        _message("dead", "u", "assistant", [{"type": "text", "text": "dead"}]),
        _message("live", "u", "assistant", [{"type": "text", "text": "live"}]),
    ])

    session = read(str(path))
    assert [message.source_id for message in session.messages] == ["u", "live"]
    assert session.loss[-1]["params"]["entry_ids"] == ["dead"]


def test_ignores_bad_tail_but_reports_bad_middle(tmp_path):
    path = tmp_path / "tail.jsonl"
    records = [_header(), _message("u", None, "user", "kept")]
    _write(path, records, tail="\n{broken")
    assert [message.source_id for message in read(str(path)).messages] == ["u"]

    path.write_text(json.dumps(records[0]) + "\n{broken\n" + json.dumps(records[1]))
    session = read(str(path))
    assert session.loss[0]["code"] == "session.malformed_record"


def test_rejects_non_v3_header(tmp_path):
    path = tmp_path / "old.jsonl"
    _write(path, [{**_header(), "version": 2}])
    with pytest.raises(AgentFormatChangedError):
        read(str(path))


def test_maps_bash_execution_message(tmp_path):
    path = tmp_path / "bash.jsonl"
    _write(path, [
        _header(),
        _message("u", None, "user", "run it"),
        {"type": "message", "id": "b", "parentId": "u",
         "timestamp": "2026-07-25T00:00:02Z",
         "message": {"role": "bashExecution", "command": "pwd",
                     "output": "/private/raw\n", "exitCode": 0,
                     "cancelled": False, "truncated": True,
                     "fullOutputPath": "/private/raw/full.txt", "timestamp": 2}},
    ])
    session = read(str(path))
    tool = session.messages[-1].blocks[0].tool
    assert tool.op == "shell.exec"
    assert tool.input == {"command": "pwd"}
    assert tool.result.stdout == "/private/raw\n"
    assert tool.result.truncated is True
    assert tool.result.attachments == [
        {"full_output_path": "/private/raw/full.txt"},
    ]
