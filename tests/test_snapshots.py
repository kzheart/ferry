"""快照作为内部安全网的契约。

快照页面已移除，快照对用户不再可见，但两条保护路径仍必须成立：
编辑前留底(写失败可回滚)、删除前留底(Toast 可撤销)。
"""
import json

import pytest

from engine.application import services
from engine.infrastructure.snapshots import backup_dir


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


def test_edit_leaves_a_recovery_copy_of_the_pre_edit_session(session):
    """原地编辑前必须留底，否则写坏了没有退路。"""
    before = session.read_bytes()
    services.edit_apply(str(session), [{"op": "delete-turn", "turn": 2}], tool="claude")

    snaps = _snapshots()
    assert len(snaps) == 1
    assert snaps[0].read_bytes() == before
    assert session.read_bytes() != before          # 编辑确实生效了


def test_save_as_does_not_leave_a_snapshot(session):
    """另存为不动源会话，不需要留底。"""
    services.edit_apply(str(session), [{"op": "delete-turn", "turn": 2}],
                        save_as=True, tool="claude")
    assert _snapshots() == []


def test_delete_is_undoable(session):
    """Toast 的「撤销」依赖这条链路。"""
    original = session.read_bytes()
    result = services.session_delete("claude", str(session))

    assert result["undoable"] is True
    assert not session.exists()

    services.session_undelete(result["snapshot"])
    assert session.exists()
    assert session.read_bytes() == original


def test_undelete_refuses_to_overwrite_a_live_session(session):
    from engine.domain.errors import SnapshotInvalidSourceError
    result = services.session_delete("claude", str(session))
    services.session_undelete(result["snapshot"])
    with pytest.raises(SnapshotInvalidSourceError):
        services.session_undelete(result["snapshot"])   # 源已回来，不得覆盖


def test_undelete_refuses_paths_outside_the_snapshot_dir(tmp_path):
    from engine.domain.errors import SnapshotInvalidSourceError
    stray = tmp_path / "stray.jsonl"
    stray.write_text("{}\n")
    with pytest.raises(SnapshotInvalidSourceError):
        services.session_undelete(str(stray))
