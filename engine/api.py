#!/usr/bin/env python3
"""稳定兼容门面；实现位于分层后的 application/interfaces 包。"""

from .application.services import (
    authoring_apply, authoring_capabilities, authoring_preview,
    edit_apply, edit_capabilities, edit_preview, env, handoff, health, history,
    list_models, migrate, resume_command, scan, session_delete,
    session_meta_list, session_meta_set, session_undelete,
    show, version,
)
from .interfaces.cli import main
from .interfaces.rpc import RPC_METHODS, rpc

__all__ = [
    "RPC_METHODS", "authoring_apply", "authoring_capabilities",
    "authoring_preview", "edit_apply", "edit_capabilities", "edit_preview", "env",
    "handoff", "health", "history", "list_models", "main", "migrate", "rpc",
    "scan", "session_delete", "session_meta_list", "session_meta_set",
    "session_undelete", "show", "version",
]

if __name__ == "__main__":
    main()
