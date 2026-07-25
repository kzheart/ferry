"""Grok resume and permanent-delete lifecycle."""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import uuid
from pathlib import Path

from ...system import executables
from ..shared.lifecycle import BaseLifecycle
from .writer import delete_index_rows


class GrokLifecycle(BaseLifecycle):
    tool = "grok"

    def resume_args(self, session_id):
        return ["--resume", session_id]

    @staticmethod
    def _owned_bundles(session_id, dest):
        destination = Path(dest).resolve()
        sessions_root = destination.parents[1]
        owned = []
        for summary_path in sessions_root.rglob("summary.json"):
            try:
                summary = json.loads(summary_path.read_text())
            except (OSError, json.JSONDecodeError):
                continue
            info = summary.get("info") or {}
            if (
                info.get("id") == session_id
                or summary.get("root_session_id") == session_id
            ):
                owned.append((str(info.get("id") or ""), summary_path.parent))
        return sessions_root, owned

    def cleanup(self, session_id, dest):
        sessions_root, owned = self._owned_bundles(session_id, dest)
        ids = [sid for sid, _ in owned if sid]
        quarantined = []
        try:
            for _, path in owned:
                temporary = path.with_name(
                    f".{path.name}.ferry-cleanup.{uuid.uuid4().hex}.tmp"
                )
                os.replace(path, temporary)
                quarantined.append((path, temporary))
            if ids:
                delete_index_rows(ids, sessions_root)
        except Exception:
            for original, temporary in reversed(quarantined):
                if temporary.exists() and not original.exists():
                    os.replace(temporary, original)
            raise
        for _, temporary in quarantined:
            shutil.rmtree(temporary)

    def delete(self, adapter, ref: str) -> dict:
        path = Path(adapter.browser.resolve_ref(ref))
        summary = json.loads((path / "summary.json").read_text())
        session_id = str(summary["info"]["id"])
        cwd = str(summary["info"]["cwd"])
        command_cwd = cwd if Path(cwd).is_dir() else str(Path.home())
        result = subprocess.run(
            executables.argv("grok", "sessions", "delete", session_id),
            cwd=command_cwd, capture_output=True, text=True, timeout=30,
            **executables.RUN_FLAGS,
        )
        if result.returncode:
            raise RuntimeError(
                f"grok sessions delete 失败: "
                f"{result.stderr or result.stdout}".strip()
            )
        delete_index_rows((session_id,), path.parents[1])
        return {"ok": True, "undoable": False, "session_id": session_id}
