"""引擎内唯一的工具能力注册表。"""

import glob
import os
import sys

from .base import ToolAdapter
from .claude.editor import ClaudeBackend
from .claude.reader import read as read_claude
from .claude.writer import write as write_claude
from .codex.editor import CodexBackend
from .codex.reader import read as read_codex
from .codex.writer import write as write_codex
from .opencode.editor import OpenCodeBackend
from .opencode.reader import read as read_opencode
from .opencode.writer import write as write_opencode


def _service(name):
    def call(*args, **kwargs):
        from ..application import services
        return getattr(services, name)(*args, **kwargs)
    return call


def _probe(tool):
    def call(sid, cwd=None, model=None):
        from ..application.verification import run_probe
        return run_probe(tool, sid, cwd, model)
    return call


def _resolve_file(tool, pattern, ref):
    if os.path.exists(ref):
        return ref
    hits = glob.glob(os.path.expanduser(pattern.format(ref=ref)))
    if not hits:
        sys.exit(f"找不到 {tool} 会话: {ref}")
    return hits[0]


def _resume(tool):
    return lambda sid, cwd: {"tool": tool, "session_id": sid, "cwd": cwd}


_ADAPTERS = {
    "claude": ToolAdapter("claude", "~/.claude/projects", _service("_scan_claude"),
        read_claude, write_claude, ClaudeBackend(), _probe("claude"),
        _service("_discover_claude_models"),
        lambda ref: _resolve_file("claude", "~/.claude/projects/*/{ref}.jsonl", ref),
        _resume("claude")),
    "codex": ToolAdapter("codex", "~/.codex/sessions", _service("_scan_codex"),
        read_codex, write_codex, CodexBackend(), _probe("codex"),
        _service("_discover_codex_models"),
        lambda ref: _resolve_file("codex", "~/.codex/sessions/*/*/*/rollout-*-{ref}.jsonl", ref),
        _resume("codex")),
    "opencode": ToolAdapter("opencode", "~/.local/share/opencode", _service("_scan_opencode"),
        read_opencode, write_opencode, OpenCodeBackend(), _probe("opencode"),
        _service("_discover_opencode_models"), lambda ref: ref, _resume("opencode")),
}


def adapter(tool: str) -> ToolAdapter:
    try:
        return _ADAPTERS[tool]
    except KeyError as error:
        raise ValueError(f"未知工具: {tool}") from error


def adapters():
    return tuple(_ADAPTERS)
