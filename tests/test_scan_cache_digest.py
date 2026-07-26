"""ScanCache 的内容摘要持久化:冷启动不该把整个会话库重新哈希一遍。"""

import os
from concurrent.futures import ThreadPoolExecutor

import pytest

from engine.sessions.index import _path_identity
from engine.sessions.scan_cache import ScanCache, shared_cache


@pytest.fixture()
def session_file(tmp_path):
    path = tmp_path / "session.jsonl"
    path.write_text('{"role":"user"}\n')
    return path


def test_first_scan_writes_digest_to_disk(tmp_path, session_file):
    cache = ScanCache(tmp_path / "scan-cache.json")
    identity = _path_identity(session_file, {}, cache)
    cache.flush()

    reopened = ScanCache(tmp_path / "scan-cache.json")
    assert reopened.get_digest(session_file, session_file.stat()) == identity[4]


def test_reopened_process_hits_cache_without_reading_content(
    tmp_path, session_file, monkeypatch,
):
    cache = ScanCache(tmp_path / "scan-cache.json")
    identity = _path_identity(session_file, {}, cache)
    cache.flush()

    # 重开进程 = 进程内缓存为空,只有磁盘缓存;命中就不该再打开文件读内容。
    reopened = ScanCache(tmp_path / "scan-cache.json")
    reopened.get_digest(session_file, session_file.stat())  # 先把缓存文件读进来

    def forbidden(*args, **kwargs):
        raise AssertionError("命中持久化摘要后仍读取了文件内容")

    monkeypatch.setattr("pathlib.Path.open", forbidden)
    assert _path_identity(session_file, {}, reopened) == identity


def test_digest_recomputed_after_file_changes(tmp_path, session_file):
    cache = ScanCache(tmp_path / "scan-cache.json")
    stale = _path_identity(session_file, {}, cache)
    cache.flush()

    session_file.write_text('{"role":"user"}\n{"role":"assistant"}\n')
    stat = session_file.stat()
    os.utime(session_file, ns=(stat.st_atime_ns, stat.st_mtime_ns + 1_000_000))

    reopened = ScanCache(tmp_path / "scan-cache.json")
    assert reopened.get_digest(session_file, session_file.stat()) is None
    fresh = _path_identity(session_file, {}, reopened)
    assert fresh[4] != stale[4]
    reopened.flush()

    again = ScanCache(tmp_path / "scan-cache.json")
    assert again.get_digest(session_file, session_file.stat()) == fresh[4]


def test_concurrent_digest_writes_all_survive(tmp_path):
    """全量扫描会由多个线程并发写摘要,不能互相丢条目。"""
    files = []
    for number in range(64):
        path = tmp_path / f"session-{number}.jsonl"
        path.write_text(f'{{"n":{number}}}\n')
        files.append(path)

    cache = ScanCache(tmp_path / "scan-cache.json")
    with ThreadPoolExecutor(max_workers=8) as pool:
        identities = list(pool.map(lambda p: _path_identity(p, {}, cache), files))
    cache.flush()

    reopened = ScanCache(tmp_path / "scan-cache.json")
    for path, identity in zip(files, identities):
        assert reopened.get_digest(path, path.stat()) == identity[4]


def test_legacy_cache_file_without_digests_key_still_loads(tmp_path, session_file):
    path = tmp_path / "scan-cache.json"
    path.write_text('{"/some/session.jsonl": {"version": 6, "meta": {}}}')

    cache = ScanCache(path)
    assert cache.get_digest(session_file, session_file.stat()) is None
    _path_identity(session_file, {}, cache)
    cache.flush()

    reopened = ScanCache(path)
    assert reopened.get_digest(session_file, session_file.stat()) is not None


def test_two_scans_flush_without_dropping_each_others_entries(tmp_path):
    """预热扫描与 scan RPC 各自 flush 时,后写的那份不能把先写的顶掉。"""
    path = tmp_path / "scan-cache.json"
    first_file = tmp_path / "a.jsonl"
    second_file = tmp_path / "b.jsonl"
    first_file.write_text("a\n")
    second_file.write_text("b\n")

    first = ScanCache(path)
    second = ScanCache(path)
    first.put(first_file, first_file.stat(), {"id": "a"})
    second.put(second_file, second_file.stat(), {"id": "b"})
    first.flush()
    second.flush()

    reopened = ScanCache(path)
    assert reopened.get(first_file, first_file.stat()) == {"id": "a"}
    assert reopened.get(second_file, second_file.stat()) == {"id": "b"}


def test_shared_cache_is_one_instance():
    assert shared_cache() is shared_cache()
