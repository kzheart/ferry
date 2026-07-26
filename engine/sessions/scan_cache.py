"""基于文件修订信息的扫描缓存。"""

import json
import os
import threading
from pathlib import Path


_DIGESTS_KEY = "digests"


class ScanCache:
    def __init__(self, path=None, version=6):
        self.path = path or Path.home() / ".resume-harness" / "scan-cache.json"
        self.version = version
        self._data = None
        self._lock = threading.Lock()

    def _load(self):
        if self._data is None:
            try:
                self._data = json.loads(self.path.read_text())
            except (OSError, json.JSONDecodeError):
                self._data = {}

    def get(self, path, stat):
        self._load()
        hit = self._data.get(str(path))
        if (hit and hit.get("version") == self.version
                and hit.get("mtime") == stat.st_mtime_ns
                and hit.get("size") == stat.st_size):
            return hit.get("meta")
        return None

    def put(self, path, stat, meta):
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
        with self._lock:
            payload = json.dumps(self._data)
        temp.write_text(payload)
        os.replace(temp, self.path)
