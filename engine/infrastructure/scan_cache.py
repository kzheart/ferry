"""基于文件修订信息的扫描缓存。"""

import json
import os
from pathlib import Path


class ScanCache:
    def __init__(self, path=None, version=5):
        self.path = path or Path.home() / ".resume-harness" / "scan-cache.json"
        self.version = version
        self._data = None

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

    def flush(self):
        if self._data is None:
            return
        self.path.parent.mkdir(parents=True, exist_ok=True)
        temp = self.path.with_name(f"{self.path.name}.{os.getpid()}.tmp")
        temp.write_text(json.dumps(self._data))
        os.replace(temp, self.path)
