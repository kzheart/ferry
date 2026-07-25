from engine.adapters.pi.reader import read
from engine.adapters.pi.writer import write
from engine.sessions.model import (
    Block, Message, Session, ToolCall, text_tool_result,
)
from engine.sessions.tool_ops import CanonicalOp


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
