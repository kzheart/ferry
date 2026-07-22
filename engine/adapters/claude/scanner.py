"""Claude 文件存储扫描。"""

import hashlib
import json
import os
from pathlib import Path

from ...domain.usage import add_tokens, dominant_model, empty_tokens, has_tokens, iso_ms
from ..base.scanner import clip_text, scan_jsonl


def _usage_tokens(usage) -> dict:
    return {"input": usage.get("input_tokens") or 0,
            "output": usage.get("output_tokens") or 0,
            "cache_read": usage.get("cache_read_input_tokens") or 0,
            "cache_write": usage.get("cache_creation_input_tokens") or 0}


def _meta(path: Path, stat, base: Path) -> dict:
    cwd, title, count = "", "", 0
    by_model, created = {}, None
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        kind = record.get("type")
        if kind in ("user", "assistant"):
            count += 1
            cwd = cwd or record.get("cwd", "")
            ts = iso_ms(record.get("timestamp"))
            if ts and (created is None or ts < created):
                created = ts
            message = record.get("message") or {}
            model = message.get("model")
            if kind == "assistant" and model and model != "<synthetic>":
                add_tokens(by_model.setdefault(model, empty_tokens()),
                           _usage_tokens(message.get("usage") or {}))
            content = message.get("content")
            if (not title and kind == "user" and isinstance(content, str)
                    and content.strip() and not content.strip().startswith("<")):
                title = clip_text(content)
        elif kind == "ai-title":
            title = record.get("title", "") or title
    try:
        relative = path.relative_to(base)
    except ValueError:
        relative = path
    child = len(relative.parts) > 2
    root_id = relative.parts[1] if child else path.stem
    tokens = empty_tokens()
    for model_tokens in by_model.values():
        add_tokens(tokens, model_tokens)
    return {} if not count else {
        "tool": "claude", "id": path.stem, "title": title, "dir": cwd,
        "updated": int(stat.st_mtime * 1000), "created": created,
        "count": count, "size": stat.st_size, "path": str(path),
        "tokens": tokens if has_tokens(tokens) else None,
        "model": dominant_model(by_model), "parent_id": root_id if child else None,
        "root_id": root_id,
    }


def scan(cache):
    base = Path(os.path.expanduser("~/.claude/projects"))
    return scan_jsonl(str(base / "**/*.jsonl"), cache,
                      lambda path, stat: _meta(path, stat, base))


def fingerprint(ref: str) -> str:
    """计算 Claude 根会话及其 subagents/journals 的只读树指纹。"""
    path = Path(ref).resolve(strict=True)
    root = Path(os.path.expanduser("~/.claude/projects")).resolve(strict=True)
    candidates = [path]
    child_root = path.with_suffix("") / "subagents"
    if child_root.exists():
        candidates.extend(sorted(child_root.rglob("*.jsonl")))
    digest = hashlib.sha256()
    for candidate in candidates:
        resolved = candidate.resolve(strict=True)
        if not resolved.is_file() or not resolved.is_relative_to(root):
            raise ValueError("Claude 会话树超出存储根目录")
        digest.update(str(resolved.relative_to(root)).encode())
        digest.update(b"\0")
        digest.update(resolved.read_bytes())
        digest.update(b"\0")
    return "sha256:" + digest.hexdigest()


def agent_fingerprint(ref: str) -> str:
    """用主会话与直属 subagent 的元数据标记 Agent 读取引用。"""
    path = Path(ref).resolve(strict=True)
    root = Path(os.path.expanduser("~/.claude/projects")).resolve(strict=True)
    candidates = [path]
    child_root = path.with_suffix("") / "subagents"
    if child_root.exists():
        candidates.extend(sorted(child_root.rglob("*.jsonl")))
    digest = hashlib.sha256()
    for candidate in candidates:
        resolved = candidate.resolve(strict=True)
        if not resolved.is_file() or not resolved.is_relative_to(root):
            raise ValueError("Claude 会话树超出存储根目录")
        stat = resolved.stat()
        digest.update(str(resolved.relative_to(root)).encode())
        digest.update(f"\0{stat.st_dev}:{stat.st_ino}:{stat.st_mtime_ns}:{stat.st_size}\0".encode())
    return "stat:" + digest.hexdigest()
