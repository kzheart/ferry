import json

from engine.domain.model import tool_result_text
from engine.adapters.claude.reader import (
    _agent_id,
    _norm_input as norm_claude_input,
    _read_transcript,
    _spawns,
    _tool_result as claude_tool_result,
)
from engine.adapters.claude.writer import _agent_input as write_claude_agent_input
from engine.adapters.opencode.session import _parse_session


def _tool(session):
    return next(
        block.tool
        for message in session.messages
        for block in message.blocks
        if block.kind == "tool"
    )


def _write_jsonl(path, records):
    path.write_text("\n".join(json.dumps(record) for record in records))


def test_claude_preserves_observed_semantic_input_fields():
    assert norm_claude_input("Edit", {
        "file_path": "/fixture/input.txt",
        "old_string": "old",
        "new_string": "new",
        "replace_all": True,
    }) == {
        "file_path": "/fixture/input.txt",
        "old": "old",
        "new": "new",
        "replace_all": True,
    }
    assert norm_claude_input("Read", {
        "file_path": "/fixture/input.txt", "offset": 12, "limit": 30,
    }) == {
        "file_path": "/fixture/input.txt", "offset": 12, "limit": 30,
    }
    assert norm_claude_input("Bash", {
        "command": "fixture-command",
        "timeout": 3000,
        "run_in_background": True,
        "dangerouslyDisableSandbox": True,
    }) == {
        "command": "fixture-command",
        "timeout_ms": 3000,
        "background": True,
        "sandbox_policy": "dangerously-disable",
    }


def test_claude_normalizes_current_agent_options(tmp_path):
    assert norm_claude_input("Agent", {
        "description": "fixture",
        "prompt": "fixture prompt",
        "subagent_type": "general-purpose",
        "name": "fixture-task",
        "model": "fixture-model",
        "mode": "fork",
        "reasoning_effort": "high",
    }) == {
        "description": "fixture",
        "prompt": "fixture prompt",
        "subagent_type": "general-purpose",
        "task_name": "fixture-task",
        "model": "fixture-model",
        "fork_mode": "fork",
        "reasoning_effort": "high",
    }

    path = tmp_path / "agent-current.jsonl"
    _write_jsonl(path, [{"agentId": "fixture-agent"}])
    assert _agent_id([{"agentId": "fixture-agent"}], path) == "fixture-agent"


def test_claude_ignores_removed_native_field_aliases(tmp_path):
    assert norm_claude_input("Bash", {
        "command": "fixture-command",
        "timeout_ms": 3000,
    }) == {"command": "fixture-command"}
    assert norm_claude_input("Agent", {
        "description": "fixture",
        "prompt": "fixture prompt",
        "subagent_type": "general-purpose",
        "task_name": "legacy-task",
        "fork_mode": "legacy-mode",
    }) == {
        "description": "fixture",
        "prompt": "fixture prompt",
        "subagent_type": "general-purpose",
    }

    path = tmp_path / "session.jsonl"
    assert _agent_id([{"agent_id": "legacy-agent"}], path) is None
    assert _agent_id([{"teammate_id": "legacy-agent"}], path) is None

    result = claude_tool_result(
        {"content": "done"},
        {"exitCode": 17},
    )
    assert result.exit_code is None


def test_claude_writer_uses_current_agent_input_fields():
    native = write_claude_agent_input({
        "description": "fixture",
        "prompt": "fixture prompt",
        "subagent_type": "general-purpose",
        "task_name": "fixture-task",
        "model": "fixture-model",
        "fork_mode": "fork",
        "reasoning_effort": "high",
    })

    assert native == {
        "description": "fixture",
        "prompt": "fixture prompt",
        "subagent_type": "general-purpose",
        "name": "fixture-task",
        "model": "fixture-model",
        "mode": "fork",
        "reasoning_effort": "high",
    }
    assert "task_name" not in native
    assert "fork_mode" not in native


def test_claude_writer_keeps_named_agent_shape_without_empty_subagent_type():
    native = write_claude_agent_input({
        "description": "fixture",
        "prompt": "fixture prompt",
        "task_name": "reviewer",
    })
    assert native == {
        "description": "fixture",
        "prompt": "fixture prompt",
        "name": "reviewer",
    }


def test_claude_spawn_reads_current_agent_id(tmp_path):
    path = tmp_path / "agent-current.jsonl"
    _write_jsonl(path, [
        {
            "type": "assistant",
            "uuid": "message-use",
            "sessionId": "fixture-session",
            "cwd": "/fixture",
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "call-fixture",
                    "name": "Agent",
                    "input": {
                        "description": "fixture",
                        "prompt": "fixture",
                        "subagent_type": "general-purpose",
                    },
                }],
            },
        },
        {
            "type": "user",
            "uuid": "message-result",
            "sessionId": "fixture-session",
            "cwd": "/fixture",
            "toolUseResult": {
                "agentId": "fixture-agent",
                "status": "completed",
            },
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "call-fixture",
                    "content": "done",
                }],
            },
        },
    ])
    session = _read_transcript(path)
    assert "fixture-agent" in _spawns(session)


def test_claude_preserves_error_and_multimodal_result(tmp_path):
    path = tmp_path / "result.jsonl"
    _write_jsonl(path, [
        {
            "type": "assistant",
            "uuid": "message-use",
            "sessionId": "fixture-session",
            "cwd": "/fixture",
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "call-fixture",
                    "name": "Bash",
                    "input": {"command": "fixture-command"},
                }],
            },
        },
        {
            "type": "user",
            "uuid": "message-result",
            "sessionId": "fixture-session",
            "cwd": "/fixture",
            "toolUseResult": {
                "stdout": "fixture stdout",
                "stderr": "fixture stderr",
                "interrupted": True,
                "truncated": True,
            },
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "call-fixture",
                    "is_error": True,
                    "content": [
                        {"type": "text", "text": "fixture error"},
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": "Zml4dHVyZQ==",
                            },
                        },
                        {
                            "type": "tool_reference",
                            "tool_name": "fixture-tool",
                        },
                    ],
                }],
            },
        },
    ])

    tool = _tool(_read_transcript(path))
    assert tool_result_text(tool.result) == "fixture error"
    assert tool.result.status == "error"
    assert tool.result.stdout == "fixture stdout"
    assert tool.result.stderr == "fixture stderr"
    assert tool.result.truncated is True
    assert [block.kind for block in tool.result.blocks] == [
        "text", "image", "tool_reference",
    ]
    assert tool.result.blocks[1].mime_type == "image/png"
    assert tool.result.blocks[1].data == "Zml4dHVyZQ=="
    assert tool.result.blocks[2].metadata["tool_name"] == "fixture-tool"
    assert tool.result.metadata["claude_tool_result"]["is_error"] is True


def test_claude_preserves_interrupted_result_without_error_flag(tmp_path):
    path = tmp_path / "interrupted.jsonl"
    _write_jsonl(path, [
        {
            "type": "assistant",
            "uuid": "message-use",
            "sessionId": "fixture-session",
            "cwd": "/fixture",
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "call-fixture",
                    "name": "Bash",
                    "input": {"command": "fixture-command"},
                }],
            },
        },
        {
            "type": "user",
            "uuid": "message-result",
            "sessionId": "fixture-session",
            "cwd": "/fixture",
            "toolUseResult": {"interrupted": True},
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "call-fixture",
                    "content": "stopped",
                }],
            },
        },
    ])

    tool = _tool(_read_transcript(path))
    assert tool.result.status == "interrupted"


def test_claude_reports_malformed_jsonl_without_losing_valid_records(tmp_path):
    path = tmp_path / "truncated.jsonl"
    path.write_text("\n".join([
        '{"type":"assistant","message":{"role":"assistant","content":"kept"}}',
        '{"type":"assistant","message":{"content":"truncated',
    ]))

    session = _read_transcript(path)

    assert session.messages[0].blocks[0].text == "kept"
    assert session.loss == [{
        "code": "session.malformed_record",
        "severity": "warning",
        "params": {
            "line_number": 2,
            "error": "Unterminated string starting at",
        },
    }]


def test_opencode_preserves_bash_and_read_options():
    data = {
        "info": {
            "id": "fixture-session",
            "directory": "/fixture",
            "title": "fixture",
        },
        "messages": [{
            "info": {
                "id": "fixture-message",
                "role": "assistant",
                "time": {"created": 1},
            },
            "parts": [
                {
                    "id": "part-bash",
                    "type": "tool",
                    "tool": "bash",
                    "callID": "call-bash",
                    "state": {
                        "status": "completed",
                        "input": {
                            "command": "fixture-command",
                            "workdir": "/fixture/work",
                            "timeout": 4000,
                            "run_in_background": True,
                        },
                        "output": "done",
                    },
                },
                {
                    "id": "part-read",
                    "type": "tool",
                    "tool": "read",
                    "callID": "call-read",
                    "state": {
                        "status": "completed",
                        "input": {
                            "filePath": "/fixture/input.txt",
                            "offset": 5,
                            "limit": 10,
                        },
                        "output": "fixture",
                    },
                },
            ],
        }],
    }

    session, _ = _parse_session(data)
    tools = [
        block.tool
        for message in session.messages
        for block in message.blocks
        if block.kind == "tool"
    ]
    assert tools[0].input == {
        "command": "fixture-command",
        "workdir": "/fixture/work",
        "timeout_ms": 4000,
        "background": True,
    }
    assert tools[1].input == {
        "file_path": "/fixture/input.txt",
        "offset": 5,
        "limit": 10,
    }
    assert tools[0].result.status == "success"


def test_opencode_preserves_error_truncation_and_attachments():
    data = {
        "info": {
            "id": "fixture-session",
            "directory": "/fixture",
            "title": "fixture",
        },
        "messages": [{
            "info": {
                "id": "fixture-message",
                "role": "assistant",
                "time": {"created": 1},
            },
            "parts": [{
                "id": "part-tool",
                "type": "tool",
                "tool": "bash",
                "callID": "call-tool",
                "state": {
                    "status": "error",
                    "input": {"command": "fixture-command"},
                    "error": "fixture failure",
                    "title": "fixture result",
                    "raw": "fixture raw state",
                    "metadata": {
                        "truncated": True,
                        "exit": 17,
                    },
                    "attachments": [{
                        "id": "fixture-file",
                        "type": "file",
                        "mime": "text/plain",
                        "url": "data:text/plain;base64,Zml4dHVyZQ==",
                    }],
                },
            }],
        }],
    }

    session, _ = _parse_session(data)
    tool = _tool(session)
    assert tool_result_text(tool.result) == "fixture failure"
    assert tool.result.status == "error"
    assert tool.result.stderr == "fixture failure"
    assert tool.result.exit_code == 17
    assert tool.result.truncated is True
    assert tool.result.metadata["opencode_state"] == {
        "title": "fixture result",
        "raw": "fixture raw state",
    }
    assert tool.result.attachments == [{
        "id": "fixture-file",
        "type": "file",
        "mime": "text/plain",
        "url": "data:text/plain;base64,Zml4dHVyZQ==",
    }]
    assert [block.kind for block in tool.result.blocks] == ["text", "file"]
    assert tool.result.blocks[1].mime_type == "text/plain"


def test_opencode_running_and_interrupted_status_are_not_faked():
    base = {
        "info": {
            "id": "fixture-session",
            "directory": "/fixture",
            "title": "fixture",
        },
        "messages": [{
            "info": {
                "id": "fixture-message",
                "role": "assistant",
                "time": {"created": 1},
            },
            "parts": [{
                "id": "part-tool",
                "type": "tool",
                "tool": "bash",
                "callID": "call-tool",
                "state": {
                    "status": "running",
                    "input": {"command": "fixture-command"},
                    "metadata": {},
                },
            }],
        }],
    }
    running, _ = _parse_session(base)
    assert _tool(running).result.status == "running"

    base["messages"][0]["parts"][0]["state"]["metadata"]["interrupted"] = True
    interrupted, _ = _parse_session(base)
    assert _tool(interrupted).result.status == "interrupted"
