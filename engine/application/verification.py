"""探针应用门面；具体 CLI 执行由 infrastructure 持有。"""

from ..adapters.registry import adapter
from ..infrastructure.probes import (
    PROBE_PROMPT, ProbeTimeout, probe_claude, probe_codex,
    probe_codex_in_env, probe_opencode,
)


def run_probe(tool, session_id, dirpath=None, model=None):
    return adapter(tool).verifier(session_id, dirpath, model)
