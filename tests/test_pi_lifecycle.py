import shutil
from pathlib import Path

from engine.adapters.pi.adapter import PiBrowser
from engine.adapters.pi.editor import PiBackend
from engine.adapters.pi.lifecycle import PiLifecycle


FIXTURE = (
    Path(__file__).parent / "fixtures" / "agent_formats" / "pi"
    / "case-01-plain" / "session.jsonl"
)


class Adapter:
    browser = PiBrowser()
    editor = PiBackend()


def test_pi_resume_uses_absolute_file(tmp_path):
    path = tmp_path / "session.jsonl"
    shutil.copy(FIXTURE, path)
    lifecycle = PiLifecycle()
    descriptor = lifecycle.resume_descriptor(str(path), str(tmp_path))
    assert descriptor["args"] == ["--session", str(path.resolve())]


def test_pi_delete_is_permanent(tmp_path, monkeypatch):
    path = tmp_path / "session.jsonl"
    shutil.copy(FIXTURE, path)
    backup = tmp_path / "backups"
    monkeypatch.setenv("FERRY_BACKUP_DIR", str(backup))
    lifecycle = PiLifecycle()
    result = lifecycle.delete(Adapter(), str(path))
    assert result["ok"] is True
    assert "snapshot" not in result
    assert not path.exists()
    assert not backup.exists()
