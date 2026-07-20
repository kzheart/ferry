"""Codex 会话验收探针：真实探测与临时 CODEX_HOME 完整树探测。"""
from __future__ import annotations

import os
import shutil
import tempfile
from pathlib import Path

from ...infrastructure import probes


class CodexVerifier:
    def probe(self, session_id, cwd, model=None):
        return probes.probe_codex(session_id, cwd, model)

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
            rep = probes.probe_codex_in_env(result["session_id"], env=env,
                                            model=model)
            rep["isolation"] = {"kind": "temp_home",
                                "id": result["session_id"], "cleaned": True}
            return rep
