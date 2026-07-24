from types import SimpleNamespace

from engine.adapters.claude.migration import ClaudeMigrationTarget
from engine.adapters.codex.migration import CodexMigrationTarget
from engine.operations import migrate as migration
from engine.domain.events import event
from engine.domain.model import (
    AgentEdge, Block, ImageAsset, Message, Session, ToolCall, text_tool_result,
)
from engine.domain.tool_ops import CanonicalOp


def test_preview_returns_target_session_without_mutating_source(monkeypatch, tmp_path):
    session = Session("claude", "source", str(tmp_path), title="Preview me")
    session.messages = [Message("assistant", [
        Block("text", "I inspected the file."),
        Block("tool", tool=ToolCall("Read", CanonicalOp.FS_READ,
                                    {"file_path": "README.md"},
                                    text_tool_result("contents"))),
    ])]
    target = CodexMigrationTarget()
    ports = SimpleNamespace(adapter=lambda _name: SimpleNamespace(
        migration_target=target))

    result = migration.MigrationService(ports).preview(
        "claude", "codex", "ignored", cwd=str(tmp_path), session=session,
    )

    blocks = result["preview"]["root"]["messages"][0]["blocks"]
    assert result["preview"]["target_tool"] == "codex"
    assert result["preview"]["schema_version"] == 3
    assert blocks[0]["kind"] == "text"
    assert blocks[0]["text"] == "I inspected the file."
    assert blocks[1]["name"] == "exec"
    assert blocks[1]["input"]["cmd"] == "cat README.md"
    difference = result["preview"]["differences"]["items"][0]
    assert difference["kind"] == "degraded"
    assert difference["reason_code"] == "tool_transformed"
    assert difference["source"]["label"] == "Read"
    assert difference["target"]["label"] == "exec"
    assert difference["anchor_id"] == "n:0/r:1"
    assert result["preview"]["differences"]["counts"] == {
        "total": 1, "degraded": 1, "dropped": 0,
        "exact": 1, "transformed": 1, "lossy": 0, "narrated": 0,
    }
    assert difference["fidelity"] == "transformed"
    assert difference["consumed_fields"] == ["file_path"]
    assert result["loss"]["degrade"] == 1
    assert session.loss == []


def test_unlinked_claude_agent_call_is_previewed_as_historical_text():
    session = Session("codex", "source", "/tmp/project")
    session.messages = [Message("assistant", [Block("tool", tool=ToolCall(
        "spawn_agent", CanonicalOp.AGENT_SPAWN,
        {"description": "review", "prompt": "Review this", "subagent_type": "general"},
        text_tool_result("done")))])]

    preview = ClaudeMigrationTarget().preview(session)

    block = preview["root"]["messages"][0]["blocks"][0]
    assert block["kind"] == "text"
    assert "spawn_agent" in block["text"]
    difference = preview["differences"]["items"][0]
    assert difference["kind"] == "degraded"
    assert '"prompt": "Review this"' in difference["source"]["detail"]
    assert '"output": "done"' in difference["source"]["detail"]
    assert difference["reason_code"] == "tool_to_history"
    assert session.loss == []


def test_preview_preserves_dropped_content_details_for_difference_review():
    session = Session("claude", "source", "/tmp/project")
    session.messages = [Message("assistant", [
        Block("thinking", "private reasoning"),
        Block("image", image=ImageAsset("img-1", "image/png", "data", "chart.png")),
    ])]

    preview = CodexMigrationTarget().preview(session)
    differences = preview["differences"]

    assert preview["root"]["messages"] == []
    assert differences["counts"] == {
        "total": 2, "degraded": 0, "dropped": 2,
        "exact": 0, "transformed": 0, "lossy": 0, "narrated": 0,
    }
    assert differences["items"][0]["source"]["detail"] == "private reasoning"
    assert differences["items"][0]["anchor_id"] is None
    assert differences["items"][1]["source"]["label"] == "chart.png"
    assert '"mime_type": "image/png"' in differences["items"][1]["source"]["detail"]
    assert CodexMigrationTarget().plan(session)["drop"] == 2


def test_difference_keys_are_stable_across_nested_sessions_and_anchor_visible_rounds():
    root = Session("claude", "same-id", "/tmp/project", title="Root")
    root.messages = [Message("user", [
        Block("text", "Keep this"),
        Block("thinking", "drop this"),
    ])]
    child = Session("claude", "same-id", "/tmp/project", title="Child")
    child.messages = [Message("assistant", [
        Block("thinking", "child-only"),
    ])]
    root.children = [child]

    preview = CodexMigrationTarget().preview(root)
    items = preview["differences"]["items"]

    assert preview["root"]["key"] == "n:0"
    assert preview["root"]["children"][0]["key"] == "n:0.0"
    assert items[0]["anchor_id"] == "n:0/r:1"
    assert items[1]["anchor_id"] is None
    assert items[0]["id"] != items[1]["id"]


def test_agent_preview_does_not_match_missing_call_ids_but_can_use_message_link():
    session = Session("codex", "source", "/tmp/project")
    session.agent_edges = [AgentEdge(
        parent_session_id="source", child_session_id="child",
        spawn_message_id="message-1")]
    message = Message("assistant", [Block("tool", tool=ToolCall(
        "spawn_agent", CanonicalOp.AGENT_SPAWN,
        {"description": "review", "prompt": "Review", "subagent_type": "general"},
        text_tool_result("done")))], source_id="message-1")
    session.messages = [message]

    claude_preview = ClaudeMigrationTarget().preview(session)
    codex_preview = CodexMigrationTarget().preview(session)

    assert claude_preview["differences"]["counts"]["degraded"] == 1
    assert codex_preview["differences"]["counts"]["total"] == 0
    assert codex_preview["root"]["messages"][0]["blocks"][0]["name"] == "spawn_agent"


def test_session_loss_events_are_classified_without_turning_notices_into_drops():
    session = Session("claude", "source", "/tmp/project")
    session.loss = [
        event("migration.apply_patch_unparsed"),
        event("migration.unknown_block_dropped", kind="audio"),
        event("session.subagent_unlinked", child_id="child"),
    ]

    target = CodexMigrationTarget()
    preview = target.preview(session)

    assert preview["differences"]["counts"] == {
        "total": 2, "degraded": 1, "dropped": 1,
        "exact": 0, "transformed": 0, "lossy": 0, "narrated": 1,
    }
    assert [item["kind"] for item in preview["differences"]["items"]] == [
        "degraded", "dropped"]
    assert target.plan(session)["degrade"] == 1
    assert target.plan(session)["drop"] == 1


def test_preview_reports_every_unconsumed_tool_field_with_a_reason():
    session = Session("claude", "source", "/tmp/project")
    session.messages = [Message("assistant", [Block("tool", tool=ToolCall(
        "Bash", CanonicalOp.SHELL_EXEC,
        {"command": "pwd", "workdir": "/tmp/project",
         "timeout_ms": 5000, "background": True},
        text_tool_result("done"),
    ))])]

    preview = CodexMigrationTarget().preview(session)
    difference, = preview["differences"]["items"]

    assert difference["fidelity"] == "lossy"
    assert difference["consumed_fields"] == ["command", "workdir"]
    assert difference["ignored_fields"] == ["background", "timeout_ms"]
    assert difference["reason_codes"] == ["unsupported_tool_fields"]
