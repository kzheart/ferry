"""OpenCode 的 AssistantReply 编译器：轮次与编解码全部来自 opencode.codec。

官方 API 缺少批量事务能力，authoring 只支持 save-as。
"""
from __future__ import annotations

from ..base.authoring import AuthoringCompiler
from ..base.codec import select_span
from .codec import CODEC, TURN_INDEX


class OpenCodeAuthoringCompiler(AuthoringCompiler):
    name = "opencode"
    inplace = False

    def replace(self, doc, turn, reply):
        span = select_span(TURN_INDEX.turns(doc.data), turn)
        return CODEC.replace_reply(doc, span, reply)
