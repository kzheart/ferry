"""AssistantReply 到各家原生历史记录的编译器。"""
from __future__ import annotations

import copy
import json
import secrets
import time
import uuid
from abc import ABC, abstractmethod
from datetime import datetime, timezone

from ..domain.authoring import AssistantReply, TextItem, ToolItem
from ..domain.reasoning import visible_text


class AuthoringCompiler(ABC):
    name: str
    inplace = True
    save_as = True

    def capabilities(self) -> dict:
        modes = (["inplace"] if self.inplace else []) + (["saveas"] if self.save_as else [])
        return {"tool": self.name, "operation": "replace-assistant-reply",
                "item_kinds": ["text", "tool"], "ordered": True,
                "tool_fields": ["name", "input", "output"],
                "turn_selectors": ["ordinal", "locator"],
                "inplace": self.inplace, "save_as": self.save_as,
                "operation_modes": {"replace-assistant-reply": modes}}

    def supports_mode(self, save_as: bool) -> bool:
        return self.save_as if save_as else self.inplace

    @abstractmethod
    def replace(self, doc, turn: int, reply: AssistantReply) -> list[str]: ...


def _positive_turn(turn) -> int:
    if isinstance(turn, bool):
        raise ValueError("turn 必须是正整数")
    try:
        value = int(turn)
    except (TypeError, ValueError) as error:
        raise ValueError("turn 必须是正整数") from error
    if value < 1 or value != turn:
        raise ValueError("turn 必须是正整数")
    return value


def _select_turn(candidates: list[tuple[int, str]], selector) -> tuple[int, int]:
    if isinstance(selector, str):
        for ordinal, (index, locator) in enumerate(candidates, 1):
            if selector == locator:
                return ordinal, index
        raise ValueError("turn locator 已失效，请刷新会话")
    ordinal = _positive_turn(selector)
    if ordinal > len(candidates):
        raise ValueError(f"轮次超界: 共 {len(candidates)} 轮")
    return ordinal, candidates[ordinal - 1][0]


def _reject_authored_spawn(reply: AssistantReply) -> None:
    names = {"agent", "spawn_agent", "task"}
    if any(isinstance(item, ToolItem) and item.name.lower() in names
           for item in reply.items):
        raise ValueError("子 Agent spawn/task 会改变会话树，authoring 已拒绝")


def _is_spawn_name(name) -> bool:
    return isinstance(name, str) and name.lower() in {"agent", "spawn_agent", "task"}


def _replace_at_first(records, is_reply, compiled):
    result = []
    inserted = False
    for record in records:
        if is_reply(record):
            if not inserted:
                result.extend(compiled)
                inserted = True
        else:
            result.append(record)
    if not inserted:
        result.extend(compiled)
    return result


class ClaudeAuthoringCompiler(AuthoringCompiler):
    name = "claude"

    @staticmethod
    def _visible_user_content(content):
        if isinstance(content, str):
            return True
        return isinstance(content, list) and any(
            isinstance(item, dict) and (
                item.get("type") in {"text", "tool_use"} or
                item.get("type") == "thinking" and
                visible_text(item.get("thinking")) is not None)
            for item in content)

    @staticmethod
    def _starts(records):
        starts = []
        for index, record in enumerate(records):
            if record.get("type") != "user" or record.get("isSidechain"):
                continue
            content = (record.get("message") or {}).get("content")
            if not (isinstance(content, list) and any(
                    item.get("type") == "tool_result" for item in content
                    if isinstance(item, dict))) and \
                    ClaudeAuthoringCompiler._visible_user_content(content):
                starts.append(index)
        return starts

    @staticmethod
    def _record(template, record_type, parent, content, stop_reason=None):
        record = {key: copy.deepcopy(value) for key, value in template.items()
                  if key not in {"uuid", "parentUuid", "promptId", "type", "message",
                                 "toolUseResult", "timestamp"}}
        record.update({"uuid": str(uuid.uuid4()), "parentUuid": parent,
                       "type": record_type, "isSidechain": False})
        message = {"role": "assistant" if record_type == "assistant" else "user",
                   "content": content}
        if record_type == "assistant":
            message.update(type="message", stop_reason=stop_reason or "end_turn")
        record["message"] = message
        record["timestamp"] = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
        return record

    @staticmethod
    def _is_reply_record(record):
        if record.get("isSidechain"):
            return False
        if record.get("type") == "assistant":
            return True
        content = (record.get("message") or {}).get("content")
        return record.get("type") == "user" and isinstance(content, list) and any(
            item.get("type") == "tool_result" for item in content
            if isinstance(item, dict))

    def replace(self, doc, turn, reply):
        starts = self._starts(doc.data)
        candidates = [(index, str(doc.data[index].get("uuid") or f"record:{index}"))
                      for index in starts]
        ordinal, user_index = _select_turn(candidates, turn)
        hi = starts[ordinal] if ordinal < len(starts) else len(doc.data)
        old = doc.data[user_index + 1:hi]
        _reject_authored_spawn(reply)
        if any(_is_spawn_name(item.get("name")) for record in old
               if self._is_reply_record(record)
               for item in ((record.get("message") or {}).get("content") or [])
               if isinstance(item, dict) and item.get("type") == "tool_use"):
            raise ValueError("目标回复包含子 Agent spawn/task，authoring 已拒绝")
        removed_ids = {record.get("uuid") for record in old
                       if self._is_reply_record(record) and record.get("uuid")}
        user = doc.data[user_index]
        template = next((record for record in old
                         if record.get("type") == "assistant" and
                         not record.get("isSidechain")), user)
        parent = user.get("uuid")
        compiled = []
        content = []
        for item in reply.items:
            if isinstance(item, TextItem):
                content.append({"type": "text", "text": item.text})
            else:
                call_id = "toolu_" + secrets.token_urlsafe(18)[:24]
                content.append({
                    "type": "tool_use", "id": call_id, "name": item.name,
                    "input": copy.deepcopy(item.input),
                })
                call = self._record(template, "assistant", parent, content, "tool_use")
                result = self._record(template, "user", call["uuid"], [{
                    "type": "tool_result", "tool_use_id": call_id,
                    "content": item.output,
                }])
                compiled.extend((call, result))
                parent = result["uuid"]
                content = []
        if content:
            final = self._record(template, "assistant", parent, content)
            compiled.append(final)
            parent = final["uuid"]
        doc.data[user_index + 1:hi] = _replace_at_first(
            old, self._is_reply_record, compiled)
        for record in doc.data[user_index + 1:]:
            if record.get("parentUuid") in removed_ids:
                record["parentUuid"] = parent
        return [f"替换第 {ordinal} 轮 AI 回复，共 {len(reply.items)} 个 item"]


class CodexAuthoringCompiler(AuthoringCompiler):
    name = "codex"

    @staticmethod
    def _user_starts(records):
        starts = []
        skipped = ("<environment_context>", "<user_instructions>",
                   "<ENVIRONMENT_CONTEXT>", "<turn_aborted>")
        for index, record in enumerate(records):
            payload = record.get("payload") or {}
            if record.get("type") != "response_item" or payload.get("type") != "message" \
                    or payload.get("role") != "user":
                continue
            text = "\n".join(str(block.get("text", "")) for block in
                             payload.get("content", []) if isinstance(block, dict))
            if text.strip() and not text.strip().startswith(skipped):
                starts.append(index)
        return starts

    @staticmethod
    def _is_reply_record(record):
        if record.get("type") != "response_item":
            return False
        payload = record.get("payload") or {}
        subtype = payload.get("type")
        return ((subtype == "message" and payload.get("role") == "assistant") or
                subtype in {"custom_tool_call", "function_call",
                            "custom_tool_call_output", "function_call_output"})

    @staticmethod
    def _is_spawn(record):
        payload = record.get("payload") or {}
        return payload.get("type") in {"custom_tool_call", "function_call"} and \
            _is_spawn_name(payload.get("name"))

    def replace(self, doc, turn, reply):
        starts = self._user_starts(doc.data)
        candidates = [(index, f"record:{index}") for index in starts]
        ordinal, user_index = _select_turn(candidates, turn)
        lo = user_index + 1
        hi = starts[ordinal] if ordinal < len(starts) else len(doc.data)
        old = doc.data[lo:hi]
        _reject_authored_spawn(reply)
        if any(self._is_spawn(record) for record in old):
            raise ValueError("目标回复包含子 Agent spawn/task，authoring 已拒绝")
        now = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
        compiled = []
        for item in reply.items:
            if isinstance(item, TextItem):
                compiled.append({"timestamp": now, "type": "response_item", "payload": {
                    "type": "message", "id": "msg_" + secrets.token_hex(12),
                    "role": "assistant", "content": [{"type": "output_text", "text": item.text}],
                    "phase": "final_answer"}})
            else:
                call_id = "call_" + secrets.token_urlsafe(18)[:24]
                arguments = (json.dumps(item.input, ensure_ascii=False)
                             if isinstance(item.input, dict) else item.input)
                compiled.extend((
                    {"timestamp": now, "type": "response_item", "payload": {
                        "type": "function_call", "id": "fc_" + secrets.token_hex(12),
                        "name": item.name, "arguments": arguments, "call_id": call_id,
                        "status": "completed"}},
                    {"timestamp": now, "type": "response_item", "payload": {
                        "type": "function_call_output", "id": "fco_" + secrets.token_hex(12),
                        "call_id": call_id, "output": item.output}},
                ))
        doc.data[lo:hi] = _replace_at_first(old, self._is_reply_record, compiled)
        return [f"替换第 {ordinal} 轮 AI 回复，共 {len(reply.items)} 个 item"]


class OpenCodeAuthoringCompiler(AuthoringCompiler):
    name = "opencode"
    inplace = False

    @staticmethod
    def _id(prefix):
        return f"{prefix}_{secrets.token_hex(6)}{secrets.token_urlsafe(12)[:14]}"

    def replace(self, doc, turn, reply):
        messages = doc.data.get("messages", [])
        users = [index for index, message in enumerate(messages)
                 if (message.get("info") or {}).get("role") == "user" and any(
                     part.get("type") in {"text", "tool"} or
                     (part.get("type") == "reasoning" and
                      visible_text(part.get("text")) is not None)
                     for part in message.get("parts", []))]
        candidates = [(index, str(messages[index].get("info", {}).get("id") or
                                  f"message:{index}")) for index in users]
        ordinal, user_index = _select_turn(candidates, turn)
        hi = users[ordinal] if ordinal < len(users) else len(messages)
        sid = (doc.data.get("info") or {}).get("id")
        user_id = messages[user_index]["info"].get("id")
        old = messages[user_index + 1:hi]
        _reject_authored_spawn(reply)
        if any(part.get("type") == "tool" and part.get("tool") == "task"
               for message in old for part in message.get("parts", [])):
            raise ValueError("目标回复包含子 Agent spawn/task，authoring 已拒绝")
        old_assistant = next((message for message in messages[user_index + 1:hi]
                              if (message.get("info") or {}).get("role") == "assistant"), None)
        info = copy.deepcopy((old_assistant or {}).get("info") or {})
        mid = self._id("msg")
        info.update({"id": mid, "sessionID": sid, "parentID": user_id,
                     "role": "assistant", "finish": "stop"})
        now = int(time.time() * 1000)
        if "time" in info:
            info["time"] = {"created": now, "completed": now}
        parts = []
        for item in reply.items:
            common = {"id": self._id("prt"), "messageID": mid, "sessionID": sid}
            if isinstance(item, TextItem):
                parts.append({**common, "type": "text", "text": item.text})
            else:
                info["finish"] = "tool-calls" if item is reply.items[-1] else info["finish"]
                parts.append({**common, "type": "tool", "tool": item.name,
                              "callID": "call_" + secrets.token_urlsafe(18)[:24],
                              "state": {"status": "completed",
                                        "input": copy.deepcopy(item.input),
                                        "output": item.output, "metadata": {},
                                        "time": {"start": now, "end": now}}})
        is_reply = lambda message: (message.get("info") or {}).get("role") == "assistant"
        messages[user_index + 1:hi] = _replace_at_first(
            old, is_reply, [{"info": info, "parts": parts}])
        return [f"替换第 {ordinal} 轮 AI 回复，共 {len(reply.items)} 个 item"]
