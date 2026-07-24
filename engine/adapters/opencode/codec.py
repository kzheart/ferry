"""OpenCode 导出负载的唯一轮次解析与编辑编解码。

轮次定义：含可见内容（text/tool/可见 reasoning）的用户消息到下一条之前。
"""
from __future__ import annotations

from ...events import event
from ...errors import LocatorStaleError, OperationUnsupportedError
from ...sessions.reasoning import visible_text
from ..shared.codec import TurnSpan


def _visible(message) -> bool:
    return any(part.get("type") in {"text", "tool"} or
               (part.get("type") == "reasoning" and
                visible_text(part.get("text")) is not None)
               for part in message.get("parts", []))


class OpenCodeTurnIndex:
    def visible_messages(self, payload) -> list[tuple[int, dict]]:
        return [(index, message) for index, message
                in enumerate(payload.get("messages", [])) if _visible(message)]

    def turns(self, payload) -> list[TurnSpan]:
        messages = payload.get("messages", [])
        users = [index for index, message in self.visible_messages(payload)
                 if (message.get("info") or {}).get("role") == "user"]
        spans = []
        for ordinal, start in enumerate(users, 1):
            end = users[ordinal] if ordinal < len(users) else len(messages)
            locator = str(messages[start].get("info", {}).get("id") or
                          f"message:{start}")
            spans.append(TurnSpan(ordinal, locator, start, end))
        return spans


class OpenCodeEditCodec:
    def delete_turn(self, doc, span: TurnSpan) -> list[str]:
        messages = doc.data.get("messages", [])
        user_id = messages[span.start]["info"].get("id")
        remove_ids = {user_id}
        remove_ids.update(m["info"].get("id") for m in messages
                          if _visible(m) and m["info"].get("parentID") == user_id)
        removed_children = {
            ((part.get("state") or {}).get("metadata") or {}).get("sessionId")
            for message in messages
            if message["info"].get("id") in remove_ids
            for part in message.get("parts", []) if part.get("tool") == "task"
        } - {None}
        doc.data["messages"] = [m for m in messages
                                if m["info"].get("id") not in remove_ids]
        tree = getattr(doc, "tree", None)
        if tree is not None and removed_children:
            tree.children = [child for child in tree.children
                             if child.source_id not in removed_children]
            tree.agent_edges = [edge for edge in tree.agent_edges
                                if edge.child_session_id not in removed_children]
        return [event("edit.turn_deleted", turn=span.ordinal)]

    def rewrite_message(self, doc, locator: str, text: str) -> list[str]:
        visible = [message for _, message in TURN_INDEX.visible_messages(doc.data)]
        message = next((m for m in visible
                        if m["info"].get("id") == locator), None)
        if message is None and str(locator).startswith("index:"):
            try:
                wanted = int(str(locator).removeprefix("index:"))
                message = visible[wanted]
            except (ValueError, IndexError):
                pass
        if message is None:
            raise LocatorStaleError("OpenCode 消息定位符已失效，请刷新会话",
                                    {"locator": locator})
        role = message["info"].get("role")
        if role not in {"user", "assistant"}:
            raise OperationUnsupportedError("opencode", "rewrite", str(role))
        text_parts = [p for p in message.get("parts", [])
                      if p.get("type") == "text"]
        if not text_parts:
            raise OperationUnsupportedError("opencode", "rewrite", "no-text")
        text_parts[0]["text"] = text
        for part in text_parts[1:]:
            part["text"] = ""
        return [event("edit.message_rewritten", count=1)]


TURN_INDEX = OpenCodeTurnIndex()
CODEC = OpenCodeEditCodec()
