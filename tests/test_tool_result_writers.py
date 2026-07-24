import json

import pytest

from engine.adapters.claude import reader as claude_reader
from engine.adapters.claude import writer as claude_writer
from engine.adapters.claude.migration import ClaudeMigrationTarget
from engine.adapters.codex import reader as codex_reader
from engine.adapters.codex import writer as codex_writer
from engine.adapters.codex.migration import CodexMigrationTarget
from engine.adapters.opencode import payload as opencode_payload
from engine.adapters.opencode import reader as opencode_reader
from engine.adapters.opencode.migration import OpenCodeMigrationTarget
from engine.adapters.opencode.native_schema import templates as opencode_templates
from engine.sessions.model import (
    Block,
    Message,
    Session,
    ToolCall,
    ToolResult,
    ToolResultBlock,
)
from engine.sessions.tool_ops import CanonicalOp


def _source_session(status="error"):
    result = ToolResult(
        status=status,
        blocks=[
            ToolResultBlock("text", text="visible output"),
            ToolResultBlock("json", data={"count": 2}),
            ToolResultBlock(
                "file", filename="report.txt", uri="file:///tmp/report.txt",
                mime_type="text/plain",
            ),
        ],
        stdout="visible output",
        stderr="failure detail" if status == "error" else None,
        exit_code=7 if status == "error" else 0,
        truncated=False,
        attachments=[{"type": "file", "filename": "report.txt",
                      "url": "file:///tmp/report.txt"}],
    )
    tool = ToolCall(
        "shell", CanonicalOp.SHELL_EXEC, {"command": "printf test"}, result,
    )
    session = Session("fixture", "source", "/tmp")
    session.messages = [
        Message("user", [Block("text", "run")]),
        Message("assistant", [Block("tool", tool=tool)]),
    ]
    return session


def _only_result(session):
    results = [
        block.tool.result
        for message in session.messages
        for block in message.blocks
        if block.kind == "tool" and block.tool
    ]
    assert len(results) == 1
    assert results[0] is not None
    return results[0]


def _roundtrip_claude(session, tmp_path):
    target = ClaudeMigrationTarget()
    records = claude_writer._generated_lines(
        session, "fixture-session", "/tmp", claude_writer._load_templates(),
        {}, {}, tool_decider=target.evaluate_tool,
    )
    path = tmp_path / "claude.jsonl"
    path.write_text("\n".join(json.dumps(record) for record in records) + "\n")
    return claude_reader._read_transcript(path)


def _roundtrip_codex(session, tmp_path):
    target = CodexMigrationTarget()
    records = codex_writer._session_records(
        codex_writer._load_templates(), session, "/tmp", "fixture-session",
        "fixture-session", None, 0, "/root", {},
        tool_decider=target.evaluate_tool,
    )
    path = tmp_path / "codex.jsonl"
    path.write_text("\n".join(json.dumps(record) for record in records) + "\n")
    return codex_reader._read_one(path)


def _roundtrip_opencode(session, _tmp_path):
    target = OpenCodeMigrationTarget()
    payload = opencode_payload.canonical_payload(
        session, "fixture-session", "/tmp", None,
        opencode_templates(), tool_decider=target.evaluate_tool,
    )
    return opencode_reader.parse_session(payload)[0]


@pytest.mark.parametrize("roundtrip", [
    _roundtrip_claude,
    _roundtrip_codex,
    _roundtrip_opencode,
])
def test_structured_error_result_projects_to_each_native_shape(
        roundtrip, tmp_path):
    result = _only_result(roundtrip(_source_session(), tmp_path))

    assert result.status == "error"
    expected_kinds = {
        _roundtrip_claude: ["text", "text", "text"],
        _roundtrip_codex: ["text"],
        _roundtrip_opencode: ["text", "file"],
    }[roundtrip]
    assert [block.kind for block in result.blocks] == expected_kinds
    assert result.stdout == "visible output"
    assert result.stderr == "failure detail"
    assert result.exit_code == 7
    assert result.truncated is False
    if roundtrip is _roundtrip_opencode:
        assert result.attachments[0]["filename"] == "report.txt"
    else:
        assert result.attachments == []


def test_claude_writer_does_not_emit_ferry_tool_result_extension():
    records = claude_writer._generated_lines(
        _source_session(), "fixture-session", "/tmp",
        claude_writer._load_templates(), {}, {},
        tool_decider=ClaudeMigrationTarget().evaluate_tool,
    )

    assert "canonicalToolResult" not in json.dumps(records)


def test_opencode_writer_does_not_emit_ferry_tool_result_extension():
    payload = opencode_payload.canonical_payload(
        _source_session(), "fixture-session", "/tmp", None,
        opencode_templates(),
        tool_decider=OpenCodeMigrationTarget().evaluate_tool,
    )

    assert "canonicalToolResult" not in json.dumps(payload)


@pytest.mark.parametrize(("target", "status", "fidelity"), [
    (ClaudeMigrationTarget(), "running", "narrated"),
    (CodexMigrationTarget(), "pending", "narrated"),
    (OpenCodeMigrationTarget(), "running", "exact"),
])
def test_result_status_support_is_part_of_shared_render_decision(
        target, status, fidelity):
    session = _source_session(status)
    tool = session.messages[1].blocks[0].tool
    tool.result = ToolResult(
        status=status,
        blocks=[ToolResultBlock("text", text="visible output")],
    )

    decision = target.evaluate_tool(tool, session, session.messages[1])

    assert decision.fidelity == fidelity
    assert (decision.rendered is None) == (fidelity == "narrated")


@pytest.mark.parametrize(("target", "block_reason"), [
    (ClaudeMigrationTarget(), "tool_result_block_degraded"),
    (CodexMigrationTarget(), "tool_result_block_dropped"),
    (OpenCodeMigrationTarget(), "tool_result_block_dropped"),
])
def test_unrepresentable_result_data_is_reported_as_migration_loss(
        target, block_reason):
    session = _source_session()
    tool = session.messages[1].blocks[0].tool

    decision = target.evaluate_tool(tool, session, session.messages[1])

    assert decision.fidelity == "lossy"
    assert block_reason in decision.reason_codes


def test_claude_writer_roundtrip_does_not_turn_interruption_into_error(tmp_path):
    result = _only_result(_roundtrip_claude(
        _source_session("interrupted"), tmp_path))

    assert result.status == "interrupted"
