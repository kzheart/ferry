"""结构化事件与快照 reason_code 的持久化契约。"""
import json

from engine.application import services
from engine.domain.model import Session
from engine.infrastructure import snapshots


def test_lose_records_structured_event():
    sess = Session("claude", "s1", cwd=".")
    sess.lose("migration.tool_degraded", tool_name="WebSearch")
    assert sess.loss == [{"code": "migration.tool_degraded",
                          "severity": "warning",
                          "params": {"tool_name": "WebSearch"}}]


def test_snapshot_file_persists_reason_code(tmp_path, monkeypatch):
    monkeypatch.setattr(snapshots, "BACKUP_DIR", tmp_path)
    src = tmp_path / "sess.jsonl"
    src.write_text("{}\n")
    dest = snapshots.snapshot_file(src, "snapshot.before_delete", "claude")
    meta = json.loads(dest.with_suffix(".meta.json").read_text())
    assert meta["reason_code"] == "snapshot.before_delete"
    assert "reason" not in meta


def test_snapshots_listing_dual_reads_legacy_reason(tmp_path, monkeypatch):
    monkeypatch.setattr(services, "snapshot_dir", lambda: tmp_path)
    (tmp_path / "old-1000-123").with_suffix(".jsonl").write_text("{}\n")
    (tmp_path / "old-1000-123.meta.json").write_text(
        json.dumps({"reason": "手动快照", "tool": "claude"}))
    (tmp_path / "new-2000-456").with_suffix(".jsonl").write_text("{}\n")
    (tmp_path / "new-2000-456.meta.json").write_text(
        json.dumps({"reason_code": "snapshot.manual", "tool": "claude"}))
    rows = {row["session"]: row for row in services.snapshots()}
    assert rows["old-1000"]["reason_code"] is None
    assert rows["old-1000"]["legacy_reason"] == "手动快照"
    assert rows["new-2000"]["reason_code"] == "snapshot.manual"


def test_probe_report_separates_status_and_diagnostic():
    from engine.infrastructure import probes
    rep = probes.report("failed", "probe.process_failed",
                        {"tool": "codex", "exit_code": 1}, stdout="x" * 9000)
    assert rep["status"] == "failed"
    assert rep["code"] == "probe.process_failed"
    assert rep["params"] == {"tool": "codex", "exit_code": 1}
    assert rep["diagnostic"]["truncated"] is True
    assert len(rep["diagnostic"]["stdout"]) == 8000


def test_narration_defaults_to_english_and_ignores_ui_locale():
    from types import SimpleNamespace

    from engine.adapters.base import narration
    tool = SimpleNamespace(name="WebSearch", input={"q": "x"}, output="done")
    assert "[History: tool WebSearch" in narration.narrate(tool)
    assert "[History" in narration.narrate(tool, locale="en-US")
    with narration.content_locale("zh-CN"):
        assert "[历史记录" in narration.narrate(tool)
    assert "[History" in narration.narrate(tool)
