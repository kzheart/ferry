"""此文件由 scripts/generate-contracts.py 生成，请勿手改。"""
from __future__ import annotations

AGENT_CAPABILITIES = ('browse', 'resume', 'migration-source', 'migration-target', 'edit', 'delete', 'probe', 'models')

AGENTS = {
    'claude': {
        'display_name': 'Claude Code',
        'icon': 'claude',
        'source_path': '~/.claude/projects',
        'capabilities': ('browse', 'resume', 'migration-source', 'migration-target', 'edit', 'delete', 'probe', 'models'),
        'edit_operations': ('delete-turn', 'rewrite', 'replace-assistant-reply'),
        'executables': ('claude',),
        'fallback_bin_dirs': (),
    },
    'codex': {
        'display_name': 'Codex CLI',
        'icon': 'codex',
        'source_path': '~/.codex/sessions',
        'capabilities': ('browse', 'resume', 'migration-source', 'migration-target', 'edit', 'delete', 'probe', 'models'),
        'edit_operations': ('delete-turn', 'rewrite', 'replace-assistant-reply'),
        'executables': ('codex',),
        'fallback_bin_dirs': (),
    },
    'opencode': {
        'display_name': 'OpenCode',
        'icon': 'opencode',
        'source_path': '~/.local/share/opencode',
        'capabilities': ('browse', 'resume', 'migration-source', 'migration-target', 'edit', 'delete', 'probe', 'models'),
        'edit_operations': ('rewrite',),
        'executables': ('opencode',),
        'fallback_bin_dirs': ('~/.opencode/bin',),
    },
    'pi': {
        'display_name': 'Pi Agent',
        'icon': 'pi',
        'source_path': '~/.pi/agent/sessions',
        'capabilities': ('browse', 'resume', 'migration-source', 'migration-target', 'edit', 'delete', 'probe', 'models'),
        'edit_operations': ('delete-turn', 'rewrite', 'replace-assistant-reply'),
        'executables': ('pi',),
        'fallback_bin_dirs': (),
    },
}
AGENT_IDS = tuple(AGENTS)
