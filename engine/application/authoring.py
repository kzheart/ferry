"""声明式回复 authoring 的应用编排边界。"""
from __future__ import annotations

from ..domain.authoring import AssistantReply
from ..domain.errors import OperationUnsupportedError
from .editing import apply_mutation, preview_mutation


def capabilities(compiler) -> dict:
    return compiler.capabilities()


def preview(editor, compiler, ref: str, turn: int | str, reply_value: dict,
            loader=None) -> dict:
    reply = AssistantReply.from_dict(reply_value)
    result = preview_mutation(
        editor, ref, lambda doc: compiler.replace(doc, turn, reply), loader=loader)
    result.update(turn=turn, reply=reply.to_dict(),
                  capabilities=compiler.capabilities())
    return result


def apply(editor, compiler, ref: str, turn: int | str, reply_value: dict,
          save_as: bool, revision: str | None = None):
    if not compiler.supports_mode(save_as):
        raise OperationUnsupportedError(
            compiler.name, "replace-assistant-reply",
            "saveas" if save_as else "inplace")
    reply = AssistantReply.from_dict(reply_value)
    return apply_mutation(
        editor, ref, lambda doc: compiler.replace(doc, turn, reply),
        save_as, expected_revision=revision)
