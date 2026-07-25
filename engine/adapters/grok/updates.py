"""Aggregate current Grok ACP session/update envelopes."""
from __future__ import annotations

import copy


def _text(content):
    if isinstance(content, str):
        return content
    if isinstance(content, dict):
        return str(content.get("text") or "")
    if isinstance(content, list):
        return "".join(_text(item) for item in content)
    return ""


def aggregate_updates(envelopes):
    prompts, order = {}, []
    for envelope in envelopes:
        params = envelope.get("params") or {}
        update, meta = params.get("update") or {}, params.get("_meta") or {}
        nested_meta = update.get("_meta") or {}
        prompt_index = meta.get("promptIndex", nested_meta.get("promptIndex"))
        prompt_id = str(meta.get("promptId") or update.get("prompt_id") or
                        f"prompt:{prompt_index if prompt_index is not None else len(order)}")
        if prompt_id not in prompts:
            prompts[prompt_id] = {
                "id": prompt_id, "index": prompt_index,
                "user": [], "assistant": [], "tools": {}, "unknown": [],
            }
            order.append(prompt_id)
        prompt = prompts[prompt_id]
        update_type = meta.get("updateType")
        kind = update.get("kind")
        session_update = update.get("sessionUpdate")
        if update_type in {"UserMessage", "Prompt"} or session_update == \
                "user_message_chunk" or kind in {
            "user_message", "prompt",
        }:
            prompt["user"].append(_text(update.get("content")))
        elif update_type == "AgentMessageChunk" or session_update == \
                "agent_message_chunk":
            prompt["assistant"].append({
                "kind": "text", "text": _text(update.get("content")),
            })
        elif update_type == "AgentThoughtChunk" or session_update == \
                "agent_thought_chunk":
            prompt["assistant"].append({
                "kind": "thinking", "text": _text(update.get("content")),
            })
        elif update_type in {"ToolCall", "ToolCallUpdate"} or session_update in {
            "tool_call", "tool_call_update",
        }:
            update_meta = meta.get("updateParams") or {}
            call_id = str(
                update_meta.get("toolCallId") or update.get("toolCallId")
                or meta.get("toolCallId") or ""
            )
            if not call_id:
                prompt["unknown"].append(copy.deepcopy(envelope))
                continue
            tool = prompt["tools"].setdefault(call_id, {
                "id": call_id, "name": update_meta.get("kind") or kind or "tool",
                "input": {}, "output": None, "status": "unknown",
            })
            if update.get("rawInput") is not None:
                tool["input"] = copy.deepcopy(update["rawInput"])
            if update.get("rawOutput") is not None:
                tool["output"] = copy.deepcopy(update["rawOutput"])
            if update_meta.get("status"):
                tool["status"] = str(update_meta["status"]).lower()
        elif kind == "compaction" or update_type == "Compaction":
            prompt["compaction"] = copy.deepcopy(update)
        else:
            prompt["unknown"].append(copy.deepcopy(envelope))
    return [prompts[key] for key in order]
