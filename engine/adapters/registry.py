"""引擎内唯一的工具能力注册表。"""

import glob
import os
import sys

from .base import ToolAdapter
from .claude.editor import ClaudeBackend
from .claude.models import discover as claude_models, fallback as claude_fallback
from .claude.reader import read as read_claude
from .claude.scanner import scan as scan_claude
from .claude.writer import write as write_claude
from .codex.editor import CodexBackend
from .codex.models import discover as codex_models, fallback as codex_fallback
from .codex.reader import read as read_codex
from .codex.scanner import scan as scan_codex
from .codex.writer import write as write_codex
from .opencode.editor import OpenCodeBackend
from .opencode.models import discover as opencode_models, fallback as opencode_fallback
from .opencode.reader import read as read_opencode
from .opencode.scanner import scan as scan_opencode
from .opencode.writer import write as write_opencode
from ..infrastructure.probes import probe_claude, probe_codex, probe_opencode


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
    "claude": ToolAdapter("claude", "~/.claude/projects", scan_claude,
        read_claude, write_claude, ClaudeBackend(), probe_claude,
        claude_models, claude_fallback,
        lambda ref: _resolve_file("claude", "~/.claude/projects/*/{ref}.jsonl", ref),
        _resume("claude")),
    "codex": ToolAdapter("codex", "~/.codex/sessions", scan_codex,
        read_codex, write_codex, CodexBackend(), probe_codex,
        codex_models, codex_fallback,
        lambda ref: _resolve_file("codex", "~/.codex/sessions/*/*/*/rollout-*-{ref}.jsonl", ref),
        _resume("codex")),
    "opencode": ToolAdapter("opencode", "~/.local/share/opencode", scan_opencode,
        read_opencode, write_opencode, OpenCodeBackend(), probe_opencode,
        opencode_models, opencode_fallback, lambda ref: ref, _resume("opencode")),
}


def adapter(tool: str) -> ToolAdapter:
    try:
        return _ADAPTERS[tool]
    except KeyError as error:
        raise ValueError(f"未知工具: {tool}") from error


def adapters():
    return tuple(_ADAPTERS)
