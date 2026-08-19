"""测试级全局隔离。"""
import pytest

from engine.bootstrap import create_context


@pytest.fixture(autouse=True)
def isolate_backup_dir(tmp_path):
    """任何测试都不得写用户真实目录（~/.ferry）。

    用独立的 MonkeyPatch 实例：`monkeypatch` fixture 是函数级共享的，测试内部
    一句 `monkeypatch.undo()` 会连这层隔离一起撤掉，从而把缓存写进真实 ~/.ferry。
    """
    with pytest.MonkeyPatch.context() as patch:
        patch.setenv("FERRY_BACKUP_DIR", str(tmp_path / "backups"))
        patch.setenv("FERRY_DATA_DIR", str(tmp_path / "data"))
        yield


@pytest.fixture
def ports():
    """为单个测试显式组合应用依赖，避免依赖进程全局 ports。"""
    return create_context()
