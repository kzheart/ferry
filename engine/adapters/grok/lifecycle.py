"""Grok resume-only lifecycle."""

from ..shared.lifecycle import BaseLifecycle


class GrokLifecycle(BaseLifecycle):
    tool = "grok"

    def resume_args(self, session_id):
        return ["--resume", session_id]

    def cleanup(self, _session_id, _dest):
        return None
