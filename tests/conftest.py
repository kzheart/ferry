"""测试级全局隔离。"""
import pytest


@pytest.fixture(autouse=True)
def isolate_backup_dir(tmp_path, monkeypatch):
    """任何测试都不得写用户真实快照目录（~/.resume-harness/backups）。"""
    monkeypatch.setenv("FERRY_BACKUP_DIR", str(tmp_path / "backups"))
