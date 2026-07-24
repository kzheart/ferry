"""结构化事件与快照 reason_code 的持久化契约。"""
import json

from engine.sessions.model import Session
from engine.storage import snapshots


def test_lose_records_structured_event():
    sess = Session("claude", "s1", cwd=".")
    sess.lose("migration.tool_degraded", tool_name="WebSearch")
    assert sess.loss == [{"code": "migration.tool_degraded",
                          "severity": "warning",
                          "params": {"tool_name": "WebSearch"}}]


def test_snapshot_file_persists_reason_code(tmp_path, monkeypatch):
    monkeypatch.setenv("FERRY_BACKUP_DIR", str(tmp_path))
    src = tmp_path / "sess.jsonl"
    src.write_text("{}\n")
    dest = snapshots.snapshot_file(src, "snapshot.before_delete", "claude")
    meta = json.loads(dest.with_suffix(".meta.json").read_text())
    assert meta["reason_code"] == "snapshot.before_delete"
    assert "reason" not in meta
