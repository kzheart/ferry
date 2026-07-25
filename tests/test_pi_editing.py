import shutil
from pathlib import Path

from engine.adapters.pi.editor import PiBackend
from engine.adapters.pi.reader import read
from engine.operations.types import AssistantReply


FIXTURE = (
    Path(__file__).parent / "fixtures" / "agent_formats" / "pi"
    / "case-01-plain" / "session.jsonl"
)


def test_pi_rewrite_replace_and_commit(tmp_path):
    path = tmp_path / "session.jsonl"
    shutil.copy(FIXTURE, path)
    editor = PiBackend()
    doc = editor.load(str(path))
    editor.apply_ops(doc, [{"op": "rewrite", "locator": "u1", "text": "/raw token"}])
    editor.replace_reply(
        doc, 1, AssistantReply.from_dict({
            "items": [{"kind": "text", "text": "replacement"}],
        }),
    )
    editor.commit(doc)

    session = read(str(path))
    assert session.messages[0].blocks[0].text == "/raw token"
    assert session.messages[1].blocks[0].text == "replacement"


def test_pi_delete_turn_preserves_valid_header(tmp_path):
    path = tmp_path / "session.jsonl"
    shutil.copy(FIXTURE, path)
    editor = PiBackend()
    doc = editor.load(str(path))
    editor.apply_ops(doc, [{"op": "delete-turn", "turn": 1}])
    editor.commit(doc)
    assert read(str(path)).messages == []
