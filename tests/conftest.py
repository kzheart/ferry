"""测试级全局隔离。"""
import pytest

from engine.composition import create_ports


@pytest.fixture(autouse=True)
def isolate_backup_dir(tmp_path, monkeypatch):
    """任何测试都不得写用户真实快照目录（~/.resume-harness/backups）。"""
    monkeypatch.setenv("FERRY_BACKUP_DIR", str(tmp_path / "backups"))


@pytest.fixture
def ports():
    """为单个测试显式组合应用依赖，避免依赖进程全局 ports。"""
    return create_ports()
