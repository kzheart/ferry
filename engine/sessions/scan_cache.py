"""基于文件修订信息的扫描缓存。"""

import json
import os
import threading
from pathlib import Path


_DIGESTS_KEY = "digests"


def _newer(candidate, current) -> bool:
    """条目合并规则:同一个 key 取 mtime 较新的那份。"""
    if not isinstance(current, dict):
        return True
    return candidate.get("mtime", -1) >= current.get("mtime", -1)


def _merge(base: dict, incoming: dict) -> dict:
    merged = {
        key: value for key, value in base.items() if key != _DIGESTS_KEY
    }
    for key, value in incoming.items():
        if key == _DIGESTS_KEY:
            continue
        if _newer(value, merged.get(key)):
            merged[key] = value
    digests = dict(base.get(_DIGESTS_KEY) or {})
    for key, value in (incoming.get(_DIGESTS_KEY) or {}).items():
        if _newer(value, digests.get(key)):
            digests[key] = value
    if digests:
        merged[_DIGESTS_KEY] = digests
    return merged


class ScanCache:
    def __init__(self, path=None, version=6):
        self.path = path or Path.home() / ".resume-harness" / "scan-cache.json"
        self.version = version
        self._data = None
        # put/put_digest 会在持锁状态下再调 _load,必须可重入。
        self._lock = threading.RLock()

    def _read_disk(self) -> dict:
        try:
            data = json.loads(self.path.read_text())
        except (OSError, json.JSONDecodeError):
            return {}
        return data if isinstance(data, dict) else {}

    def _load(self):
        with self._lock:
            if self._data is None:
                self._data = self._read_disk()

    def get(self, path, stat):
        self._load()
        hit = self._data.get(str(path))
        if (hit and hit.get("version") == self.version
                and hit.get("mtime") == stat.st_mtime_ns
                and hit.get("size") == stat.st_size):
            return hit.get("meta")
        return None

    def put(self, path, stat, meta):
        with self._lock:
            self._load()
            self._data[str(path)] = {"version": self.version,
                "mtime": stat.st_mtime_ns, "size": stat.st_size, "meta": meta}

    def get_digest(self, path, stat):
        """取文件内容摘要:stat 四元组完全一致才算命中。"""
        self._load()
        hit = self._data.get(_DIGESTS_KEY, {}).get(str(path))
        if (hit and hit.get("dev") == stat.st_dev
                and hit.get("ino") == stat.st_ino
                and hit.get("mtime") == stat.st_mtime_ns
                and hit.get("size") == stat.st_size):
            return hit.get("sha256")
        return None

    def put_digest(self, path, stat, sha256):
        # 全量扫描时会被多个规范化线程并发写入。
        with self._lock:
            self._load()
            digests = self._data.setdefault(_DIGESTS_KEY, {})
            digests[str(path)] = {
                "dev": stat.st_dev, "ino": stat.st_ino,
                "mtime": stat.st_mtime_ns, "size": stat.st_size,
                "sha256": sha256,
            }

    def flush(self):
        if self._data is None:
            return
        self.path.parent.mkdir(parents=True, exist_ok=True)
        # 进程内可能有并发扫描(预热线程 + RPC),临时文件必须按线程区分,
        # 否则两个线程互相 replace 掉对方的临时文件。
        temp = self.path.with_name(
            f"{self.path.name}.{os.getpid()}.{threading.get_ident()}.tmp",
        )
        # 直接整份覆盖会把别人刚写进去的条目丢掉(两次扫描先后 replace,后写的
        # 赢)。持锁做「读回磁盘最新 → 合并本实例增量 → 写回」。
        with self._lock:
            merged = _merge(self._read_disk(), self._data)
            self._data = merged
            temp.write_text(json.dumps(merged))
            os.replace(temp, self.path)


_shared: ScanCache | None = None
_shared_lock = threading.Lock()


def shared_cache() -> ScanCache:
    """进程级共享缓存:预热扫描与 scan RPC 复用同一份,不再互相覆盖。"""
    global _shared
    with _shared_lock:
        if _shared is None:
            _shared = ScanCache()
        return _shared
