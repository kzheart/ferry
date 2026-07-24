"""Canonical tool-operation and target-fidelity contract tests."""
import pytest

from engine.adapters.claude.migration import ClaudeMigrationTarget
from engine.adapters.claude.writer import OP_FIDELITY as CLAUDE_FIDELITY
from engine.adapters.claude.writer import OP_WRITERS as CLAUDE_WRITERS
from engine.adapters.codex.migration import CodexMigrationTarget
from engine.adapters.codex.writer import OP_FIDELITY as CODEX_FIDELITY
from engine.adapters.codex.writer import OP_WRITERS as CODEX_WRITERS
from engine.adapters.opencode.migration import OpenCodeMigrationTarget
from engine.adapters.opencode.session import OP_FIDELITY as OPENCODE_FIDELITY
from engine.adapters.opencode.session import OP_WRITERS as OPENCODE_WRITERS
from engine.sessions.model import ToolCall, text_tool_result
from engine.sessions.tool_ops import (
    CANONICAL_OPS, CanonicalOp, TOOL_OP_SPECS, has_valid_tool_input,
)


ALL_FIDELITY = (CLAUDE_FIDELITY, CODEX_FIDELITY, OPENCODE_FIDELITY)
ALL_WRITERS = (CLAUDE_WRITERS, CODEX_WRITERS, OPENCODE_WRITERS)


def _valid_input(op):
    values = {
        CanonicalOp.SHELL_EXEC: {"command": "pwd"},
        CanonicalOp.FS_READ: {"file_path": "/work/file"},
        CanonicalOp.FS_WRITE: {"file_path": "/work/file", "content": ""},
        CanonicalOp.FS_EDIT: {"file_path": "/work/file", "old": "", "new": ""},
        CanonicalOp.FS_PATCH: {
            "operations": [{"operation": "update", "path": "/work/file"}],
            "raw_patch": "*** Begin Patch\n*** End Patch",
        },
        CanonicalOp.FS_SEARCH: {"query": "needle"},
        CanonicalOp.FS_GLOB: {"pattern": "*.py"},
        CanonicalOp.WEB_FETCH: {"url": "https://example.com"},
        CanonicalOp.WEB_SEARCH: {"query": "example"},
        CanonicalOp.TOOL_INVOKE: {
            "namespace": "mcp", "name": "lookup", "input": {"query": "x"},
        },
        CanonicalOp.AGENT_SPAWN: {
            "description": "delegate", "prompt": "review", "subagent_type": "general",
        },
    }
    return values[op]


def test_canonical_operation_specs_are_complete():
    assert set(TOOL_OP_SPECS) == CANONICAL_OPS
    assert CanonicalOp.AGENT_SPAWN in CANONICAL_OPS
    assert all(spec.required_inputs for spec in TOOL_OP_SPECS.values())


@pytest.mark.parametrize("op", sorted(CANONICAL_OPS))
def test_canonical_operation_schema_rejects_incomplete_input(op):
    assert has_valid_tool_input(op, _valid_input(op))
    assert not has_valid_tool_input(op, {})


@pytest.mark.parametrize("fidelity", ALL_FIDELITY)
def test_target_fidelity_only_declares_canonical_operations(fidelity):
    assert set(fidelity) <= CANONICAL_OPS
    assert set(fidelity.values()) <= {"native", "degrade"}


@pytest.mark.parametrize("writers,fidelity", zip(ALL_WRITERS, ALL_FIDELITY))
def test_every_writer_mapping_has_a_fidelity_verdict(writers, fidelity):
    assert set(writers) <= CANONICAL_OPS
    assert set(writers) <= set(fidelity)


@pytest.mark.parametrize(("target", "fidelity"), [
    (ClaudeMigrationTarget(), CLAUDE_FIDELITY),
    (CodexMigrationTarget(), CODEX_FIDELITY),
    (OpenCodeMigrationTarget(), OPENCODE_FIDELITY),
])
@pytest.mark.parametrize("op", sorted(CANONICAL_OPS))
def test_migration_preview_uses_the_full_target_mapping_matrix(target, fidelity, op):
    call = ToolCall(name="test", op=op, input=_valid_input(op))
    assert target.classify_tool_call(call) == fidelity[op]


def test_unknown_operation_is_always_a_degradation():
    call = ToolCall(name="test", op="web.fetch", input={})
    assert ClaudeMigrationTarget().classify_tool_call(call) == "degrade"


def test_invalid_input_is_a_degradation_even_for_a_supported_operation():
    call = ToolCall(name="test", op=CanonicalOp.FS_READ, input={})
    assert ClaudeMigrationTarget().classify_tool_call(call) == "degrade"


def test_unknown_explicit_result_is_narrated_instead_of_fabricating_success():
    from engine.sessions.model import Session, ToolResult

    call = ToolCall(
        name="Read", op=CanonicalOp.FS_READ,
        input={"file_path": "/work/file"},
        result=text_tool_result("contents", status="unknown"),
    )
    decision = ClaudeMigrationTarget().evaluate_tool(
        call, Session("codex", "source", "/work"))

    assert decision.fidelity == "narrated"
    assert decision.rendered is None
    assert decision.reason_code == "unknown_result_status"
