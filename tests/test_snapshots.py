"""快照作为内部安全网的契约。

快照页面已移除，快照对用户不再可见。编辑前留底(写失败可回滚)仍必须成立；
删除是永久性的，不再落快照。
"""
import json

import pytest

from engine.operations import edit as editing
from engine.operations.delete import SessionDeletionService
from engine.system.snapshots import backup_dir


def _turns(n):
    records = []
    for i in range(n):
        records.append({"type": "user", "sessionId": "sess", "cwd": "/tmp",
                        "uuid": f"u{i}", "parentUuid": f"a{i-1}" if i else None,
                        "message": {"role": "user", "content": f"question {i}"}})
        records.append({"type": "assistant", "sessionId": "sess", "cwd": "/tmp",
                        "uuid": f"a{i}", "parentUuid": f"u{i}",
                        "message": {"role": "assistant",
                                    "content": [{"type": "text", "text": f"answer {i}"}]}})
    return records


@pytest.fixture
def session(tmp_path):
    path = tmp_path / "sess.jsonl"
    path.write_text("\n".join(json.dumps(r) for r in _turns(3)) + "\n")
    return path


def _snapshots():
    root = backup_dir()
    return sorted(root.glob("*.jsonl")) if root.exists() else []


def test_edit_leaves_a_recovery_copy_of_the_pre_edit_session(session, ports):
    """原地编辑前必须留底，否则写坏了没有退路。"""
    before = session.read_bytes()
    editor = ports.adapter("claude").editor
    editing.apply(
        editor, str(session), [{"op": "delete-turn", "turn": 2}],
    )

    snaps = _snapshots()
    assert len(snaps) == 1
    assert snaps[0].read_bytes() == before
    assert session.read_bytes() != before          # 编辑确实生效了


def test_delete_is_permanent_and_leaves_no_snapshot(session, ports):
    """删除就是删除:不落快照,快照目录保持干净。"""
    result = SessionDeletionService(ports).delete("claude", str(session))

    assert result["ok"] is True
    assert "snapshot" not in result
    assert not session.exists()
    assert _snapshots() == []
