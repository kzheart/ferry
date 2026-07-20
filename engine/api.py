#!/usr/bin/env python3
"""稳定兼容门面；实现位于分层后的 application/interfaces 包。"""

from .application.services import (
    edit_apply, edit_capabilities, edit_preview, env, handoff, health, history,
    list_models, migrate, resume_command, scan, show, snapshot_delete,
    snapshot_restore, snapshots, version,
)
from .interfaces.cli import main
from .interfaces.rpc import RPC_METHODS, rpc

__all__ = [
    "RPC_METHODS", "edit_apply", "edit_capabilities", "edit_preview", "env",
    "handoff", "health", "history", "list_models", "main", "migrate", "rpc",
    "scan", "show", "snapshot_delete", "snapshot_restore", "snapshots",
    "version",
]

if __name__ == "__main__":
    main()
