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
from engine.domain.model import ToolCall
from engine.domain.tool_ops import CANONICAL_OPS, CanonicalOp, TOOL_OP_SPECS


ALL_FIDELITY = (CLAUDE_FIDELITY, CODEX_FIDELITY, OPENCODE_FIDELITY)
ALL_WRITERS = (CLAUDE_WRITERS, CODEX_WRITERS, OPENCODE_WRITERS)


def test_canonical_operation_specs_are_complete():
    assert set(TOOL_OP_SPECS) == CANONICAL_OPS
    assert CanonicalOp.AGENT_SPAWN in CANONICAL_OPS
    assert all(spec.required_inputs for spec in TOOL_OP_SPECS.values())


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
    call = ToolCall(name="test", op=op, input={}, output="")
    assert target.classify_tool_call(call) == fidelity[op]


def test_unknown_operation_is_always_a_degradation():
    call = ToolCall(name="test", op="web.fetch", input={}, output="")
    assert ClaudeMigrationTarget().classify_tool_call(call) == "degrade"
