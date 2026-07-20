"""声明式回复 authoring 的应用编排边界。"""
from __future__ import annotations

from ..domain.authoring import AssistantReply
from .editing import apply_mutation, preview_mutation


def capabilities(compiler) -> dict:
    return compiler.capabilities()


def preview(editor, compiler, ref: str, turn: int | str, reply_value: dict) -> dict:
    reply = AssistantReply.from_dict(reply_value)
    result = preview_mutation(
        editor, ref, lambda doc: compiler.replace(doc, turn, reply))
    result.update(turn=turn, reply=reply.to_dict(),
                  capabilities=compiler.capabilities())
    return result


def apply(editor, compiler, ref: str, turn: int | str, reply_value: dict,
          save_as: bool, revision: str | None = None):
    if not compiler.supports_mode(save_as):
        mode = "另存为" if save_as else "原地编辑"
        raise ValueError(f"{compiler.name} authoring 不支持{mode}")
    reply = AssistantReply.from_dict(reply_value)
    return apply_mutation(
        editor, ref, lambda doc: compiler.replace(doc, turn, reply),
        save_as, expected_revision=revision)
