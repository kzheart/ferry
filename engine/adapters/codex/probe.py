"""Codex 会话验收探针：真实探测与临时 CODEX_HOME 完整树探测。"""
from __future__ import annotations

import os
import shutil
import tempfile
from pathlib import Path

from ...infrastructure import executables, probes


def _probe_in_env(session_id, model=None, env=None):
    command = executables.argv("codex", "exec", "resume", session_id,
                               "--skip-git-repo-check")
    if model:
        command += ["-m", model]
    result = probes.run(command + [probes.PROBE_PROMPT], env=env)
    if result.returncode != 0:
        return probes.report("failed", "probe.process_failed",
                             {"tool": "codex", "exit_code": result.returncode},
                             stdout=result.stdout, stderr=result.stderr)
    return probes.report("passed", stdout=result.stdout, stderr=result.stderr)


def _probe(session_id, _cwd, model=None):
    return _probe_in_env(session_id, model=model)


class CodexVerifier:
    def probe(self, session_id, cwd, model=None):
        return _probe(session_id, cwd, model)

    def probe_edited(self, _editor, _doc, result, model=None):
        with tempfile.TemporaryDirectory(prefix="ferry-codex-probe-") as tmp:
            codex_home = Path(tmp) / ".codex"
            sessions = codex_home / "sessions" / "probe" / "01" / "01"
            sessions.mkdir(parents=True)
            for raw in result.get("published_paths", [result["saved_as"]]):
                shutil.copy(raw, sessions / Path(raw).name)
            for name in ("auth.json", "config.toml"):
                source = Path.home() / ".codex" / name
                if source.exists():
                    shutil.copy(source, codex_home / name)
            env = dict(os.environ)
            env["CODEX_HOME"] = str(codex_home)
            rep = _probe_in_env(result["session_id"], env=env, model=model)
            rep["isolation"] = {"kind": "temp_home",
                                "id": result["session_id"], "cleaned": True}
            return rep
