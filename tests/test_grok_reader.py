from pathlib import Path

from engine.adapters.grok.reader import read


FIXTURES = Path(__file__).parent / "fixtures" / "agent_formats" / "grok"


def test_updates_are_primary_and_chunks_stay_one_message():
    session = read(str(FIXTURES / "case-02-tools"))
    assert [message.role for message in session.messages] == ["user", "assistant"]
    assistant = session.messages[1]
    assert [block.kind for block in assistant.blocks] == ["text", "tool"]
    assert assistant.blocks[0].text == "Inspecting now."
    tool = assistant.blocks[-1].tool
    assert tool.source_call_id == "tool-1"
    assert tool.input["input"]["path"] == "/fixture/grok/tools/input.txt"
    assert tool.result.blocks[0].data["FileContent"]["content"] == "sk-test-fixture"


def test_rewind_dead_branch_is_not_visible():
    session = read(str(FIXTURES / "case-03-rewind"))
    text = " ".join(block.text for message in session.messages
                    for block in message.blocks if block.kind == "text")
    assert "live" in text
    assert "dead" not in text


def test_chat_v1_fallback_preserves_raw_path():
    session = read(str(FIXTURES / "case-04-chat-fallback"))
    assert session.messages[0].blocks[0].text == "fallback /fixture/grok/chat"
