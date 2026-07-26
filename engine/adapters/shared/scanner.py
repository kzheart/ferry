"""Shared scanner mechanics for line-delimited session stores."""
from __future__ import annotations

import glob
import hashlib
import json
import os
from collections.abc import Callable, Iterator
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from ...sessions.scan_progress import TRACKER
from ...sessions.topology import session_roots

_PARALLEL_SCAN_THRESHOLD = 16
_SCAN_WORKERS = min(8, (os.cpu_count() or 4))


def clip_text(text: str, size: int = 80) -> str:
    text = " ".join(text.split())
    return text[:size] + ("…" if len(text) > size else "")


def stat_digest(label, stat) -> str:
    """把文件 stat 折成稳定的修订标记。"""
    marker = f"{label}:{stat.st_dev}:{stat.st_ino}:{stat.st_mtime_ns}:{stat.st_size}"
    return "stat:" + hashlib.sha256(marker.encode()).hexdigest()


def path_stat_fingerprint(ref: str) -> str:
    """Agent 检索阶段的 O(1) 修订标记；深度校验留给写入链路。"""
    path = Path(ref).resolve(strict=True)
    return stat_digest(path, path.stat())


def iter_lines(path: Path) -> Iterator[str]:
    """逐行读 JSONL。

    `read_text().splitlines()` 会把整个文件读进内存再复制一份行列表,大会话的
    峰值内存是文件体积的两倍。行内容与原写法一致(行尾换行符统一削掉)。
    """
    with path.open(encoding="utf-8") as stream:
        for line in stream:
            yield line.rstrip("\n")


def _scan_one(filename: str, cache, parse) -> dict | None:
    TRACKER.advance()
    path = Path(filename)
    try:
        stat = path.stat()
    except OSError:
        return None
    cached = cache.get(path, stat)
    if cached is not None:
        return cached or None
    try:
        meta = parse(path, stat)
    except (json.JSONDecodeError, OSError):
        return None
    cache.put(path, stat, meta)
    return meta or None


def scan_jsonl(pattern: str, cache, parse: Callable[[Path, object], dict]) -> list[dict]:
    """Scan cached JSONL files; adapters only implement their record schema."""
    filenames = glob.glob(pattern, recursive=True)
    # 进度上报只在 RPC scan 期间生效,其他入口(如内容索引预热)是空操作
    TRACKER.set_total(len(filenames))
    # JSONL 解析是冷启动最耗时的一段,文件之间互不依赖,且解析中的读文件与
    # json.loads 都会释放 GIL。结果用 map 按输入顺序回收,输出与串行时一致。
    if len(filenames) < _PARALLEL_SCAN_THRESHOLD:
        parsed = [_scan_one(name, cache, parse) for name in filenames]
    else:
        with ThreadPoolExecutor(max_workers=_SCAN_WORKERS) as pool:
            parsed = list(
                pool.map(lambda name: _scan_one(name, cache, parse), filenames),
            )
    return session_roots([meta for meta in parsed if meta])
