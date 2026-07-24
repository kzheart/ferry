"""快照作为内部安全网的契约。

快照页面已移除，快照对用户不再可见，但两条保护路径仍必须成立：
编辑前留底(写失败可回滚)、删除前留底(Toast 可撤销)。
"""
import json

import pytest

from engine.operations import edit as editing
from engine.operations.delete import SessionDeletionService
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


def test_delete_is_undoable(session, ports):
    """Toast 的「撤销」依赖这条链路。"""
    original = session.read_bytes()
    service = SessionDeletionService(ports)
    result = service.delete("claude", str(session))

    assert result["undoable"] is True
    assert not session.exists()

    service.restore(result["snapshot"])
    assert session.exists()
    assert session.read_bytes() == original


def test_undelete_refuses_to_overwrite_a_live_session(session, ports):
    from engine.domain.errors import SnapshotInvalidSourceError
    service = SessionDeletionService(ports)
    result = service.delete("claude", str(session))
    service.restore(result["snapshot"])
    with pytest.raises(SnapshotInvalidSourceError):
        service.restore(result["snapshot"])   # 源已回来，不得覆盖


def test_undelete_refuses_paths_outside_the_snapshot_dir(tmp_path, ports):
    from engine.domain.errors import SnapshotInvalidSourceError
    stray = tmp_path / "stray.jsonl"
    stray.write_text("{}\n")
    with pytest.raises(SnapshotInvalidSourceError):
        SessionDeletionService(ports).restore(str(stray))


def test_undelete_routes_snapshot_to_its_adapter_lifecycle(monkeypatch):
    root = backup_dir()
    root.mkdir(parents=True)
    snapshot = root / "session.jsonl"
    snapshot.write_text("{}\n")
    snapshot.with_suffix(".meta.json").write_text(json.dumps({
        "tool": "fake", "source": "/work/session.jsonl",
    }))
    calls = []

    class Lifecycle:
        def restore_delete(self, snap, meta):
            calls.append((snap, meta))
            return {"ok": True, "target": meta["source"]}

    class Plugin:
        lifecycle = Lifecycle()

    ports = type("Ports", (), {
        "snapshot_dir": lambda _self: root,
        "adapter": lambda _self, tool: Plugin() if tool == "fake" else None,
    })()

    assert SessionDeletionService(ports).restore(str(snapshot)) == {
        "ok": True, "target": "/work/session.jsonl"}
    assert calls == [(snapshot, {"tool": "fake", "source": "/work/session.jsonl"})]
