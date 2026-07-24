"""Claude 会话生命周期：清理、删除与 sidecar 归档策略。"""
from __future__ import annotations

import glob
import os
import shutil
from pathlib import Path

from ..shared.lifecycle import FileSessionLifecycle


class ClaudeLifecycle(FileSessionLifecycle):
    tool = "claude"

    def resume_args(self, session_id):
        return ["--resume", session_id]

    def cleanup(self, session_id, _dest):
        for hit in glob.glob(os.path.expanduser(
                f"~/.claude/projects/*/{session_id}.jsonl")):
            Path(hit).unlink(missing_ok=True)
            shutil.rmtree(Path(hit).with_suffix(""), ignore_errors=True)

    def _archive_sidecar(self, path: Path, snap: Path) -> None:
        sidecar = path.with_suffix("")
        if sidecar.is_dir():
            shutil.move(str(sidecar), str(snap.with_suffix("")))
