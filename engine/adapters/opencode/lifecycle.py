"""OpenCode 会话生命周期：数据库型删除（快照后经 CLI 清理，不可撤销）。"""
from __future__ import annotations

import subprocess

from ..base.lifecycle import BaseLifecycle
from . import session as opencode_session


class OpenCodeLifecycle(BaseLifecycle):
    tool = "opencode"

    def resume_args(self, session_id):
        return ["-s", session_id]

    def handoff_args(self):
        return ["run"]

    def cleanup(self, session_id, _dest):
        try:
            tree = opencode_session.read(session_id)
            ids = [node.source_id for node in reversed(list(tree.walk()))]
        except Exception:
            ids = [session_id]
        for sid in ids:
            subprocess.run(["opencode", "session", "delete", sid],
                           capture_output=True, text=True, timeout=30)

    def validation_ref(self, session_id, _dest) -> str:
        return session_id

    def delete(self, plugin, ref: str) -> dict:
        editor = plugin.require("editor")
        doc = editor.load(ref)
        snap = editor.snapshot(doc, reason_code="snapshot.before_delete")
        self.cleanup(ref, None)
        return {"ok": True, "snapshot": str(snap), "undoable": False}
