"""Aggregate current Grok ACP session/update envelopes."""
from __future__ import annotations

import copy


_TERMINAL_TOOL_STATUSES = {"completed", "failed"}


def _text(content):
    if isinstance(content, str):
        return content
    if isinstance(content, dict):
        return str(content.get("text") or "")
    if isinstance(content, list):
        return "".join(_text(item) for item in content)
    return ""


def _parts(envelope):
    params = envelope.get("params") or {}
    update = params.get("update") or {}
    meta = params.get("_meta") or {}
    nested_meta = update.get("_meta") or {}
    return update, meta, nested_meta


def _prompt_identity(meta, update, nested_meta):
    prompt_id = meta.get("promptId") or update.get("prompt_id")
    prompt_index = meta.get("promptIndex", nested_meta.get("promptIndex"))
    return (str(prompt_id) if prompt_id else None), prompt_index


def _tool_event(meta, update):
    return (
        meta.get("updateType") in {"ToolCall", "ToolCallUpdate"}
        or update.get("sessionUpdate") in {"tool_call", "tool_call_update"}
    )


def _call_id(meta, update):
    update_meta = meta.get("updateParams") or {}
    return str(
        update_meta.get("toolCallId")
        or update.get("toolCallId")
        or meta.get("toolCallId")
        or ""
    )


def _tool_name(meta, update, nested_meta):
    update_meta = meta.get("updateParams") or {}
    tool_meta = nested_meta.get("x.ai/tool") or {}
    return str(
        tool_meta.get("name")
        or update.get("title")
        or update_meta.get("kind")
        or update.get("kind")
        or "tool"
    )


def aggregate_updates(envelopes):
    envelopes = list(envelopes)
    index_prompt_ids = {}
    for envelope in envelopes:
        update, meta, nested_meta = _parts(envelope)
        prompt_id, prompt_index = _prompt_identity(meta, update, nested_meta)
        if prompt_id is not None and prompt_index is not None:
            index_prompt_ids.setdefault(prompt_index, set()).add(prompt_id)

    def prompt_key(meta, update, nested_meta):
        prompt_id, prompt_index = _prompt_identity(meta, update, nested_meta)
        if prompt_id is not None:
            return prompt_id, prompt_index
        candidates = index_prompt_ids.get(prompt_index, set())
        if len(candidates) == 1:
            return next(iter(candidates)), prompt_index
        if prompt_index is not None:
            return f"prompt:{prompt_index}", prompt_index
        return None, None

    call_owners = {}
    for envelope in envelopes:
        update, meta, nested_meta = _parts(envelope)
        if not _tool_event(meta, update):
            continue
        call_id = _call_id(meta, update)
        key, _ = prompt_key(meta, update, nested_meta)
        if call_id and key is not None:
            call_owners.setdefault(call_id, set()).add(key)

    prompts, order = {}, []

    def ensure_prompt(prompt_id, prompt_index):
        if prompt_id not in prompts:
            prompts[prompt_id] = {
                "id": prompt_id, "index": prompt_index,
                "user": [], "blocks": [], "tools": {}, "unknown": [],
            }
            order.append(prompt_id)
        return prompts[prompt_id]

    for envelope in envelopes:
        update, meta, nested_meta = _parts(envelope)
        update_type = meta.get("updateType")
        kind = update.get("kind")
        session_update = update.get("sessionUpdate")
        is_tool = _tool_event(meta, update)
        prompt_id, prompt_index = prompt_key(meta, update, nested_meta)
        call_id = _call_id(meta, update) if is_tool else ""
        if is_tool and prompt_id is None:
            owners = call_owners.get(call_id, set())
            if len(owners) == 1:
                prompt_id = next(iter(owners))
        if is_tool and (prompt_id is None or len(call_owners.get(call_id, set())) > 1):
            prompt = ensure_prompt("prompt:unassigned", None)
            prompt["unknown"].append(copy.deepcopy(envelope))
            continue
        if prompt_id is None:
            prompt_id = f"prompt:{len(order)}"
        prompt = ensure_prompt(prompt_id, prompt_index)

        if update_type in {"UserMessage", "Prompt"} or session_update == \
                "user_message_chunk" or kind in {
            "user_message", "prompt",
        }:
            prompt["user"].append(_text(update.get("content")))
        elif update_type == "AgentMessageChunk" or session_update == \
                "agent_message_chunk":
            text = _text(update.get("content"))
            if prompt["blocks"] and prompt["blocks"][-1]["kind"] == "text":
                prompt["blocks"][-1]["text"] += text
            else:
                prompt["blocks"].append({"kind": "text", "text": text})
        elif update_type == "AgentThoughtChunk" or session_update == \
                "agent_thought_chunk":
            text = _text(update.get("content"))
            if prompt["blocks"] and prompt["blocks"][-1]["kind"] == "thinking":
                prompt["blocks"][-1]["text"] += text
            else:
                prompt["blocks"].append({"kind": "thinking", "text": text})
        elif is_tool:
            update_meta = meta.get("updateParams") or {}
            if not call_id:
                prompt["unknown"].append(copy.deepcopy(envelope))
                continue
            first = call_id not in prompt["tools"]
            name = _tool_name(meta, update, nested_meta)
            tool = prompt["tools"].setdefault(call_id, {
                "id": call_id,
                "name": name,
                "input": {}, "output": None, "status": "unknown",
            })
            if first:
                prompt["blocks"].append({"kind": "tool", "id": call_id})
            elif tool["name"] == "tool" and name != "tool":
                tool["name"] = name
            if update.get("rawInput") is not None:
                tool["input"] = copy.deepcopy(update["rawInput"])
            if update.get("rawOutput") is not None:
                tool["output"] = copy.deepcopy(update["rawOutput"])
            content_text = _text(update.get("content"))
            if content_text:
                tool["output"] = content_text
            if update_meta.get("status"):
                status = str(update_meta["status"]).lower()
                if (
                    tool["status"] not in _TERMINAL_TOOL_STATUSES
                    or status in _TERMINAL_TOOL_STATUSES
                ):
                    tool["status"] = status
        elif kind == "compaction" or update_type == "Compaction":
            prompt["compaction"] = copy.deepcopy(update)
        else:
            prompt["unknown"].append(copy.deepcopy(envelope))
    return [prompts[key] for key in order]
