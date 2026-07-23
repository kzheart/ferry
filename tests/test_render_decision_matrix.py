import pytest

from engine.adapters.claude import writer as claude_writer
from engine.adapters.claude.migration import ClaudeMigrationTarget
from engine.adapters.codex import writer as codex_writer
from engine.adapters.codex.migration import CodexMigrationTarget
from engine.adapters.opencode import session as opencode_session
from engine.adapters.opencode.migration import OpenCodeMigrationTarget
from engine.domain.model import (
    Block, Message, Session, ToolCall, text_tool_result,
)
from engine.domain.tool_ops import CanonicalOp


OPS = [
    CanonicalOp.SHELL_EXEC,
    CanonicalOp.FS_READ,
    CanonicalOp.FS_WRITE,
    CanonicalOp.FS_EDIT,
    CanonicalOp.FS_PATCH,
    CanonicalOp.FS_SEARCH,
    CanonicalOp.FS_GLOB,
    CanonicalOp.WEB_FETCH,
    CanonicalOp.WEB_SEARCH,
    CanonicalOp.TOOL_INVOKE,
]


def _input(op, namespace):
    return {
        CanonicalOp.SHELL_EXEC: {"command": "pwd"},
        CanonicalOp.FS_READ: {"file_path": "README.md"},
        CanonicalOp.FS_WRITE: {"file_path": "new.txt", "content": "new"},
        CanonicalOp.FS_EDIT: {
            "file_path": "old.txt", "old": "old", "new": "new",
        },
        CanonicalOp.FS_PATCH: {
            "operations": [{"operation": "update", "path": "old.txt"}],
            "raw_patch": (
                "*** Begin Patch\n*** Update File: old.txt\n"
                "@@\n-old\n+new\n*** End Patch"
            ),
        },
        CanonicalOp.FS_SEARCH: {"query": "needle", "path": "src"},
        CanonicalOp.FS_GLOB: {"pattern": "*.py", "path": "src"},
        CanonicalOp.WEB_FETCH: {
            "url": "https://example.com", "prompt": "summarize",
        },
        CanonicalOp.WEB_SEARCH: {"query": "example"},
        CanonicalOp.TOOL_INVOKE: {
            "namespace": namespace, "name": "native_lookup",
            "input": {"query": "x"},
        },
    }[op]


def _session(op, namespace):
    session = Session("fixture", "source", "/tmp")
    session.messages = [Message("assistant", [Block("tool", tool=ToolCall(
        op, op, _input(op, namespace), text_tool_result("output"),
    ))])]
    return session


def _claude_has_native_tool(session, target):
    records = claude_writer._generated_lines(
        session, "fixture-session", "/tmp", claude_writer._load_templates(),
        {}, {}, tool_decider=target.evaluate_tool,
    )
    return any(
        item.get("type") == "tool_use"
        for record in records
        for item in ((record.get("message") or {}).get("content") or [])
        if isinstance(item, dict)
    )


def _codex_has_native_tool(session, target):
    records = codex_writer._session_records(
        codex_writer._load_templates(), session, "/tmp", "fixture-session",
        "fixture-session", None, 0, "/root", {},
        tool_decider=target.evaluate_tool,
    )
    return any(
        (record.get("payload") or {}).get("type") in {
            "custom_tool_call", "function_call"}
        for record in records
    )


def _opencode_has_native_tool(session, target):
    payload = opencode_session._canonical_payload(
        session, "fixture-session", "/tmp", None,
        opencode_session._template(), tool_decider=target.evaluate_tool,
    )
    return any(
        part.get("type") == "tool"
        for message in payload["messages"]
        for part in message["parts"]
    )


@pytest.mark.parametrize(("target", "namespace", "writer_dispatch"), [
    (ClaudeMigrationTarget(), "claude", _claude_has_native_tool),
    (CodexMigrationTarget(), "codex", _codex_has_native_tool),
    (OpenCodeMigrationTarget(), "opencode", _opencode_has_native_tool),
])
@pytest.mark.parametrize("op", OPS)
def test_plan_preview_and_writer_share_one_call_level_render_decision(
        target, namespace, writer_dispatch, op):
    session = _session(op, namespace)
    tool = session.messages[0].blocks[0].tool
    decision = target.evaluate_tool(tool, session, session.messages[0])
    plan = target.plan(session)
    preview = target.preview(session)

    assert plan[decision.fidelity] == 1
    if decision.fidelity == "exact":
        assert preview["differences"]["counts"]["total"] == 0
    else:
        assert preview["differences"]["counts"][decision.fidelity] == 1
    assert writer_dispatch(session, target) is (decision.rendered is not None)
