"""Codex 的 AssistantReply 编译器：轮次与编解码全部来自 codex.codec。"""
from __future__ import annotations

from ..base.authoring import AuthoringCompiler
from ..base.codec import select_span
from .codec import CODEC, TURN_INDEX


class CodexAuthoringCompiler(AuthoringCompiler):
    name = "codex"

    def replace(self, doc, turn, reply):
        span = select_span(TURN_INDEX.turns(doc.data), turn)
        return CODEC.replace_reply(doc, span, reply)
