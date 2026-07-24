"""Canonical tool-operation contract shared by every agent adapter.

Adapters normalize native calls into these operations. Writers declare which
operations they can preserve natively or only render as a degradation, so the
migration preview and the actual write path share one vocabulary.
"""
from dataclasses import dataclass
from typing import Final


class CanonicalOp:
    SHELL_EXEC: Final = "shell.exec"
    FS_READ: Final = "fs.read"
    FS_WRITE: Final = "fs.write"
    FS_EDIT: Final = "fs.edit"
    FS_PATCH: Final = "fs.patch"
    FS_SEARCH: Final = "fs.search"
    FS_GLOB: Final = "fs.glob"
    WEB_FETCH: Final = "web.fetch"
    WEB_SEARCH: Final = "web.search"
    TOOL_INVOKE: Final = "tool.invoke"
    AGENT_SPAWN: Final = "agent.spawn"


@dataclass(frozen=True)
class ToolOpSpec:
    required_inputs: tuple[str, ...]
    optional_inputs: tuple[str, ...] = ()
    nonempty_inputs: tuple[str, ...] = ()
    input_types: tuple[tuple[str, type | tuple[type, ...]], ...] = ()


TOOL_OP_SPECS: Final = {
    CanonicalOp.SHELL_EXEC: ToolOpSpec(
        ("command",),
        ("workdir", "timeout_ms", "background", "sandbox_policy"),
        ("command",),
        (
            ("command", str), ("workdir", str), ("timeout_ms", int),
            ("background", bool), ("sandbox_policy", str),
        ),
    ),
    CanonicalOp.FS_READ: ToolOpSpec(
        ("file_path",), ("offset", "limit"), ("file_path",),
        (("file_path", str), ("offset", int), ("limit", int)),
    ),
    CanonicalOp.FS_WRITE: ToolOpSpec(
        ("file_path", "content"), (), ("file_path",),
        (("file_path", str), ("content", str)),
    ),
    CanonicalOp.FS_EDIT: ToolOpSpec(
        ("file_path", "old", "new"), ("replace_all",), ("file_path",),
        (
            ("file_path", str), ("old", str), ("new", str),
            ("replace_all", bool),
        ),
    ),
    CanonicalOp.FS_PATCH: ToolOpSpec(
        ("operations",), ("raw_patch", "workdir"), (),
        (("operations", list), ("raw_patch", str), ("workdir", str)),
    ),
    CanonicalOp.FS_SEARCH: ToolOpSpec(
        ("query",), ("path", "glob", "max_results"), ("query",),
        (
            ("query", str), ("path", str), ("glob", str),
            ("max_results", int),
        ),
    ),
    CanonicalOp.FS_GLOB: ToolOpSpec(
        ("pattern",), ("path",), ("pattern",),
        (("pattern", str), ("path", str)),
    ),
    CanonicalOp.WEB_FETCH: ToolOpSpec(
        ("url",), ("method", "headers", "body", "prompt", "format",
                   "timeout_ms"), ("url",),
        (
            ("url", str), ("method", str), ("headers", dict),
            ("body", (str, dict, list)), ("prompt", str), ("format", str),
            ("timeout_ms", int),
        ),
    ),
    CanonicalOp.WEB_SEARCH: ToolOpSpec(
        ("query",), ("domains", "recency_days", "num_results"), ("query",),
        (
            ("query", str), ("domains", list), ("recency_days", int),
            ("num_results", int),
        ),
    ),
    CanonicalOp.TOOL_INVOKE: ToolOpSpec(
        ("namespace", "name", "input"), ("structure_summary", "children"),
        ("namespace", "name"),
        (
            ("namespace", str), ("name", str), ("input", (dict, str)),
            ("structure_summary", (dict, str)), ("children", list),
        ),
    ),
    CanonicalOp.AGENT_SPAWN: ToolOpSpec(
        ("description", "prompt", "subagent_type"),
        ("task_name", "model", "fork_mode", "fork_turns", "reasoning_effort"),
        ("description", "subagent_type"),
        (
            ("description", str), ("prompt", str), ("subagent_type", str),
            ("task_name", str), ("model", str), ("fork_mode", str),
            ("fork_turns", str), ("reasoning_effort", str),
        ),
    ),
}

CANONICAL_OPS: Final = frozenset(TOOL_OP_SPECS)


def has_valid_tool_input(op: str | None, value) -> bool:
    """Return whether a canonical tool call has the fields its writer needs."""
    spec = TOOL_OP_SPECS.get(op)
    if spec is None or not isinstance(value, dict):
        return False
    if any(field not in value or value[field] is None for field in spec.required_inputs):
        return False
    if not all(bool(value[field]) for field in spec.nonempty_inputs):
        return False
    for field, expected in spec.input_types:
        if field not in value or value[field] is None:
            continue
        actual = value[field]
        expected_types = expected if isinstance(expected, tuple) else (expected,)
        if int in expected_types and isinstance(actual, bool):
            return False
        if not isinstance(actual, expected_types):
            return False
    return True
