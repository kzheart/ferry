from engine.adapters.grok.adapter import build
from engine.adapters.grok.migration import GrokMigrationTarget
from engine.adapters.grok import migration
from engine.sessions.model import (
    Block, ContextCompaction, Message, Session, ToolCall, text_tool_result,
)
from engine.sessions.tool_ops import CanonicalOp


def test_grok_adapter_enables_every_capability_except_edit():
    adapter = build()

    assert adapter.manifest.capabilities == (
        "browse", "resume", "migration-source", "migration-target",
        "delete", "probe", "models",
    )
    assert adapter.migration_target is not None
    assert adapter.verifier is not None
    assert adapter.editor is None
    assert adapter.manifest.edit_operations == ()


def test_grok_target_preserves_native_tool_payload():
    target = GrokMigrationTarget()
    session = Session("fixture", "source", "/tmp")
    tool = ToolCall(
        "read", CanonicalOp.TOOL_INVOKE,
        {"namespace": "grok", "name": "read",
         "input": {"path": "/raw/input.txt"}},
        text_tool_result("raw output"),
    )

    decision = target.evaluate_tool(tool, session)

    assert decision.fidelity == "exact"
    assert decision.rendered == {
        "kind": "tool", "name": "read",
        "input": {"path": "/raw/input.txt"},
        "output": "raw output",
    }


def test_grok_target_maps_shell_natively_and_drops_compaction():
    target = GrokMigrationTarget()
    session = Session("fixture", "source", "/tmp")
    tool = ToolCall(
        "shell", CanonicalOp.SHELL_EXEC,
        {"command": "printf test"}, text_tool_result("test"),
    )
    session.messages = []
    session.context_compactions = [
        ContextCompaction("compact-1", "fixture"),
    ]

    decision = target.evaluate_tool(tool, session)
    plan = target.plan(session)

    # 方言表里有 SHELL_EXEC 的当前代映射,迁入 Grok 是原生工具调用。
    assert decision.fidelity == "exact"
    assert decision.rendered["name"] == "run_terminal_command"
    assert plan["dropped"] == 1
    assert plan["drop_details"][0]["params"]["kind"] == "compaction"


def test_grok_target_keeps_unmapped_op_as_foreign_record():
    target = GrokMigrationTarget()
    session = Session("fixture", "source", "/tmp")
    tool = ToolCall(
        "apply_patch", CanonicalOp.FS_PATCH,
        {"operations": [{"operation": "update", "path": "a.txt"}],
         "raw_patch": "*** Begin Patch\n*** End Patch"},
        text_tool_result("ok"),
    )

    decision = target.evaluate_tool(tool, session)

    # 名称参数原样落地时,差异卡只能靠理由码说明"改写"改的是身份而不是形态。
    assert decision.fidelity == "transformed"
    assert decision.reason_code == "foreign_tool_record"
    assert decision.rendered["name"] == "apply_patch"


def test_user_role_tool_is_explicitly_narrated():
    target = GrokMigrationTarget()
    session = Session("fixture", "source", "/tmp")
    tool = ToolCall(
        "read", CanonicalOp.TOOL_INVOKE,
        {"namespace": "grok", "name": "read", "input": {"path": "x"}},
        text_tool_result("x"),
    )
    message = Message("user", [Block("tool", tool=tool)])

    decision = target.evaluate_tool(tool, session, message)

    assert decision.fidelity == "narrated"
    assert decision.rendered is None


def test_grok_target_passes_shared_tool_decider_to_writer(monkeypatch):
    captured = {}

    def fake_write(session, cwd, tool_decider=None):
        captured.update(session=session, cwd=cwd, decider=tool_decider)
        return "sid", "/tmp/sid"

    monkeypatch.setattr(migration, "write", fake_write)
    target = GrokMigrationTarget()
    session = Session("fixture", "source", "/tmp")

    assert target.write(session, "/tmp") == ("sid", "/tmp/sid")
    assert captured["session"] is session
    assert captured["cwd"] == "/tmp"
    assert captured["decider"].__self__ is target
