"""OpenCode 会话验收探针：编辑后克隆影子副本探测并清理。"""
from __future__ import annotations

from ...infrastructure import probes


class OpenCodeVerifier:
    def probe(self, session_id, cwd, model=None):
        return probes.probe_opencode(session_id, cwd, model)

    def probe_edited(self, editor, doc, result, model=None):
        authored = editor.load(result["session_id"])
        shadow = editor.save_copy(authored)
        try:
            cwd = doc.data.get("info", {}).get("directory") or "."
            rep = probes.probe_opencode(shadow["session_id"], cwd, model)
            rep["isolation"] = {"kind": "shadow_session",
                                "id": shadow["session_id"], "cleaned": True}
            return rep
        finally:
            editor.discard(shadow)
