"""此文件由 scripts/generate-contracts.py 生成，请勿手改。"""
from __future__ import annotations

AGENTS = {
    'claude': {
        'display_name': 'Claude Code',
        'icon': 'claude',
        'source_path': '~/.claude/projects',
        'edit_operations': ('delete-turn', 'rewrite', 'replace-assistant-reply'),
        'executables': ('claude',),
        'fallback_bin_dirs': (),
    },
    'codex': {
        'display_name': 'Codex CLI',
        'icon': 'codex',
        'source_path': '~/.codex/sessions',
        'edit_operations': ('delete-turn', 'rewrite', 'replace-assistant-reply'),
        'executables': ('codex',),
        'fallback_bin_dirs': (),
    },
    'opencode': {
        'display_name': 'OpenCode',
        'icon': 'opencode',
        'source_path': '~/.local/share/opencode',
        'edit_operations': ('rewrite',),
        'executables': ('opencode',),
        'fallback_bin_dirs': ('~/.opencode/bin',),
    },
}
AGENT_IDS = tuple(AGENTS)
