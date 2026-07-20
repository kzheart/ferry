"""公共 adapter 契约层：只含 Protocol/ABC 与格式无关工具。

本包不得 import 任何具体 Agent（claude/codex/opencode）实现。
"""

from .authoring import AuthoringCompiler
from .codec import NativeEditCodec, TurnIndex, TurnSpan
from .editing import EditBackend, EditDocument
from .plugin import (
    ModelCatalog, MigrationSource, MigrationTarget, ReplyAuthor,
    SessionBrowser, SessionEditor, SessionLifecycle, SessionVerifier,
    ToolManifest, ToolPlugin,
)

__all__ = [
    "AuthoringCompiler", "EditBackend", "EditDocument",
    "NativeEditCodec", "TurnIndex", "TurnSpan",
    "ModelCatalog", "MigrationSource", "MigrationTarget", "ReplyAuthor",
    "SessionBrowser", "SessionEditor", "SessionLifecycle", "SessionVerifier",
    "ToolManifest", "ToolPlugin",
]
