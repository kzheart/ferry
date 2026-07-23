"""Focused tests for the canonical tool call/result contract."""

import pytest

from engine.domain.model import (
    TOOL_RESULT_BLOCK_KINDS,
    TOOL_RESULT_STATUSES,
    ToolCall,
    ToolResult,
    ToolResultBlock,
    normalize_tool_result_status,
)
from engine.domain.tool_ops import (
    CANONICAL_OPS,
    TOOL_OP_SPECS,
    CanonicalOp,
    has_valid_tool_input,
)


def test_legacy_output_remains_a_string_and_tracks_late_reader_pairing():
    call = ToolCall("Read", CanonicalOp.FS_READ, {"file_path": "safe.txt"}, "")

    call.output = "paired later"

    assert call.output == "paired later"
    assert call.tool_result.status == "unknown"
    assert call.tool_result.blocks == [
        ToolResultBlock("text", text="paired later"),
    ]


def test_structured_result_can_be_passed_as_output_with_legacy_access():
    result = ToolResult(
        status="completed",
        blocks=[
            ToolResultBlock("text", text="done"),
            ToolResultBlock("json", data={"count": 2}),
            ToolResultBlock(
                "image", mime_type="image/png", filename="preview.png",
                data="base64-data",
            ),
        ],
        stdout="stdout",
        stderr="warning",
        exit_code=0,
        truncated=False,
        attachments=[{"kind": "file", "name": "report.txt"}],
        metadata={"duration_ms": 12},
    )

    call = ToolCall("tool", None, {}, result)

    assert call.output == 'done\n{"count":2}'
    assert call.result is result
    assert call.tool_result is result
    assert call.status == "success"
    assert call.tool_result.blocks[2].kind == "image"


def test_explicit_result_keeps_empty_legacy_output_usable():
    call = ToolCall(
        "tool",
        None,
        {},
        "",
        result=ToolResult(
            status="error",
            blocks=[ToolResultBlock("text", text="failed")],
        ),
    )

    assert call.output == "failed"
    assert call.status == "error"


def test_set_result_updates_the_legacy_view_and_status():
    call = ToolCall("tool", None, {}, "old", status="pending")

    call.set_result(ToolResult(
        status="interrupted",
        blocks=[ToolResultBlock("text", text="stopped")],
    ))

    assert call.output == "stopped"
    assert call.status == "interrupted"
    assert call.result is call.tool_result


def test_legacy_result_extracts_known_metadata_without_dropping_it():
    metadata = {
        "stdout": "out",
        "stderr": "err",
        "exit": 7,
        "truncated": True,
        "attachments": [{"name": "log.txt"}],
        "native_field": "preserved",
    }

    result = ToolResult.from_legacy(
        "legacy text", status="failed", metadata=metadata,
    )

    assert result.status == "error"
    assert result.stdout == "out"
    assert result.stderr == "err"
    assert result.exit_code == 7
    assert result.truncated is True
    assert result.attachments == [{"name": "log.txt"}]
    assert result.metadata == metadata
    assert result.legacy_output() == "legacy text"


@pytest.mark.parametrize("status", sorted(TOOL_RESULT_STATUSES))
def test_canonical_result_statuses_are_stable(status):
    assert normalize_tool_result_status(status) == status


def test_unknown_native_status_is_preserved_as_metadata():
    result = ToolResult(status="native-future-state")

    assert result.status == "unknown"
    assert result.metadata["source_status"] == "native-future-state"


def test_result_contract_rejects_ambiguous_bool_integer():
    with pytest.raises(TypeError, match="not bool"):
        ToolResult(exit_code=True)


def test_result_block_contract_rejects_unknown_kind():
    assert TOOL_RESULT_BLOCK_KINDS == {
        "text", "json", "image", "file", "tool_reference",
    }
    with pytest.raises(ValueError, match="unsupported"):
        ToolResultBlock("native-secret-kind")


def _valid_inputs():
    return {
        CanonicalOp.SHELL_EXEC: {
            "command": "pwd", "workdir": "/workspace", "timeout_ms": 1000,
            "background": False, "sandbox_policy": "default",
        },
        CanonicalOp.FS_READ: {
            "file_path": "file.txt", "offset": 1, "limit": 20,
        },
        CanonicalOp.FS_WRITE: {
            "file_path": "file.txt", "content": "",
        },
        CanonicalOp.FS_EDIT: {
            "file_path": "file.txt", "old": "", "new": "",
            "replace_all": True,
        },
        CanonicalOp.FS_PATCH: {
            "operations": [
                {
                    "action": "move", "file_path": "old.txt",
                    "destination": "new.txt", "hunks": [],
                },
            ],
            "raw_patch": "*** Begin Patch",
        },
        CanonicalOp.FS_SEARCH: {
            "query": "needle", "path": "src", "glob": "*.py",
            "max_results": 10,
        },
        CanonicalOp.FS_GLOB: {
            "pattern": "**/*.py", "path": "src",
        },
        CanonicalOp.WEB_FETCH: {
            "url": "https://example.invalid", "method": "GET",
            "headers": {"accept": "application/json"},
        },
        CanonicalOp.WEB_SEARCH: {
            "query": "canonical tools", "domains": ["example.invalid"],
            "recency_days": 30,
        },
        CanonicalOp.TOOL_INVOKE: {
            "namespace": "mcp.example", "name": "custom",
            "input": {"opaque": True},
        },
        CanonicalOp.AGENT_SPAWN: {
            "description": "review", "prompt": "inspect",
            "subagent_type": "general", "task_name": "reviewer",
            "model": "agent-model", "fork_mode": "all",
            "reasoning_effort": "high",
        },
    }


def test_extended_operation_specs_accept_observed_fields_and_extra_fields():
    values = _valid_inputs()

    assert set(values) == CANONICAL_OPS == set(TOOL_OP_SPECS)
    for op, input_value in values.items():
        with_extra = {**input_value, "native_extra": {"kept": True}}
        assert has_valid_tool_input(op, with_extra), op
        assert with_extra["native_extra"] == {"kept": True}


@pytest.mark.parametrize(
    ("op", "field", "invalid"),
    [
        (CanonicalOp.SHELL_EXEC, "timeout_ms", True),
        (CanonicalOp.FS_READ, "offset", False),
        (CanonicalOp.FS_READ, "limit", "20"),
        (CanonicalOp.FS_EDIT, "replace_all", 1),
        (CanonicalOp.FS_SEARCH, "max_results", True),
        (CanonicalOp.WEB_SEARCH, "recency_days", False),
    ],
)
def test_input_type_validation_does_not_allow_bool_to_impersonate_int(
    op, field, invalid,
):
    input_value = _valid_inputs()[op]
    input_value[field] = invalid

    assert not has_valid_tool_input(op, input_value)


def test_input_type_validation_checks_required_and_optional_values():
    assert not has_valid_tool_input(
        CanonicalOp.FS_READ, {"file_path": 42},
    )
    assert not has_valid_tool_input(
        CanonicalOp.WEB_FETCH,
        {"url": "https://example.invalid", "headers": ["not", "a", "map"]},
    )
    assert has_valid_tool_input(
        CanonicalOp.FS_READ,
        {"file_path": "file.txt", "offset": None, "source_specific": 1},
    )
