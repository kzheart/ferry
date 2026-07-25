"""Pi file session lifecycle."""
from pathlib import Path

from ..shared.lifecycle import FileSessionLifecycle


class PiLifecycle(FileSessionLifecycle):
    tool = "pi"

    def resume_args(self, session_id):
        from .adapter import resolve

        try:
            target = str(resolve(session_id))
        except Exception:
            target = session_id
        return ["--session", target]

    def cleanup(self, _session_id, dest):
        Path(dest).unlink(missing_ok=True)
