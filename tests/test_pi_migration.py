from pathlib import Path

import pytest

from engine.adapters.pi.migration import PiMigrationTarget
from engine.adapters.pi.reader import read
from engine.adapters.pi.writer import _records, write
from engine.sessions.model import (
    Block, Message, Session, ToolCall, text_tool_result,
)
from engine.sessions.tool_ops import CanonicalOp
from engine.system import executables


# write() 会用真实 Pi RPC 验收产物,这是刻意的集成语义,不 mock。
@pytest.mark.skipif(
    not Path(executables.argv("pi", "--version")[0]).exists(),
    reason="未安装 Pi Agent CLI",
)
def test_pi_writer_roundtrips_text_tools_and_children(tmp_path):
    call = ToolCall(
        "read", CanonicalOp.FS_READ, {"file_path": "/raw/input.txt"},
        text_tool_result("raw output"), source_call_id="call-fixed",
    )
    root = Session("fixture", "root", str(tmp_path), messages=[
        Message("user", [Block("text", "read /raw/input.txt")]),
        Message("assistant", [
            Block("text", "before"), Block("tool", tool=call),
            Block("text", "after"),
        ]),
    ])
    root.children = [Session(
        "fixture", "child", str(tmp_path),
        messages=[Message("user", [Block("text", "child")])],
    )]

    sid, path = write(root, str(tmp_path), tmp_path)
    migrated = read(str(path))

    assert migrated.source_id == sid
    assert [block.kind for block in migrated.messages[1].blocks] == [
        "text", "tool", "text",
    ]
    tool = migrated.messages[1].blocks[1].tool
    assert tool.input == {"file_path": "/raw/input.txt"}
    assert tool.result.blocks[0].text == "raw output"
    children = [item for item in tmp_path.glob("*.jsonl") if item != path]
    assert len(children) == 1
    assert read(str(children[0])).messages[0].blocks[0].text == "child"


def _content(records):
    return [item for record in records
            for item in record.get("message", {}).get("content", [])]


def test_pi_writer_narrates_tool_calls_the_target_cannot_render():
    # 外部 namespace 的 TOOL_INVOKE 在 Pi 端没有原生形态，必须走叙述降级。
    foreign = ToolCall(
        "native_lookup", CanonicalOp.TOOL_INVOKE,
        {"namespace": "codex", "name": "native_lookup", "input": {"query": "x"}},
        text_tool_result("output"),
    )
    session = Session("fixture", "root", "/tmp", messages=[
        Message("assistant", [Block("tool", tool=foreign)]),
    ])
    target = PiMigrationTarget()

    assert target.evaluate_tool(
        foreign, session, session.messages[0]).rendered is None
    kinds = [item["type"] for item in _content(
        _records(session, "/tmp", "sid", tool_decider=target.evaluate_tool))]
    assert "toolCall" not in kinds
    assert kinds == ["text"]
