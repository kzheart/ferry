"""Claude 会话验收探针：真实探测与编辑后的影子副本探测。"""
from __future__ import annotations

import shutil
import uuid
from pathlib import Path

from ...infrastructure import probes
from . import editing as claude_edit


class ClaudeVerifier:
    def probe(self, session_id, cwd, model=None):
        return probes.probe_claude(session_id, cwd, model)

    def probe_edited(self, editor, _doc, result, model=None):
        path = Path(result["saved_as"])
        records = claude_edit.load(path)
        cwd = next((row.get("cwd") for row in records if row.get("cwd")), ".")
        shadow_id = str(uuid.uuid4())
        for row in records:
            if "sessionId" in row:
                row["sessionId"] = shadow_id
        shadow = path.with_name(f"{shadow_id}.jsonl")
        claude_edit.save(shadow, records)
        sidecar = path.with_suffix("")
        shadow_sidecar = shadow.with_suffix("")
        if sidecar.is_dir():
            shutil.copytree(sidecar, shadow_sidecar, dirs_exist_ok=True)
        try:
            rep = probes.probe_claude(shadow_id, cwd, model)
            rep["isolation"] = {"kind": "shadow_session", "id": shadow_id,
                                "cleaned": True}
            return rep
        finally:
            shadow.unlink(missing_ok=True)
            shutil.rmtree(shadow_sidecar, ignore_errors=True)
